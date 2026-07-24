#[cfg(unix)]
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(windows)]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use sky_cua_platform::config::{
    BrowserControlMode as PlatformBrowserControlMode, resolved_browser_control_config,
};
use sky_cua_platform::{
    CLIENT_CLEARED_SESSION_ENV_KEYS_ENV,
    model::{
        DoctorSessionEnvRepair, ServiceRequest, ServiceResponse,
        browser_control_mode_from_capabilities,
    },
};
use sky_cua_platform::{CLIENT_SESSION_ENV_REPAIRS_ENV, GRAPHICAL_SESSION_ENV_KEYS};
#[cfg(unix)]
use sky_cua_platform::{SERVICE_SOCKET_PATH_ENV, service_socket_path};
#[cfg(windows)]
use sky_cua_platform::{SERVICE_TCP_ADDR_ENV, service_tcp_addr};

#[cfg(unix)]
use crate::isolated_desktop::IsolatedDesktopHandle;
use crate::launch_environment::LaunchEnvironment;
#[cfg(unix)]
use sky_cua_platform::config::{Lifecycle, ViewerMode, resolve_isolated_desktop_selection};

/// Cap on a single IPC line (request or response). Must match the daemon's
/// `MAX_IPC_LINE_BYTES` in `sky-cua-service`: screenshots travel base64-inline
/// in responses, so this stays generous rather than truncating real replies.
const MAX_IPC_LINE_BYTES: u64 = 64 * 1024 * 1024;
const SERVICE_READ_TIMEOUT: Duration = Duration::from_secs(60);
const SERVICE_WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const STARTUP_HEALTH_READ_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_HEALTH_WRITE_TIMEOUT: Duration = Duration::from_millis(250);
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(150);
const STARTUP_HEALTH_ATTEMPTS: usize = 160;
#[cfg(unix)]
const STALE_SERVICE_TERMINATION_TIMEOUT: Duration = Duration::from_secs(3);
#[derive(Debug, Clone)]
pub struct ServiceClient {
    endpoint: ServiceEndpoint,
    child: Arc<Mutex<Option<Child>>>,
    cached_stream: Arc<Mutex<Option<EitherStream>>>,
    /// Live handle to the private xpra desktop in isolated mode. `None` on the
    /// non-isolated path. Wrapped in `Arc` because `IsolatedDesktopHandle` owns
    /// a non-`Clone` child and `ServiceClient` is `Clone`; the handle is shared,
    /// not duplicated, so `spawn_service` re-spawns stay sandboxed via the same
    /// handle without re-ensuring xpra.
    #[cfg(unix)]
    isolated: Option<std::sync::Arc<crate::isolated_desktop::IsolatedDesktopHandle>>,
    /// The resolved isolated-desktop lifecycle, recorded alongside the handle so
    /// shutdown can consult it. `Persistent` (the default) leaves the xpra
    /// session running across sessions; `Ephemeral` tears it down when the
    /// client exits. Meaningful only when `isolated` is `Some`.
    #[cfg(unix)]
    isolated_lifecycle: Lifecycle,
}

impl ServiceClient {
    pub fn connect_or_spawn() -> Result<Self> {
        // Isolated mode: bring up the private xpra desktop and redirect all
        // socket resolution at the isolated daemon BEFORE the endpoint and any
        // probe reads it. A config-resolution error must never brick normal
        // startup, so a bad config logs a warning and falls through to the
        // non-isolated path.
        #[cfg(unix)]
        let isolated = match resolve_isolated_desktop_selection() {
            Ok(cfg) if cfg.enabled => Some(cfg),
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "failed to resolve isolated desktop configuration; \
                     continuing without an isolated desktop"
                );
                None
            }
        };

        #[cfg(unix)]
        let isolated = match isolated {
            Some(cfg) => {
                // Fail closed, not open: if isolated mode was requested but the
                // private desktop cannot be established (missing xpra/openbox/
                // xdotool, unreachable display, no XDG_RUNTIME_DIR, dbus failure),
                // surface a clear error rather than silently falling back to the
                // user's live desktop — a silent fallback would have the agent
                // drive the real screen, defeating the non-interference purpose.
                // Milestone 4 refines this into a structured per-tool diagnostic
                // that keeps non-desktop tools (phone, browser) available; until
                // then a clear hard failure is the safe behavior.
                let handle = IsolatedDesktopHandle::ensure(&cfg).with_context(|| {
                    "isolated desktop was requested (via [isolated_desktop] or \
                     SKY_CUA_ISOLATED_DESKTOP) but could not be established; refusing \
                     to fall back to the user's live desktop. Ensure xpra, openbox, and \
                     xdotool are installed and a display is reachable"
                })?;
                // Redirect every socket probe (including the call() re-probe)
                // at the isolated daemon for this client's lifetime. The
                // endpoint and spawn paths both honor SKY_CUA_SERVICE_SOCKET_PATH.
                tracing::debug!(
                    socket = %handle.socket_path().display(),
                    "redirecting the sky-cua-service socket at the isolated daemon"
                );
                // SAFETY: this runs inside `block_on` on a multi-thread runtime,
                // so worker threads exist — but they are parked and never touch
                // the process environment. `connect_or_spawn` does no `tokio::spawn`,
                // and the only readers of SKY_CUA_SERVICE_SOCKET_PATH
                // (service_socket_path_for_launch_environment, ServiceEndpoint::new)
                // run on this startup thread before `serve` spawns any task, so the
                // mutation is effectively single-threaded. It is set once at startup.
                unsafe {
                    std::env::set_var(SERVICE_SOCKET_PATH_ENV, handle.socket_path());
                }
                Some((std::sync::Arc::new(handle), cfg.viewer, cfg.lifecycle))
            }
            None => None,
        };

        // In isolated mode the daemon's health-equality expectations must be the
        // sandbox graphical session, not the client's host (Wayland) session;
        // otherwise the sandboxed daemon (DISPLAY=:N, XDG_SESSION_TYPE=x11, no
        // WAYLAND_DISPLAY) is permanently rejected as "stale". The non-isolated
        // path keeps the probed host launch environment byte-for-byte.
        #[cfg(unix)]
        let launch_environment = match &isolated {
            Some((handle, _, _)) => {
                LaunchEnvironment::for_isolated_daemon(&handle.spawn_env(), &handle.removed_env())
            }
            None => LaunchEnvironment::probe(),
        };
        #[cfg(not(unix))]
        let launch_environment = LaunchEnvironment::probe();
        // Resolve policy before touching an existing singleton. A malformed
        // per-process override must fail this client, never classify a healthy
        // shared daemon as stale and replace it.
        resolved_browser_control_config().map_err(anyhow::Error::msg)?;
        let client = Self::new(&launch_environment)?;
        #[cfg(unix)]
        let client = match &isolated {
            Some((handle, _, lifecycle)) => {
                client.with_isolated(std::sync::Arc::clone(handle), *lifecycle)
            }
            None => client,
        };
        // Startup probes must fail fast; a stale or half-ready daemon should not
        // consume the entire MCP server startup window before we even spawn a
        // fresh service instance.
        match client.startup_health(&launch_environment) {
            Ok(_) => {
                #[cfg(unix)]
                if let Some((handle, viewer, _)) = &isolated {
                    launch_isolated_viewer(handle, *viewer);
                }
                return Ok(client);
            }
            Err(error) => {
                if error
                    .downcast_ref::<SharedBrowserDaemonConflict>()
                    .is_some()
                {
                    return Err(error);
                }
                if is_stale_startup_health_error(&error) {
                    client.displace_stale_service(&error)?;
                }
            }
        }

        client.spawn_service(&launch_environment)?;
        client.wait_for_startup_health(&launch_environment)?;
        #[cfg(unix)]
        if let Some((handle, viewer, _)) = &isolated {
            launch_isolated_viewer(handle, *viewer);
        }
        Ok(client)
    }

    fn startup_health(&self, launch_environment: &LaunchEnvironment) -> Result<ServiceResponse> {
        let (response, owner_pid) = self.call_with_timeouts_with_peer(
            &ServiceRequest::Health,
            STARTUP_HEALTH_READ_TIMEOUT,
            STARTUP_HEALTH_WRITE_TIMEOUT,
        )?;
        let requested_mode = resolved_browser_control_config()
            .map_err(anyhow::Error::msg)?
            .mode
            .unwrap_or(PlatformBrowserControlMode::Legacy);
        let reported_mode = reported_browser_control_mode(&response);
        ensure_browser_control_mode_compatible(requested_mode, reported_mode).map_err(
            |detail| {
                if reported_mode.is_some_and(persistent_browser_control_mode) && !self.is_isolated()
                {
                    anyhow!(SharedBrowserDaemonConflict { detail })
                } else {
                    anyhow!(StaleStartupService { detail, owner_pid })
                }
            },
        )?;
        if let Err(error) = launch_environment.ensure_startup_health(&response, true) {
            if reported_mode.is_some_and(persistent_browser_control_mode) && !self.is_isolated() {
                return Err(anyhow!(SharedBrowserDaemonConflict {
                    detail: error.to_string(),
                }));
            }
            return Err(anyhow!(StaleStartupService {
                detail: error.to_string(),
                owner_pid,
            }));
        }
        Ok(response)
    }

    pub fn clear_portal_tokens(&self) -> Result<ServiceResponse> {
        self.call(&ServiceRequest::ResetPortalTokens)
    }

    /// Whether this client drives the private isolated xpra desktop.
    ///
    /// True only when isolated mode was requested and the desktop was
    /// successfully established (the client holds a live handle). Tools that
    /// must never touch the user's live session — e.g. launching applications —
    /// gate on this.
    #[cfg(unix)]
    #[must_use]
    pub fn is_isolated(&self) -> bool {
        self.isolated.is_some()
    }

    #[cfg(not(unix))]
    #[must_use]
    pub fn is_isolated(&self) -> bool {
        false
    }

    /// Tear down the private xpra desktop when the resolved lifecycle is
    /// [`Lifecycle::Ephemeral`]. A no-op on the non-isolated path and on
    /// [`Lifecycle::Persistent`] (where the session is reused across agent
    /// sessions and stopped only on explicit `isolated-desktop stop`).
    ///
    /// Best-effort: a teardown failure logs a warning and returns `Ok(())` so the
    /// MCP shutdown seam never changes its exit code on a failed reap. Stopping
    /// the xpra session, reaping a client-owned sandbox bus, and removing a stale
    /// `/tmp/.X<N>-lock` are all delegated to [`IsolatedDesktopHandle::stop`],
    /// which filters strictly by the known display number so it can never touch
    /// the user's real session.
    ///
    /// [`IsolatedDesktopHandle::stop`]: crate::isolated_desktop::IsolatedDesktopHandle::stop
    #[cfg(unix)]
    pub fn shutdown_isolated_if_ephemeral(&self) {
        let Some(handle) = self.isolated.as_ref() else {
            return;
        };
        if self.isolated_lifecycle != Lifecycle::Ephemeral {
            return;
        }
        if let Err(error) = handle.stop() {
            tracing::warn!(
                %error,
                xpra_display = %handle.display(),
                "failed to tear down the ephemeral isolated desktop on shutdown \
                 (best-effort)"
            );
        } else {
            tracing::info!(
                xpra_display = %handle.display(),
                "tore down the ephemeral isolated desktop on client shutdown"
            );
        }
    }

    /// On non-Unix targets there is no isolated desktop, so shutdown teardown is
    /// a no-op. Present so the shutdown seam in `main` can call it unconditionally.
    #[cfg(not(unix))]
    pub fn shutdown_isolated_if_ephemeral(&self) {}

    pub fn call(&self, request: &ServiceRequest) -> Result<ServiceResponse> {
        match self.call_with_timeouts(request, SERVICE_READ_TIMEOUT, SERVICE_WRITE_TIMEOUT) {
            Ok(response) => Ok(response),
            Err(first_error) => {
                // A respawn-then-retry re-sends the identical request. That's
                // only safe when the request is idempotent, or the first
                // failure provably preceded daemon receipt (so nothing was
                // ever executed to double up on).
                if !should_retry_error(request, &first_error) {
                    return Err(ambiguous_failure_error(first_error));
                }
                self.reap_exited_child()?;
                let launch_environment = self.recovery_launch_environment();
                self.spawn_service(&launch_environment)?;
                self.wait_for_startup_health(&launch_environment)
                    .with_context(|| format!("after service call failed: {first_error}"))?;
                self.call_with_timeouts(request, SERVICE_READ_TIMEOUT, SERVICE_WRITE_TIMEOUT)
                    .with_context(|| format!("after service call failed: {first_error}"))
            }
        }
    }

    /// The launch environment to use when `call()` recovers by re-spawning the
    /// daemon. In isolated mode the daemon's health expectations are the sandbox
    /// graphical session (so a re-spawn is not rejected as "stale"); otherwise
    /// the host environment is re-probed exactly as before.
    fn recovery_launch_environment(&self) -> LaunchEnvironment {
        #[cfg(unix)]
        if let Some(handle) = self.isolated.as_ref() {
            return LaunchEnvironment::for_isolated_daemon(
                &handle.spawn_env(),
                &handle.removed_env(),
            );
        }
        LaunchEnvironment::probe()
    }

    fn take_cached_stream(&self) -> Option<EitherStream> {
        self.cached_stream
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    fn store_cached_stream(&self, stream: EitherStream) {
        let _ = self
            .cached_stream
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(stream);
    }

    fn clear_cached_stream(&self) {
        let _ = self
            .cached_stream
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }

    fn call_with_timeouts(
        &self,
        request: &ServiceRequest,
        read_timeout: Duration,
        write_timeout: Duration,
    ) -> Result<ServiceResponse> {
        self.call_with_timeouts_with_peer(request, read_timeout, write_timeout)
            .map(|(response, _)| response)
    }

    fn call_with_timeouts_with_peer(
        &self,
        request: &ServiceRequest,
        read_timeout: Duration,
        write_timeout: Duration,
    ) -> Result<(ServiceResponse, Option<u32>)> {
        // Attempt 1: try cached stream if available.
        if let Some(stream) = self.take_cached_stream() {
            match self.perform_call_on_stream(stream, request, read_timeout, write_timeout) {
                Ok((response, stream, owner_pid)) => {
                    self.store_cached_stream(stream);
                    return Ok((response, owner_pid));
                }
                Err(failure) if is_stale_stream_failure(&failure) => {
                    self.clear_cached_stream();
                    // Cache invalidation (dropping a dead cached stream) is
                    // orthogonal to whether re-sending the request is safe.
                    // Only fall through to attempt 2 when this exact request
                    // may be safely repeated.
                    if !should_retry(request, &failure) {
                        return Err(ambiguous_failure_error(failure.into()));
                    }
                }
                Err(failure) => {
                    // A non-stale error from the cached stream is an
                    // application-level failure (e.g. a daemon error
                    // response), not a connection problem — return it
                    // directly rather than retrying with a fresh connect.
                    self.clear_cached_stream();
                    return Err(failure.into());
                }
            }
        }

        // Attempt 2: fresh connection.
        let stream = self.endpoint.connect().map_err(CallFailure::BeforeWrite)?;
        let (response, stream, owner_pid) =
            self.perform_call_on_stream(stream, request, read_timeout, write_timeout)?;
        self.store_cached_stream(stream);
        Ok((response, owner_pid))
    }

    fn perform_call_on_stream(
        &self,
        mut stream: EitherStream,
        request: &ServiceRequest,
        read_timeout: Duration,
        write_timeout: Duration,
    ) -> Result<(ServiceResponse, EitherStream, Option<u32>), CallFailure> {
        let owner_pid = stream.peer_pid();
        stream
            .set_read_timeout(Some(read_timeout))
            .context("failed to set a read timeout on the sky-cua-service socket")
            .map_err(CallFailure::BeforeWrite)?;
        stream
            .set_write_timeout(Some(write_timeout))
            .context("failed to set a write timeout on the sky-cua-service socket")
            .map_err(CallFailure::BeforeWrite)?;
        let payload = serde_json::to_vec(request).map_err(|error| {
            CallFailure::BeforeWrite(
                anyhow::Error::new(error).context("failed to serialize request"),
            )
        })?;
        stream
            .write_all(&payload)
            .map_err(|error| CallFailure::AfterWrite(error.into()))?;
        stream
            .write_all(b"\n")
            .map_err(|error| CallFailure::AfterWrite(error.into()))?;
        stream
            .flush()
            .map_err(|error| CallFailure::AfterWrite(error.into()))?;

        // Everything past this point runs after the request has been written
        // (and flushed) to the daemon: the daemon may already have received
        // and executed it, so any failure from here on is classified
        // `AfterWrite` and is not safe to blind-retry for a non-idempotent
        // request (see `should_retry`).
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let read = {
            let mut limited = (&mut reader).take(MAX_IPC_LINE_BYTES);
            limited
                .read_line(&mut line)
                .map_err(|error| CallFailure::AfterWrite(error.into()))?
        };
        if read == 0 || line.trim().is_empty() {
            return Err(CallFailure::AfterWrite(anyhow!(
                "sky-cua-service connection closed before response"
            )));
        }
        if read as u64 == MAX_IPC_LINE_BYTES && !line.ends_with('\n') {
            return Err(CallFailure::AfterWrite(anyhow!(
                "sky-cua-service response exceeded {MAX_IPC_LINE_BYTES} bytes without a newline"
            )));
        }
        let response: ServiceResponse = serde_json::from_str(line.trim_end())
            .map_err(|error| CallFailure::AfterWrite(error.into()))?;
        let stream = reader.into_inner();
        Ok((response, stream, owner_pid))
    }

    fn new(launch_environment: &LaunchEnvironment) -> Result<Self> {
        Ok(Self {
            endpoint: ServiceEndpoint::new(launch_environment)?,
            child: Arc::new(Mutex::new(None)),
            cached_stream: Arc::new(Mutex::new(None)),
            #[cfg(unix)]
            isolated: None,
            #[cfg(unix)]
            isolated_lifecycle: Lifecycle::Persistent,
        })
    }

    /// Attach a live isolated-desktop handle and its resolved lifecycle to this
    /// client, consuming and returning `self` so the caller can shadow without an
    /// `unused_mut` warning on the non-isolated path. Used only when isolated
    /// mode is enabled.
    #[cfg(unix)]
    fn with_isolated(
        mut self,
        handle: std::sync::Arc<crate::isolated_desktop::IsolatedDesktopHandle>,
        lifecycle: Lifecycle,
    ) -> Self {
        self.isolated = Some(handle);
        self.isolated_lifecycle = lifecycle;
        self
    }

    fn spawn_service(&self, launch_environment: &LaunchEnvironment) -> Result<()> {
        let mut child_guard = self
            .child
            .lock()
            .map_err(|_| anyhow!("sky-cua-service child state mutex was poisoned"))?;
        if let Some(child) = child_guard.as_mut()
            && child.try_wait()?.is_none()
        {
            return Ok(());
        }

        // Drop any cached stream from a previous service process before
        // spawning a new one so we don't write to a dead socket.
        self.clear_cached_stream();

        let service_path = service_path();
        let log_stem = self.endpoint.daemon_log_stem();
        let mut command = Command::new(&service_path);
        command
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(crate::daemon_log::daemon_stderr_destination(&log_stem));

        // Tell the daemon where its per-endpoint log lives so it can size-cap
        // and rotate its own tracing output at runtime, instead of appending
        // without bound until its next launch. The inherited stderr above
        // captures pre-tracing-init output; the daemon then keeps fd 2 pointed
        // at the live log across rotations (unix), so panics follow it too; the
        // daemon routes steady-state tracing to this path via its self-rotating
        // writer. Absent this var, the daemon falls back to plain stderr.
        if let Some(log_path) = crate::daemon_log::daemon_log_path(&log_stem) {
            command.env(sky_cua_platform::DAEMON_LOG_PATH_ENV, log_path);
        }

        configure_launch_environment_env(&mut command, launch_environment);

        self.endpoint.configure_service_command(&mut command);

        // Apply the isolated-desktop sandbox env LAST so it wins over both the
        // repaired desktop vars and the socket env: DISPLAY=:N, XDG_SESSION_TYPE=x11,
        // QT_QPA_PLATFORM=xcb, GDK_BACKEND=x11, DBUS_SESSION_BUS_ADDRESS, the
        // isolated socket, and WAYLAND_DISPLAY removed. call()->spawn_service
        // reuses self.isolated, so re-spawns stay sandboxed without re-ensuring xpra.
        #[cfg(unix)]
        if let Some(handle) = self.isolated.as_ref() {
            for (key, value) in handle.spawn_env() {
                command.env(key, value);
            }
            for key in handle.removed_env() {
                command.env_remove(key);
            }
        }

        let child = command
            .spawn()
            .with_context(|| format!("failed to spawn service at {}", service_path.display()))?;
        *child_guard = Some(child);
        Ok(())
    }

    fn wait_for_startup_health(&self, launch_environment: &LaunchEnvironment) -> Result<()> {
        let mut last_error: Option<anyhow::Error> = None;
        for _ in 0..STARTUP_HEALTH_ATTEMPTS {
            match self.startup_health(launch_environment) {
                Ok(_) => return Ok(()),
                Err(error) => last_error = Some(error),
            }
            self.reap_exited_child()?;
            thread::sleep(STARTUP_POLL_INTERVAL);
        }

        // Surface the concrete per-poll failure; "did not become healthy"
        // alone hides whether the daemon was unreachable, slow, or rejected
        // by an environment staleness check.
        let detail = last_error
            .map(|error| format!(": last health error: {error:#}"))
            .unwrap_or_default();
        Err(anyhow!(
            "sky-cua-service did not become healthy on {}{detail}",
            self.endpoint
        ))
    }

    fn reap_exited_child(&self) -> Result<()> {
        let mut child_guard = self
            .child
            .lock()
            .map_err(|_| anyhow!("sky-cua-service child state mutex was poisoned"))?;
        if let Some(child) = child_guard.as_mut()
            && child.try_wait()?.is_some()
        {
            *child_guard = None;
        }
        Ok(())
    }

    #[cfg(unix)]
    fn displace_stale_service(&self, reason: &anyhow::Error) -> Result<()> {
        let fallback_owner_pid = reason
            .downcast_ref::<StaleStartupService>()
            .and_then(|error| error.owner_pid);
        match self.endpoint.terminate_stale_owners(fallback_owner_pid) {
            Ok(killed) if !killed.is_empty() => {
                self.clear_cached_stream();
                self.endpoint.wait_for_singleton_release()?;
                Ok(())
            }
            Ok(_) => Err(anyhow!(
                "existing sky-cua-service is stale ({reason}) but its singleton owner could not be identified"
            )),
            Err(error) => Err(error).context("failed to terminate stale sky-cua-service"),
        }
    }

    #[cfg(windows)]
    fn displace_stale_service(&self, reason: &anyhow::Error) -> Result<()> {
        Err(anyhow!(
            "existing sky-cua-service is stale ({reason}) and automatic daemon replacement is not implemented on Windows"
        ))
    }
}

fn reported_browser_control_mode(response: &ServiceResponse) -> Option<PlatformBrowserControlMode> {
    let ServiceResponse::Health { capabilities, .. } = response else {
        return None;
    };
    browser_control_mode_from_capabilities(capabilities)
}

fn persistent_browser_control_mode(mode: PlatformBrowserControlMode) -> bool {
    matches!(
        mode,
        PlatformBrowserControlMode::Hybrid | PlatformBrowserControlMode::Strict
    )
}

fn ensure_browser_control_mode_compatible(
    requested: PlatformBrowserControlMode,
    reported: Option<PlatformBrowserControlMode>,
) -> std::result::Result<(), String> {
    let compatible = matches!(
        (requested, reported),
        (
            PlatformBrowserControlMode::Legacy,
            None | Some(PlatformBrowserControlMode::Legacy)
        ) | (
            PlatformBrowserControlMode::Hybrid,
            Some(PlatformBrowserControlMode::Hybrid | PlatformBrowserControlMode::Strict),
        ) | (
            PlatformBrowserControlMode::Strict,
            Some(PlatformBrowserControlMode::Strict)
        )
    );
    if compatible {
        return Ok(());
    }
    Err(format!(
        "browser-control mode mismatch: client requested {requested:?}, daemon reported {}",
        reported.map_or("legacy/unknown".to_owned(), |mode| format!("{mode:?}"))
    ))
}

#[derive(Debug, Clone)]
enum ServiceEndpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    #[cfg(windows)]
    Tcp(String),
}

impl ServiceEndpoint {
    fn new(launch_environment: &LaunchEnvironment) -> Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self::Unix(service_socket_path_for_launch_environment(
                launch_environment,
            )))
        }
        #[cfg(windows)]
        {
            let _ = launch_environment;
            Ok(Self::Tcp(resolve_service_tcp_addr()?))
        }
    }

    fn connect(&self) -> Result<EitherStream> {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => UnixStream::connect(path)
                .with_context(|| {
                    format!(
                        "failed to connect to sky-cua-service socket {}",
                        path.display()
                    )
                })
                .map(EitherStream::Unix),
            #[cfg(windows)]
            Self::Tcp(addr) => TcpStream::connect(addr)
                .with_context(|| {
                    format!("failed to connect to sky-cua-service TCP endpoint {addr}")
                })
                .map(EitherStream::Tcp),
        }
    }

    /// One log per daemon endpoint: the default daemon and an isolated-desktop
    /// daemon (distinct socket) must not interleave in a single file.
    fn daemon_log_stem(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => {
                let socket_stem = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "service".to_string());
                format!("daemon-{socket_stem}")
            }
            #[cfg(windows)]
            Self::Tcp(_) => "daemon-service".to_string(),
        }
    }

    fn configure_service_command(&self, command: &mut Command) {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => {
                command.env(SERVICE_SOCKET_PATH_ENV, path);
            }
            #[cfg(windows)]
            Self::Tcp(addr) => {
                command.env(SERVICE_TCP_ADDR_ENV, addr);
            }
        }
    }

    #[cfg(unix)]
    fn terminate_stale_owners(&self, fallback_owner_pid: Option<u32>) -> Result<Vec<u32>> {
        let Self::Unix(path) = self;
        let candidates = owner_pids_for_termination(path, fallback_owner_pid)?;
        let mut killed = Vec::new();
        for pid in candidates {
            // SAFETY: `kill` with SIGTERM has no Rust-side memory safety
            // preconditions. Candidate PIDs are either the connected Unix
            // socket peer or a lock-file owner that still looks like our
            // service binary.
            let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            if result == 0 {
                killed.push(pid);
            } else {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ESRCH) {
                    killed.push(pid);
                } else {
                    return Err(error.into());
                }
            }
        }
        Ok(killed)
    }

    #[cfg(unix)]
    fn wait_for_singleton_release(&self) -> Result<()> {
        let Self::Unix(path) = self;
        let deadline = Instant::now() + STALE_SERVICE_TERMINATION_TIMEOUT;
        while Instant::now() < deadline {
            if singleton_lock_is_available(path)? {
                return Ok(());
            }
            thread::sleep(STARTUP_POLL_INTERVAL);
        }
        Err(anyhow!(
            "timed out waiting for stale sky-cua-service singleton lock to release"
        ))
    }
}

#[cfg(unix)]
fn service_socket_path_for_launch_environment(launch_environment: &LaunchEnvironment) -> PathBuf {
    if std::env::var_os(SERVICE_SOCKET_PATH_ENV).is_some() {
        return service_socket_path();
    }

    if std::env::var_os("XDG_RUNTIME_DIR").is_none_or(|value| value.is_empty())
        && let Some(runtime_dir) = launch_environment.repaired_desktop_var("XDG_RUNTIME_DIR")
    {
        return PathBuf::from(runtime_dir)
            .join("sky-cua")
            .join("service.sock");
    }

    service_socket_path()
}

#[cfg(unix)]
fn owner_pids_for_termination(
    socket_path: &std::path::Path,
    peer_pid: Option<u32>,
) -> Result<Vec<u32>> {
    let mut candidates = BTreeSet::new();
    if let Some(pid) = peer_pid {
        candidates.insert(pid);
    }
    if let Some(pid) = crate::daemon_singleton::read_owner_pid(socket_path)?
        && crate::daemon_singleton::pid_is_sky_cua_service(pid)
    {
        candidates.insert(pid);
    }
    Ok(candidates.into_iter().collect())
}

#[cfg(unix)]
fn singleton_lock_is_available(socket_path: &std::path::Path) -> Result<bool> {
    use std::os::unix::io::AsRawFd;

    let lock_path = crate::daemon_singleton::socket_lock_path(socket_path);
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    let result = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        let _ = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_UN) };
        Ok(true)
    } else {
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN => Ok(false),
            Some(libc::EINTR) => Ok(false),
            _ => Err(error.into()),
        }
    }
}

impl std::fmt::Display for ServiceEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => write!(formatter, "{}", path.display()),
            #[cfg(windows)]
            Self::Tcp(addr) => write!(formatter, "{addr}"),
        }
    }
}

/// Launch the configured read-only viewer once the isolated daemon is healthy.
/// The viewer is non-essential: a failure to start it logs a warning and never
/// fails the session. [`ViewerMode::None`] is a no-op inside `launch_viewer`.
#[cfg(unix)]
fn launch_isolated_viewer(handle: &IsolatedDesktopHandle, viewer: ViewerMode) {
    if let Err(error) = handle.launch_viewer(viewer) {
        tracing::warn!(
            %error,
            xpra_display = %handle.display(),
            "failed to launch the isolated desktop viewer (non-essential)"
        );
    }
}

fn configure_launch_environment_env(command: &mut Command, launch_environment: &LaunchEnvironment) {
    // Forward reconstructed desktop session env vars so the service can
    // initialize its platform backends even when the MCP host did not pass
    // them through.
    if launch_environment.detached_graphical_env() {
        let cleared_keys = GRAPHICAL_SESSION_ENV_KEYS
            .iter()
            .copied()
            .chain(std::iter::once("XAUTHORITY"))
            .collect::<Vec<_>>();
        for key in &cleared_keys {
            command.env_remove(key);
        }
        if let Ok(serialized) = serde_json::to_string(&cleared_keys) {
            command.env(CLIENT_CLEARED_SESSION_ENV_KEYS_ENV, serialized);
        }
    }
    for (key, value) in launch_environment.repaired_desktop_vars() {
        command.env(key, value);
    }
    if !launch_environment.repaired_desktop_vars().is_empty() {
        let repairs = launch_environment
            .repaired_desktop_vars()
            .iter()
            .map(|(key, value)| DoctorSessionEnvRepair {
                key: key.clone(),
                source: "client-launch".to_string(),
                value: Some(value.clone()),
            })
            .collect::<Vec<_>>();
        if let Ok(serialized) = serde_json::to_string(&repairs) {
            command.env(CLIENT_SESSION_ENV_REPAIRS_ENV, serialized);
        }
    }
}

#[derive(Debug)]
enum EitherStream {
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    Tcp(TcpStream),
}

impl std::io::Read for EitherStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buf),
            #[cfg(windows)]
            Self::Tcp(stream) => stream.read(buf),
        }
    }
}

impl std::io::Write for EitherStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buf),
            #[cfg(windows)]
            Self::Tcp(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
            #[cfg(windows)]
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

impl EitherStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.set_read_timeout(timeout),
            #[cfg(windows)]
            Self::Tcp(stream) => stream.set_read_timeout(timeout),
        }
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Unix(stream) => stream.set_write_timeout(timeout),
            #[cfg(windows)]
            Self::Tcp(stream) => stream.set_write_timeout(timeout),
        }
    }

    fn peer_pid(&self) -> Option<u32> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Unix(stream) => unix_stream_peer_pid(stream),
            #[cfg(all(unix, not(target_os = "linux")))]
            Self::Unix(_stream) => None,
            #[cfg(windows)]
            Self::Tcp(_stream) => None,
        }
    }
}

#[cfg(target_os = "linux")]
fn unix_stream_peer_pid(stream: &UnixStream) -> Option<u32> {
    use std::mem::MaybeUninit;
    use std::os::unix::io::AsRawFd;

    let mut credentials = MaybeUninit::<libc::ucred>::uninit();
    let mut credentials_len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            credentials.as_mut_ptr().cast(),
            &mut credentials_len,
        )
    };
    if result != 0 || credentials_len < std::mem::size_of::<libc::ucred>() as libc::socklen_t {
        return None;
    }
    let credentials = unsafe { credentials.assume_init() };
    (credentials.pid > 1).then_some(credentials.pid as u32)
}

/// Which side of the request write a `perform_call_on_stream` failure occurred
/// on. This is orthogonal to [`is_stale_stream_error`] (which decides whether
/// a *cached* connection should be dropped): it decides whether a *request*
/// can be safely re-sent.
///
/// `BeforeWrite` failures (connect, timeout setup, serialize) provably precede
/// daemon receipt, so resending is always safe. Once the first write begins,
/// every failure is `AfterWrite`: `write_all` may have written a prefix, and a
/// failed newline write can still leave a complete JSON request that the
/// daemon parses at EOF. Read/parse failures are likewise after dispatch.
/// Resending after any of those failures is safe only for idempotent requests.
#[derive(Debug)]
enum CallFailure {
    BeforeWrite(anyhow::Error),
    AfterWrite(anyhow::Error),
}

impl std::fmt::Display for CallFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BeforeWrite(error) | Self::AfterWrite(error) => write!(formatter, "{error}"),
        }
    }
}

// `CallFailure` wraps an already-fully-formatted `anyhow::Error`, so it has no
// separate `source()`; the wrapped message is everything callers need. This
// (plus `Send + Sync + 'static`, satisfied because `anyhow::Error` is) is
// enough for anyhow's blanket `From<E: std::error::Error + Send + Sync +
// 'static>` impl, which is what lets `?` convert a `CallFailure` into an
// `anyhow::Error` while remaining downcastable via `.chain()`.
impl std::error::Error for CallFailure {}

/// Whether a request that failed with `failure` should be retried (either on
/// a fresh connection after a stale cached stream, or via a full respawn).
///
/// Idempotent requests are always retried: repeating them cannot corrupt
/// observable state. Non-idempotent requests are retried only when `failure`
/// provably precedes daemon receipt (`BeforeWrite`) — an `AfterWrite` failure
/// means the daemon may already have executed the action, so blind-retrying
/// would risk double-executing a click, keystroke, launch, or other mutating
/// action.
fn should_retry(request: &ServiceRequest, failure: &CallFailure) -> bool {
    request.is_idempotent() || matches!(failure, CallFailure::BeforeWrite(_))
}

/// Same gate as [`should_retry`], applied to an already-converted
/// `anyhow::Error` (the public `call_with_timeouts` boundary type). Recovers
/// the original [`CallFailure`] from the error chain; when none is found (the
/// error did not originate from `perform_call_on_stream` or the fresh
/// connect), falls back to idempotency alone — the conservative, safe
/// default for a failure of unknown stage.
fn should_retry_error(request: &ServiceRequest, error: &anyhow::Error) -> bool {
    match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<CallFailure>())
    {
        Some(failure) => should_retry(request, failure),
        None => request.is_idempotent(),
    }
}

/// Build the error returned for a non-retryable ambiguous failure on a
/// non-idempotent request. The wording is agent-facing: it tells the model to
/// re-observe current state rather than blindly repeating the action.
/// Wraps (rather than replaces) `error` via `.context()` so the original
/// `CallFailure` survives in the chain for any further inspection.
fn ambiguous_failure_error(error: anyhow::Error) -> anyhow::Error {
    let message = format!(
        "action may or may not have executed: response was lost after the request was sent \
         ({error}); not retrying a non-idempotent action — observe the current state before \
         repeating it"
    );
    error.context(message)
}

fn is_stale_stream_error(error: &anyhow::Error) -> bool {
    let error_string = error.to_string().to_lowercase();
    error_string.contains("broken pipe")
        || error_string.contains("connection refused")
        || error_string.contains("connection reset")
        || error_string.contains("connection closed before response")
        || error_string.contains("not connected")
        || error_string.contains("unexpected eof")
}

/// [`is_stale_stream_error`] applied to a [`CallFailure`], for the cached
/// -stream retry site (which sees the failure before it is converted to
/// `anyhow::Error`).
fn is_stale_stream_failure(failure: &CallFailure) -> bool {
    match failure {
        CallFailure::BeforeWrite(error) | CallFailure::AfterWrite(error) => {
            is_stale_stream_error(error)
        }
    }
}

#[derive(Debug)]
struct StaleStartupService {
    detail: String,
    owner_pid: Option<u32>,
}

impl std::fmt::Display for StaleStartupService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "existing sky-cua-service is stale: {}",
            self.detail
        )
    }
}

impl std::error::Error for StaleStartupService {}

#[derive(Debug)]
struct SharedBrowserDaemonConflict {
    detail: String,
}

impl std::fmt::Display for SharedBrowserDaemonConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "existing shared sky-cua browser daemon is incompatible and was left running: {}",
            self.detail
        )
    }
}

impl std::error::Error for SharedBrowserDaemonConflict {}

fn is_stale_startup_health_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<StaleStartupService>().is_some()
}

fn service_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SKY_CUA_SERVICE_PATH") {
        return PathBuf::from(path);
    }
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(sibling) = exe_path
            .parent()
            .map(|parent| parent.join(service_binary_name()))
        && sibling.is_file()
    {
        return sibling;
    }
    let repo_root = std::env::var_os("SKY_CUA_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    repo_root.join("bin").join(service_binary_name())
}

fn service_binary_name() -> &'static str {
    if cfg!(windows) {
        "sky-cua-service.exe"
    } else {
        "sky-cua-service"
    }
}

#[cfg(windows)]
fn resolve_service_tcp_addr() -> Result<String> {
    use std::net::TcpListener;

    if std::env::var_os(SERVICE_TCP_ADDR_ENV).is_some_and(|value| !value.is_empty()) {
        return Ok(service_tcp_addr());
    }

    let configured = service_tcp_addr();
    let bind_addr = configured
        .rsplit_once(':')
        .map(|(host, _)| format!("{host}:0"))
        .unwrap_or_else(|| "127.0.0.1:0".to_string());
    let listener = TcpListener::bind(&bind_addr)
        .with_context(|| format!("failed to reserve sky-cua-service TCP endpoint {bind_addr}"))?;
    let addr = listener
        .local_addr()
        .context("failed to read reserved sky-cua-service TCP endpoint")?
        .to_string();
    drop(listener);
    Ok(addr)
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    use sky_cua_platform::{
        CLIENT_CLEARED_SESSION_ENV_KEYS_ENV, CLIENT_SESSION_ENV_REPAIRS_ENV,
        SERVICE_SOCKET_PATH_ENV,
    };

    use super::*;

    #[test]
    fn daemon_log_stem_is_derived_from_the_socket_file_stem() {
        let default_endpoint =
            ServiceEndpoint::Unix(PathBuf::from("/run/user/1000/sky-cua/service.sock"));
        assert_eq!(default_endpoint.daemon_log_stem(), "daemon-service");

        let isolated_endpoint = ServiceEndpoint::Unix(PathBuf::from(
            "/run/user/1000/sky-cua/service-isolated-100.sock",
        ));
        assert_eq!(
            isolated_endpoint.daemon_log_stem(),
            "daemon-service-isolated-100"
        );
    }

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn unix_service_command_uses_client_socket_endpoint() {
        let socket_path = PathBuf::from("/tmp/sky-cua-test/service.sock");
        let endpoint = ServiceEndpoint::Unix(socket_path.clone());
        let mut command = Command::new("sky-cua-service");

        endpoint.configure_service_command(&mut command);

        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == SERVICE_SOCKET_PATH_ENV)
                .and_then(|(_, value)| value),
            Some(socket_path.as_os_str())
        );
    }

    #[test]
    fn detached_launch_env_removes_stale_graphical_keys_before_repairs() {
        let launch_environment =
            LaunchEnvironment::from_repaired_desktop_vars_and_detached_for_tests(
                vec![
                    ("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string()),
                    ("DISPLAY".to_string(), ":0".to_string()),
                    (
                        "XAUTHORITY".to_string(),
                        "/run/user/1000/xpra/Xauthority".to_string(),
                    ),
                ],
                true,
            );
        let mut command = Command::new("sky-cua-service");

        configure_launch_environment_env(&mut command, &launch_environment);

        assert_eq!(command_env_value(&command, "WAYLAND_DISPLAY"), Some(None));
        assert_eq!(
            command_env_value(&command, "XAUTHORITY"),
            Some(Some(OsStr::new("/run/user/1000/xpra/Xauthority")))
        );
        assert_eq!(
            command_env_value(&command, "XDG_RUNTIME_DIR"),
            Some(Some(OsStr::new("/run/user/1000")))
        );
        assert_eq!(
            command_env_value(&command, "DISPLAY"),
            Some(Some(OsStr::new(":0")))
        );

        let raw_repairs = command_env_value(&command, CLIENT_SESSION_ENV_REPAIRS_ENV)
            .and_then(|value| value)
            .and_then(OsStr::to_str)
            .expect("client launch repairs should be serialized");
        let repairs = serde_json::from_str::<Vec<DoctorSessionEnvRepair>>(raw_repairs)
            .expect("client launch repairs should be valid JSON");
        assert_eq!(repairs.len(), 3);
        assert!(
            repairs
                .iter()
                .all(|repair| repair.source == "client-launch")
        );
        assert!(repairs.iter().any(|repair| {
            repair.key == "XDG_RUNTIME_DIR" && repair.value.as_deref() == Some("/run/user/1000")
        }));

        let raw_cleared = command_env_value(&command, CLIENT_CLEARED_SESSION_ENV_KEYS_ENV)
            .and_then(|value| value)
            .and_then(OsStr::to_str)
            .expect("client cleared keys should be serialized");
        let cleared =
            serde_json::from_str::<Vec<String>>(raw_cleared).expect("cleared keys should be JSON");
        assert!(cleared.iter().any(|key| key == "DISPLAY"));
        assert!(cleared.iter().any(|key| key == "WAYLAND_DISPLAY"));
        assert!(cleared.iter().any(|key| key == "XAUTHORITY"));
    }

    #[test]
    fn detached_launch_clears_inherited_xauthority_when_selected_display_has_none() {
        let launch_environment =
            LaunchEnvironment::from_repaired_desktop_vars_and_detached_for_tests(
                vec![("DISPLAY".to_string(), ":100".to_string())],
                true,
            );
        let mut command = Command::new("sky-cua-service");

        configure_launch_environment_env(&mut command, &launch_environment);

        assert_eq!(command_env_value(&command, "XAUTHORITY"), Some(None));
        assert_eq!(
            command_env_value(&command, "DISPLAY"),
            Some(Some(OsStr::new(":100")))
        );
    }

    #[test]
    fn unix_service_endpoint_uses_repaired_runtime_dir_before_cache_fallback() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_socket_path = std::env::var_os(SERVICE_SOCKET_PATH_ENV);
        let old_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        unsafe {
            std::env::remove_var(SERVICE_SOCKET_PATH_ENV);
            std::env::remove_var("XDG_RUNTIME_DIR");
        }
        let launch_environment = LaunchEnvironment::from_repaired_desktop_vars_for_tests(vec![(
            "XDG_RUNTIME_DIR".to_string(),
            "/run/user/1000".to_string(),
        )]);

        let endpoint = ServiceEndpoint::new(&launch_environment).expect("endpoint should resolve");

        restore_env(SERVICE_SOCKET_PATH_ENV, old_socket_path);
        restore_env("XDG_RUNTIME_DIR", old_runtime_dir);
        match endpoint {
            ServiceEndpoint::Unix(path) => {
                assert_eq!(path, PathBuf::from("/run/user/1000/sky-cua/service.sock"));
            }
        }
    }

    #[test]
    fn unix_service_endpoint_treats_empty_runtime_dir_as_missing() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_socket_path = std::env::var_os(SERVICE_SOCKET_PATH_ENV);
        let old_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        unsafe {
            std::env::remove_var(SERVICE_SOCKET_PATH_ENV);
            std::env::set_var("XDG_RUNTIME_DIR", "");
        }
        let launch_environment = LaunchEnvironment::from_repaired_desktop_vars_for_tests(vec![(
            "XDG_RUNTIME_DIR".to_string(),
            "/run/user/1000".to_string(),
        )]);

        let endpoint = ServiceEndpoint::new(&launch_environment).expect("endpoint should resolve");

        restore_env(SERVICE_SOCKET_PATH_ENV, old_socket_path);
        restore_env("XDG_RUNTIME_DIR", old_runtime_dir);
        match endpoint {
            ServiceEndpoint::Unix(path) => {
                assert_eq!(path, PathBuf::from("/run/user/1000/sky-cua/service.sock"));
            }
        }
    }

    #[test]
    fn unix_service_endpoint_preserves_explicit_socket_override() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let old_socket_path = std::env::var_os(SERVICE_SOCKET_PATH_ENV);
        let old_runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
        unsafe {
            std::env::set_var(SERVICE_SOCKET_PATH_ENV, "/tmp/sky-cua-explicit.sock");
            std::env::remove_var("XDG_RUNTIME_DIR");
        }
        let launch_environment = LaunchEnvironment::from_repaired_desktop_vars_for_tests(vec![(
            "XDG_RUNTIME_DIR".to_string(),
            "/run/user/1000".to_string(),
        )]);

        let endpoint = ServiceEndpoint::new(&launch_environment).expect("endpoint should resolve");

        restore_env(SERVICE_SOCKET_PATH_ENV, old_socket_path);
        restore_env("XDG_RUNTIME_DIR", old_runtime_dir);
        match endpoint {
            ServiceEndpoint::Unix(path) => {
                assert_eq!(path, PathBuf::from("/tmp/sky-cua-explicit.sock"));
            }
        }
    }

    #[test]
    fn closed_cached_connection_is_retryable() {
        let error = anyhow!("sky-cua-service connection closed before response");

        assert!(is_stale_stream_error(&error));
    }

    fn action_request() -> sky_cua_platform::ActionRequest {
        sky_cua_platform::ActionRequest {
            action: sky_cua_platform::ActionName::Click,
            snapshot_id: None,
            element_index: None,
            arguments: serde_json::json!({}),
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: None,
            resolved_focused_app: None,
            environment: None,
        }
    }

    #[test]
    fn idempotent_request_is_retried_after_an_after_write_failure() {
        let request = ServiceRequest::Health;
        let failure = CallFailure::AfterWrite(anyhow!("boom"));

        assert!(should_retry(&request, &failure));
    }

    #[test]
    fn execute_action_after_write_failure_is_not_retried() {
        let request = ServiceRequest::ExecuteAction {
            request: Box::new(action_request()),
        };
        let failure =
            CallFailure::AfterWrite(anyhow!("sky-cua-service connection closed before response"));

        assert!(!should_retry(&request, &failure));

        let error = ambiguous_failure_error(failure.into());
        assert!(
            error.to_string().contains("may or may not have executed"),
            "unexpected error message: {error}"
        );
    }

    #[test]
    fn execute_action_before_write_failure_is_retried() {
        let request = ServiceRequest::ExecuteAction {
            request: Box::new(action_request()),
        };
        let failure = CallFailure::BeforeWrite(anyhow!(
            "failed to connect to sky-cua-service socket: connection refused"
        ));

        assert!(should_retry(&request, &failure));
    }

    #[test]
    fn any_request_write_failure_is_ambiguous_for_mutations() {
        let request = ServiceRequest::ExecuteAction {
            request: Box::new(action_request()),
        };

        // A failed payload/newline/flush write can leave a complete JSON frame
        // readable at EOF. The transport must classify every such failure as
        // after-dispatch instead of replaying a mutation on a fresh socket.
        let failure = CallFailure::AfterWrite(anyhow!("request write failed"));
        assert!(!should_retry(&request, &failure));
        assert!(should_retry(&ServiceRequest::Health, &failure));
    }

    #[test]
    fn browser_click_is_non_idempotent_but_browser_status_is_idempotent() {
        let click = ServiceRequest::Browser {
            identity: None,
            context: None,
            request: sky_cua_platform::BrowserRequest::Click {
                target: Some(sky_cua_platform::BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                x: 10.0,
                y: 10.0,
            },
        };
        let status = ServiceRequest::Browser {
            request: sky_cua_platform::BrowserRequest::Status,
            identity: None,
            context: None,
        };
        let after_write = CallFailure::AfterWrite(anyhow!("boom"));

        assert!(!should_retry(&click, &after_write));
        assert!(should_retry(&status, &after_write));
    }

    #[test]
    fn should_retry_error_recovers_call_failure_stage_from_the_error_chain() {
        let request = ServiceRequest::ExecuteAction {
            request: Box::new(action_request()),
        };
        let after_write_error: anyhow::Error =
            CallFailure::AfterWrite(anyhow!("connection closed before response")).into();
        let before_write_error: anyhow::Error =
            CallFailure::BeforeWrite(anyhow!("connection refused")).into();

        assert!(!should_retry_error(&request, &after_write_error));
        assert!(should_retry_error(&request, &before_write_error));
    }

    #[test]
    fn startup_health_stale_error_is_typed() {
        let error = anyhow!(StaleStartupService {
            detail: "DISPLAY".to_string(),
            owner_pid: Some(4242),
        });

        assert!(is_stale_startup_health_error(&error));
        assert_eq!(
            error
                .downcast_ref::<StaleStartupService>()
                .and_then(|error| error.owner_pid),
            Some(4242)
        );
    }

    #[test]
    fn browser_control_health_mode_matrix_is_backward_compatible_and_fail_closed() {
        use PlatformBrowserControlMode::{Hybrid, Legacy, Strict};

        assert!(ensure_browser_control_mode_compatible(Legacy, None).is_ok());
        assert!(ensure_browser_control_mode_compatible(Legacy, Some(Legacy)).is_ok());
        assert!(ensure_browser_control_mode_compatible(Hybrid, Some(Hybrid)).is_ok());
        assert!(ensure_browser_control_mode_compatible(Hybrid, Some(Strict)).is_ok());
        assert!(ensure_browser_control_mode_compatible(Strict, Some(Strict)).is_ok());

        assert!(ensure_browser_control_mode_compatible(Hybrid, None).is_err());
        assert!(ensure_browser_control_mode_compatible(Strict, None).is_err());
        assert!(ensure_browser_control_mode_compatible(Strict, Some(Hybrid)).is_err());
        assert!(ensure_browser_control_mode_compatible(Legacy, Some(Hybrid)).is_err());
    }

    #[test]
    fn health_capabilities_report_the_daemon_browser_control_mode() {
        let response = ServiceResponse::Health {
            ok: true,
            service_socket: "/tmp/sky-cua/service.sock".to_owned(),
            protocol_version: 1,
            service_version: "0.1.0".to_owned(),
            capabilities: vec![sky_cua_platform::model::browser_control_mode_capability(
                PlatformBrowserControlMode::Strict,
            )],
            desktop_env: Default::default(),
            browser_env: Default::default(),
        };
        assert_eq!(
            reported_browser_control_mode(&response),
            Some(PlatformBrowserControlMode::Strict)
        );
    }

    #[test]
    fn unix_service_termination_ignores_unverified_lock_pid_when_peer_is_known() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sky-cua-client-peer-pid-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("create test temp dir");
        let socket_path = temp_dir.join("service.sock");
        fs::write(
            crate::daemon_singleton::socket_lock_path(&socket_path),
            "4242\n",
        )
        .expect("write lock pid");

        let pids =
            owner_pids_for_termination(&socket_path, Some(7777)).expect("pid should resolve");

        let _ = fs::remove_dir_all(&temp_dir);
        assert_eq!(pids, vec![7777]);
    }

    #[test]
    fn startup_health_budget_allows_slow_desktop_service_startup() {
        let budget = STARTUP_POLL_INTERVAL * STARTUP_HEALTH_ATTEMPTS as u32;

        assert!(budget >= Duration::from_secs(20));
        assert!(budget < Duration::from_secs(30));
    }

    #[test]
    fn respawns_service_after_child_exits() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        let temp_dir = std::env::temp_dir().join(format!(
            "sky-cua-client-respawn-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).expect("create test temp dir");
        let service_script = temp_dir.join("fake-sky-cua-service");
        let socket_path = temp_dir.join("service.sock");
        fs::write(&service_script, FAKE_SERVICE).expect("write fake service script");
        fs::set_permissions(&service_script, fs::Permissions::from_mode(0o755))
            .expect("make fake service executable");

        let old_service_path = std::env::var_os("SKY_CUA_SERVICE_PATH");
        let old_socket_path = std::env::var_os(SERVICE_SOCKET_PATH_ENV);
        let old_browser_control_mode =
            std::env::var_os(sky_cua_platform::config::BROWSER_CONTROL_MODE_ENV);
        unsafe {
            std::env::set_var("SKY_CUA_SERVICE_PATH", &service_script);
            std::env::set_var(SERVICE_SOCKET_PATH_ENV, &socket_path);
            std::env::set_var(sky_cua_platform::config::BROWSER_CONTROL_MODE_ENV, "legacy");
        }

        let result = run_respawn_test();

        restore_env("SKY_CUA_SERVICE_PATH", old_service_path);
        restore_env(SERVICE_SOCKET_PATH_ENV, old_socket_path);
        restore_env(
            sky_cua_platform::config::BROWSER_CONTROL_MODE_ENV,
            old_browser_control_mode,
        );
        let _ = fs::remove_dir_all(&temp_dir);

        result.expect("service client should respawn exited child");
    }

    fn run_respawn_test() -> Result<()> {
        let client = ServiceClient::connect_or_spawn()?;
        let first_child_id = child_id(&client)?;
        anyhow::ensure!(
            matches!(
                client.call(&ServiceRequest::Health)?,
                ServiceResponse::Health { ok: true, .. }
            ),
            "initial health call did not return ok"
        );

        terminate_child(&client)?;
        anyhow::ensure!(
            matches!(
                client.call(&ServiceRequest::Health)?,
                ServiceResponse::Health { ok: true, .. }
            ),
            "respawned health call did not return ok"
        );
        let second_child_id = child_id(&client)?;
        anyhow::ensure!(
            first_child_id != second_child_id,
            "service child id did not change after respawn"
        );
        terminate_child(&client)?;
        Ok(())
    }

    fn command_env_value<'a>(
        command: &'a Command,
        key: &str,
    ) -> Option<Option<&'a std::ffi::OsStr>> {
        command
            .get_envs()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, value)| value)
    }

    fn child_id(client: &ServiceClient) -> Result<u32> {
        let child_guard = client
            .child
            .lock()
            .map_err(|_| anyhow!("child state mutex was poisoned"))?;
        child_guard
            .as_ref()
            .map(Child::id)
            .ok_or_else(|| anyhow!("expected spawned service child"))
    }

    fn terminate_child(client: &ServiceClient) -> Result<()> {
        let mut child_guard = client
            .child
            .lock()
            .map_err(|_| anyhow!("child state mutex was poisoned"))?;
        if let Some(child) = child_guard.as_mut() {
            child.kill()?;
            let _ = child.wait()?;
        }
        Ok(())
    }

    fn restore_env(key: &str, old_value: Option<std::ffi::OsString>) {
        unsafe {
            if let Some(value) = old_value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }

    const FAKE_SERVICE: &str = r#"#!/usr/bin/env python3
import json
import os
import socket
import sys

if len(sys.argv) < 2 or sys.argv[1] != "daemon":
    raise SystemExit("expected daemon mode")

path = os.environ["SKY_CUA_SERVICE_SOCKET_PATH"]
try:
    os.unlink(path)
except FileNotFoundError:
    pass

server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(path)
server.listen(8)

while True:
    conn, _ = server.accept()
    with conn:
        data = b""
        while not data.endswith(b"\n"):
            chunk = conn.recv(4096)
            if not chunk:
                break
            data += chunk
        if not data:
            continue
        request = json.loads(data.decode("utf-8"))
        if request.get("type") == "health":
            response = {
                "type": "health",
                "ok": True,
                "service_socket": path,
                "desktop_env": {
                    key: os.environ[key] for key in [
                        "DBUS_SESSION_BUS_ADDRESS",
                        "DESKTOP_SESSION",
                        "DISPLAY",
                        "PATH",
                        "WAYLAND_DISPLAY",
                        "XDG_CURRENT_DESKTOP",
                        "XDG_RUNTIME_DIR",
                        "XDG_SESSION_TYPE",
                    ]
                    if os.environ.get(key)
                },
                "browser_env": {
                    key: os.environ[key] for key in [
                        "SKY_CUA_BROWSER_USE_SOCKET_DIR",
                        "CODEX_BROWSER_USE_SOCKET_DIR",
                        "SKY_CUA_BROWSER",
                    ]
                    if os.environ.get(key)
                },
            }
        else:
            response = {"type": "error", "code": "UnexpectedRequest", "message": request.get("type", "<missing>")}
        conn.sendall(json.dumps(response).encode("utf-8") + b"\n")
"#;
}
