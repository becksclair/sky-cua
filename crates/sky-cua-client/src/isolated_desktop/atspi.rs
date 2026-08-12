//! Private AT-SPI bootstrap and teardown for an isolated Xpra session.
//!
//! Xpra supplies a private session bus, but it does not guarantee that the
//! accessibility bus launcher or registry are ready on that bus. This module
//! brings them up before the isolated service starts, records their exact D-Bus
//! owners, and tears down only owners that still match that recorded private
//! session. It never calls the host user-systemd AT-SPI repair path.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use zbus::blocking::{Connection, Proxy, connection, fdo::DBusProxy};
use zbus::names::{BusName, WellKnownName};

use super::probe::{AT_SPI_BUS_LAUNCHER_PATH, AT_SPI_REGISTRY_PATH};
use super::sky_cua_runtime_dir;

const A11Y_BUS_NAME: &str = "org.a11y.Bus";
const A11Y_BUS_PATH: &str = "/org/a11y/bus";
const A11Y_BUS_INTERFACE: &str = "org.a11y.Bus";
const REGISTRY_NAME: &str = "org.a11y.atspi.Registry";
const REGISTRY_PATH_DBUS: &str = "/org/a11y/atspi/registry";
const REGISTRY_INTERFACE: &str = "org.a11y.atspi.Registry";
const DBUS_METHOD_TIMEOUT: Duration = Duration::from_secs(3);
const OWNER_READY_TIMEOUT: Duration = Duration::from_secs(5);
const OWNER_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STATE_VERSION: u32 = 1;
static NEXT_STATE_TEMP: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
struct IsolatedAtspiEnv {
    display: String,
    session_bus_address: String,
    xauthority: String,
}

#[derive(Debug)]
struct SpawnedProcess {
    pid: u32,
    child: Option<Child>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProcessIdentity {
    pid: u32,
    start_ticks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AtspiSessionState {
    version: u32,
    display: String,
    xauthority: String,
    session_bus_address: String,
    accessibility_bus_address: String,
    launcher: ProcessIdentity,
    registry: ProcessIdentity,
    registry_direct: bool,
}

trait AtspiOps {
    fn owner_pid(&mut self, bus_address: &str, name: &str) -> Result<Option<u32>>;
    fn accessibility_bus_address(&mut self, session_bus_address: &str) -> Result<String>;
    fn activate_registry(&mut self, accessibility_bus_address: &str) -> Result<()>;
    fn probe_registry(&mut self, accessibility_bus_address: &str) -> Result<()>;
    fn spawn_launcher(&mut self, env: &IsolatedAtspiEnv) -> Result<SpawnedProcess>;
    fn spawn_registry(
        &mut self,
        env: &IsolatedAtspiEnv,
        accessibility_bus_address: &str,
    ) -> Result<SpawnedProcess>;
    fn validate_process(
        &mut self,
        pid: u32,
        executable: &Path,
        env: &IsolatedAtspiEnv,
        accessibility_bus_address: Option<&str>,
    ) -> Result<u64>;
    fn terminate_process(&mut self, pid: u32) -> Result<()>;
    fn terminate_spawned(&mut self, process: &mut SpawnedProcess);
    fn sleep(&mut self, duration: Duration);
}

#[derive(Debug, Default)]
struct ProcessAtspiOps;

impl AtspiOps for ProcessAtspiOps {
    fn owner_pid(&mut self, bus_address: &str, name: &str) -> Result<Option<u32>> {
        let connection = connect_bus(bus_address)?;
        owner_pid_on_connection(&connection, name)
    }

    fn accessibility_bus_address(&mut self, session_bus_address: &str) -> Result<String> {
        let connection = connect_bus(session_bus_address)
            .context("failed to connect to the private Xpra session bus")?;
        let proxy = Proxy::new(
            &connection,
            A11Y_BUS_NAME,
            A11Y_BUS_PATH,
            A11Y_BUS_INTERFACE,
        )
        .context("failed to create the private org.a11y.Bus proxy")?;
        let address: String = proxy
            .call("GetAddress", &())
            .context("private org.a11y.Bus.GetAddress failed")?;
        if address.trim().is_empty() {
            bail!("private org.a11y.Bus.GetAddress returned an empty address");
        }
        Ok(address)
    }

    fn activate_registry(&mut self, accessibility_bus_address: &str) -> Result<()> {
        let connection = connect_bus(accessibility_bus_address)
            .context("failed to connect to the private accessibility bus")?;
        let proxy = DBusProxy::new(&connection)
            .context("failed to create the private accessibility-bus D-Bus proxy")?;
        let name = WellKnownName::try_from(REGISTRY_NAME)
            .context("the AT-SPI registry well-known name is invalid")?;
        proxy
            .start_service_by_name(name, 0)
            .context("private accessibility-bus registry activation failed")?;
        Ok(())
    }

    fn probe_registry(&mut self, accessibility_bus_address: &str) -> Result<()> {
        let connection = connect_bus(accessibility_bus_address)
            .context("failed to reconnect to the private accessibility bus")?;
        let proxy = Proxy::new(
            &connection,
            REGISTRY_NAME,
            REGISTRY_PATH_DBUS,
            REGISTRY_INTERFACE,
        )
        .context("failed to create the private AT-SPI registry proxy")?;
        proxy
            .call_method("GetRegisteredEvents", &())
            .context("the private AT-SPI registry did not answer GetRegisteredEvents")?;
        Ok(())
    }

    fn spawn_launcher(&mut self, env: &IsolatedAtspiEnv) -> Result<SpawnedProcess> {
        ensure_executable(Path::new(AT_SPI_BUS_LAUNCHER_PATH))?;
        let mut command = Command::new(AT_SPI_BUS_LAUNCHER_PATH);
        command.arg("--launch-immediately");
        configure_child(&mut command, env, None);
        spawn_child(command, "private AT-SPI bus launcher")
    }

    fn spawn_registry(
        &mut self,
        env: &IsolatedAtspiEnv,
        accessibility_bus_address: &str,
    ) -> Result<SpawnedProcess> {
        ensure_executable(Path::new(AT_SPI_REGISTRY_PATH))?;
        let mut command = Command::new(AT_SPI_REGISTRY_PATH);
        command.arg("--use-gnome-session");
        configure_child(&mut command, env, Some(accessibility_bus_address));
        spawn_child(command, "private AT-SPI registry")
    }

    fn validate_process(
        &mut self,
        pid: u32,
        executable: &Path,
        env: &IsolatedAtspiEnv,
        accessibility_bus_address: Option<&str>,
    ) -> Result<u64> {
        validate_process_identity(pid, executable, env, accessibility_bus_address)
    }

    fn terminate_process(&mut self, pid: u32) -> Result<()> {
        let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error)
                    .with_context(|| format!("failed to terminate private pid {pid}"));
            }
        }
        Ok(())
    }

    fn terminate_spawned(&mut self, process: &mut SpawnedProcess) {
        if let Some(child) = process.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn sleep(&mut self, duration: Duration) {
        thread::sleep(duration);
    }
}

/// Ensure the private Xpra session has a responsive AT-SPI registry before its
/// service or applications are launched.
pub(super) fn ensure_session(
    display: &str,
    session_bus_address: &str,
    xauthority: &str,
    display_number: u32,
) -> Result<()> {
    let state_path = state_path(display_number).ok_or_else(|| {
        anyhow!("cannot persist private AT-SPI ownership because XDG_RUNTIME_DIR is unset")
    })?;
    let env = IsolatedAtspiEnv {
        display: display.to_string(),
        session_bus_address: session_bus_address.to_string(),
        xauthority: xauthority.to_string(),
    };

    thread::Builder::new()
        .name(format!("sky-cua-atspi-{display_number}"))
        .spawn(move || {
            let mut ops = ProcessAtspiOps;
            let previous = read_state(&state_path).ok().flatten();
            ensure_with(&mut ops, &env, previous.as_ref(), |state| {
                persist_state(&state_path, state)
            })
            .map(|_| ())
        })
        .context("failed to start the private AT-SPI bootstrap thread")?
        .join()
        .map_err(|_| anyhow!("the private AT-SPI bootstrap thread panicked"))?
}

/// Best-effort teardown of the exact private AT-SPI owners recorded for this
/// display. A stale, foreign, or unverifiable process is never signalled.
pub(super) fn terminate_session(display_number: u32) {
    let Some(path) = state_path(display_number) else {
        return;
    };
    let state = match read_state(&path) {
        Ok(Some(state)) => state,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, "discarding unreadable isolated AT-SPI ownership state");
            let _ = fs::remove_file(path);
            return;
        }
    };

    let result = thread::Builder::new()
        .name(format!("sky-cua-atspi-stop-{display_number}"))
        .spawn(move || terminate_recorded_state(&state))
        .map_err(anyhow::Error::from)
        .and_then(|handle| match handle.join() {
            Ok(result) => result,
            Err(_) => Err(anyhow!("isolated AT-SPI teardown thread panicked")),
        });
    finish_termination(&path, result);
}

fn finish_termination(path: &Path, result: Result<()>) {
    match result {
        Ok(()) => {
            if let Err(error) = fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(%error, state = %path.display(), "failed to remove completed isolated AT-SPI ownership state");
            }
        }
        Err(error) => {
            tracing::warn!(%error, state = %path.display(), "isolated AT-SPI teardown was incomplete; retaining exact-owner state for retry");
        }
    }
}

fn bootstrap_with<O: AtspiOps>(
    ops: &mut O,
    env: &IsolatedAtspiEnv,
    previous: Option<&AtspiSessionState>,
) -> Result<AtspiSessionState> {
    let mut spawned = Vec::new();
    let result = (|| {
        let mut launcher_pid = ops.owner_pid(&env.session_bus_address, A11Y_BUS_NAME)?;
        if launcher_pid.is_none() {
            let launcher = ops.spawn_launcher(env)?;
            let spawned_pid = launcher.pid;
            spawned.push(launcher);
            launcher_pid = Some(wait_for_owner(
                ops,
                &env.session_bus_address,
                A11Y_BUS_NAME,
            )?);
            if launcher_pid != Some(spawned_pid) {
                // A concurrent ensure won the D-Bus name race. Reap only this
                // attempt's losing child, then adopt the winner after the same
                // executable/environment validation used for ordinary reuse.
                if let Some(mut losing_child) = spawned.pop() {
                    ops.terminate_spawned(&mut losing_child);
                }
            }
        }
        let launcher_pid = launcher_pid.expect("launcher owner was established");

        let accessibility_bus_address = ops.accessibility_bus_address(&env.session_bus_address)?;
        // Attribute the launcher before asking its accessibility bus to start
        // anything. A responsive private bus is not enough if its well-known
        // owner no longer belongs to this exact Xpra session.
        let launcher_start_ticks =
            ops.validate_process(launcher_pid, Path::new(AT_SPI_BUS_LAUNCHER_PATH), env, None)?;
        let launcher = ProcessIdentity {
            pid: launcher_pid,
            start_ticks: launcher_start_ticks,
        };
        let mut registry_direct = false;
        let mut reused_direct_registry = false;
        let mut registry_pid = ops.owner_pid(&accessibility_bus_address, REGISTRY_NAME)?;
        if let Some(registry_pid) = registry_pid {
            reused_direct_registry = previous.is_some_and(|previous| {
                previous_direct_registry_candidate(
                    previous,
                    env,
                    &accessibility_bus_address,
                    &launcher,
                    registry_pid,
                )
            });
            registry_direct = reused_direct_registry;
        }
        if registry_pid.is_none() {
            let activation_result = ops
                .activate_registry(&accessibility_bus_address)
                .and_then(|()| wait_for_owner(ops, &accessibility_bus_address, REGISTRY_NAME));
            match activation_result {
                Ok(pid) => registry_pid = Some(pid),
                Err(activation_error) => {
                    // Activation can be wired to an unavailable user-systemd unit
                    // or report success without ever establishing an owner even
                    // though this dedicated bus is sound. Fall back only after
                    // proving both buses still answer and the registry remains free.
                    let current_launcher =
                        ops.owner_pid(&env.session_bus_address, A11Y_BUS_NAME)?;
                    if current_launcher != Some(launcher_pid) {
                        return Err(activation_error).context(
                            "registry activation failed and the private org.a11y.Bus owner changed",
                        );
                    }
                    registry_pid = ops.owner_pid(&accessibility_bus_address, REGISTRY_NAME)?;
                    if registry_pid.is_none() {
                        // Recheck immediately before spawning to close the
                        // activation/direct-launch ownership race.
                        registry_pid = ops.owner_pid(&accessibility_bus_address, REGISTRY_NAME)?;
                    }
                    if registry_pid.is_none() {
                        let registry = ops
                            .spawn_registry(env, &accessibility_bus_address)
                            .with_context(|| {
                                format!("{activation_error:#}; direct registry fallback failed")
                            })?;
                        let spawned_pid = registry.pid;
                        spawned.push(registry);
                        registry_direct = true;
                        registry_pid = Some(wait_for_owner(
                            ops,
                            &accessibility_bus_address,
                            REGISTRY_NAME,
                        )?);
                        if registry_pid != Some(spawned_pid) {
                            if let Some(mut losing_child) = spawned.pop() {
                                ops.terminate_spawned(&mut losing_child);
                            }
                            registry_direct = false;
                        }
                    }
                }
            }
        }
        let registry_pid = registry_pid.expect("registry owner was established");

        ops.probe_registry(&accessibility_bus_address)?;
        let registry_start_ticks = ops.validate_process(
            registry_pid,
            Path::new(AT_SPI_REGISTRY_PATH),
            env,
            registry_direct.then_some(accessibility_bus_address.as_str()),
        )?;

        let state = AtspiSessionState {
            version: STATE_VERSION,
            display: env.display.clone(),
            xauthority: env.xauthority.clone(),
            session_bus_address: env.session_bus_address.clone(),
            accessibility_bus_address,
            launcher,
            registry: ProcessIdentity {
                pid: registry_pid,
                start_ticks: registry_start_ticks,
            },
            registry_direct,
        };
        if reused_direct_registry
            && !previous.is_some_and(|previous| preserves_direct_registry(previous, &state))
        {
            bail!("persisted direct AT-SPI registry generation changed during reuse");
        }
        Ok(state)
    })();

    if result.is_err() {
        for process in spawned.iter_mut().rev() {
            ops.terminate_spawned(process);
        }
    }
    result
}

fn ensure_with<O, P>(
    ops: &mut O,
    env: &IsolatedAtspiEnv,
    previous: Option<&AtspiSessionState>,
    persist: P,
) -> Result<AtspiSessionState>
where
    O: AtspiOps,
    P: FnOnce(&AtspiSessionState) -> Result<()>,
{
    let state = bootstrap_with(ops, env, previous)?;

    // `previous` was read from this display's durable state immediately before
    // bootstrap. If every validated owner generation and session attribute is
    // unchanged, rewriting the same record adds no durability but turns a
    // transient filesystem error into destructive teardown of a healthy reused
    // accessibility session.
    if previous == Some(&state) {
        return Ok(state);
    }

    if let Err(persist_error) = persist(&state) {
        let cleanup_result = terminate_recorded_state_with(ops, &state);
        return match cleanup_result {
            Ok(()) => Err(persist_error),
            Err(cleanup_error) => Err(persist_error.context(format!(
                "failed to clean up private AT-SPI owners after persistence failure: {cleanup_error:#}"
            ))),
        };
    }
    Ok(state)
}

fn wait_for_owner<O: AtspiOps>(ops: &mut O, bus_address: &str, name: &str) -> Result<u32> {
    let deadline = Instant::now() + OWNER_READY_TIMEOUT;
    loop {
        if let Some(pid) = ops.owner_pid(bus_address, name)? {
            return Ok(pid);
        }
        if Instant::now() >= deadline {
            bail!("{name} did not acquire its private D-Bus name within {OWNER_READY_TIMEOUT:?}");
        }
        ops.sleep(OWNER_POLL_INTERVAL);
    }
}

fn connect_bus(address: &str) -> Result<Connection> {
    connection::Builder::address(address)
        .context("invalid private D-Bus address")?
        .method_timeout(DBUS_METHOD_TIMEOUT)
        .build()
        .context("failed to connect to a private D-Bus address")
}

fn owner_pid_on_connection(connection: &Connection, name: &str) -> Result<Option<u32>> {
    let proxy = DBusProxy::new(connection).context("failed to create a D-Bus daemon proxy")?;
    let bus_name = BusName::try_from(name).context("invalid D-Bus owner name")?;
    if !proxy
        .name_has_owner(bus_name.clone())
        .context("failed to query private D-Bus name ownership")?
    {
        return Ok(None);
    }
    proxy
        .get_connection_unix_process_id(bus_name)
        .map(Some)
        .context("failed to resolve the private D-Bus owner pid")
}

fn configure_child(
    command: &mut Command,
    env: &IsolatedAtspiEnv,
    accessibility_bus_address: Option<&str>,
) {
    command
        .env("DISPLAY", &env.display)
        .env("XAUTHORITY", &env.xauthority)
        .env("DBUS_SESSION_BUS_ADDRESS", &env.session_bus_address)
        .env("XDG_SESSION_TYPE", "x11")
        .env("NO_AT_BRIDGE", "0")
        .env("ACCESSIBILITY_ENABLED", "1")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("AT_SPI_BUS_ADDRESS")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(address) = accessibility_bus_address {
        command.env("AT_SPI_BUS_ADDRESS", address);
    }
    // These are persistent session resources. Detach them from the transient
    // MCP client's process group; D-Bus ownership and the persisted Xpra-scoped
    // state remain the lifecycle authority.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn spawn_child(mut command: Command, description: &str) -> Result<SpawnedProcess> {
    let child = command
        .spawn()
        .with_context(|| format!("failed to spawn the {description}"))?;
    Ok(SpawnedProcess {
        pid: child.id(),
        child: Some(child),
    })
}

fn ensure_executable(path: &Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        bail!(
            "isolated desktop AT-SPI dependency {} is missing; install at-spi2-core",
            path.display()
        )
    }
}

fn validate_process_identity(
    pid: u32,
    expected_executable: &Path,
    expected_env: &IsolatedAtspiEnv,
    expected_accessibility_bus: Option<&str>,
) -> Result<u64> {
    let actual_executable = fs::read_link(format!("/proc/{pid}/exe"))
        .with_context(|| format!("cannot inspect executable for private AT-SPI owner pid {pid}"))?;
    let expected_executable = fs::canonicalize(expected_executable).with_context(|| {
        format!(
            "cannot canonicalize private AT-SPI executable {}",
            expected_executable.display()
        )
    })?;
    if actual_executable != expected_executable {
        bail!(
            "private AT-SPI owner pid {pid} runs {}, expected {}",
            actual_executable.display(),
            expected_executable.display()
        );
    }

    let environ = read_process_environment(pid)?;
    validate_process_environment(&environ, expected_env, expected_accessibility_bus, pid)?;
    process_start_ticks(pid)
}

fn validate_process_environment(
    environ: &BTreeMap<String, String>,
    expected_env: &IsolatedAtspiEnv,
    expected_accessibility_bus: Option<&str>,
    pid: u32,
) -> Result<()> {
    require_env(environ, "DISPLAY", &expected_env.display, pid)?;
    require_env(
        environ,
        "DBUS_SESSION_BUS_ADDRESS",
        &expected_env.session_bus_address,
        pid,
    )?;
    require_env(environ, "XAUTHORITY", &expected_env.xauthority, pid)?;
    require_env(environ, "XDG_SESSION_TYPE", "x11", pid)?;
    require_env(environ, "NO_AT_BRIDGE", "0", pid)?;
    require_env(environ, "ACCESSIBILITY_ENABLED", "1", pid)?;
    if environ.contains_key("WAYLAND_DISPLAY") {
        bail!("private AT-SPI owner pid {pid} still carries WAYLAND_DISPLAY");
    }
    match expected_accessibility_bus {
        Some(address) => require_env(environ, "AT_SPI_BUS_ADDRESS", address, pid)?,
        None if environ.contains_key("AT_SPI_BUS_ADDRESS") => {
            bail!("private AT-SPI owner pid {pid} carries unexpected AT_SPI_BUS_ADDRESS")
        }
        None => {}
    }
    Ok(())
}

fn read_process_environment(pid: u32) -> Result<BTreeMap<String, String>> {
    let raw = fs::read(format!("/proc/{pid}/environ"))
        .with_context(|| format!("cannot read environment for private AT-SPI owner pid {pid}"))?;
    Ok(raw
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let split = entry.iter().position(|byte| *byte == b'=')?;
            Some((
                String::from_utf8_lossy(&entry[..split]).into_owned(),
                String::from_utf8_lossy(&entry[split + 1..]).into_owned(),
            ))
        })
        .collect())
}

fn require_env(
    environ: &BTreeMap<String, String>,
    key: &str,
    expected: &str,
    pid: u32,
) -> Result<()> {
    match environ.get(key).map(String::as_str) {
        Some(actual) if actual == expected => Ok(()),
        actual => {
            bail!("private AT-SPI owner pid {pid} has {key}={actual:?}, expected {expected:?}")
        }
    }
}

fn process_start_ticks(pid: u32) -> Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
        .with_context(|| format!("cannot read start time for private AT-SPI owner pid {pid}"))?;
    let (_, fields) = stat
        .rsplit_once(") ")
        .ok_or_else(|| anyhow!("malformed /proc/{pid}/stat"))?;
    fields
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| anyhow!("missing start time in /proc/{pid}/stat"))?
        .parse::<u64>()
        .with_context(|| format!("invalid start time in /proc/{pid}/stat"))
}

fn state_path(display_number: u32) -> Option<PathBuf> {
    sky_cua_runtime_dir().map(|dir| dir.join(format!("isolated-atspi-{display_number}.json")))
}

fn persist_state(path: &Path, state: &AtspiSessionState) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("private AT-SPI state path has no parent"))?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create private AT-SPI state dir {}",
            parent.display()
        )
    })?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("isolated-atspi");
    let temporary = path.with_file_name(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        NEXT_STATE_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut bytes = serde_json::to_vec(state)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .with_context(|| {
            format!(
                "failed to create private AT-SPI state {}",
                temporary.display()
            )
        })?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        fs::File::open(parent)?.sync_all()?;
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("failed to persist private AT-SPI state {}", path.display()))
}

fn read_state(path: &Path) -> Result<Option<AtspiSessionState>> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let state: AtspiSessionState = serde_json::from_slice(&raw)?;
    if state.version != STATE_VERSION {
        bail!(
            "unsupported isolated AT-SPI state version {}",
            state.version
        );
    }
    Ok(Some(state))
}

fn terminate_recorded_state(state: &AtspiSessionState) -> Result<()> {
    let mut ops = ProcessAtspiOps;
    terminate_recorded_state_with(&mut ops, state)
}

fn terminate_recorded_state_with<O: AtspiOps>(
    ops: &mut O,
    state: &AtspiSessionState,
) -> Result<()> {
    let mut errors = Vec::new();
    if let Err(error) = terminate_recorded_process(
        ops,
        &state.accessibility_bus_address,
        REGISTRY_NAME,
        &state.registry,
        Path::new(AT_SPI_REGISTRY_PATH),
        state,
        state
            .registry_direct
            .then_some(state.accessibility_bus_address.as_str()),
    ) {
        errors.push(error.context("private AT-SPI registry teardown failed"));
    }
    if let Err(error) = terminate_recorded_process(
        ops,
        &state.session_bus_address,
        A11Y_BUS_NAME,
        &state.launcher,
        Path::new(AT_SPI_BUS_LAUNCHER_PATH),
        state,
        None,
    ) {
        errors.push(error.context("private AT-SPI bus launcher teardown failed"));
    }
    if errors.is_empty() {
        return Ok(());
    }
    let details = errors
        .into_iter()
        .map(|error| format!("{error:#}"))
        .collect::<Vec<_>>()
        .join("; ");
    Err(anyhow!(
        "isolated AT-SPI teardown was incomplete: {details}"
    ))
}

fn terminate_recorded_process<O: AtspiOps>(
    ops: &mut O,
    bus_address: &str,
    name: &str,
    expected: &ProcessIdentity,
    executable: &Path,
    state: &AtspiSessionState,
    accessibility_bus_address: Option<&str>,
) -> Result<()> {
    let Some(owner_pid) = ops.owner_pid(bus_address, name)? else {
        return Ok(());
    };
    let env = IsolatedAtspiEnv {
        display: state.display.clone(),
        session_bus_address: state.session_bus_address.clone(),
        xauthority: state.xauthority.clone(),
    };
    let actual_start = match ops.validate_process(
        owner_pid,
        executable,
        &env,
        accessibility_bus_address,
    ) {
        Ok(start) => start,
        Err(error) => {
            tracing::warn!(%error, pid = owner_pid, %name, "refusing to signal mismatched AT-SPI owner");
            return Ok(());
        }
    };
    if !recorded_owner_matches(expected, owner_pid, actual_start) {
        tracing::warn!(
            pid = owner_pid,
            expected_pid = expected.pid,
            expected_start_ticks = expected.start_ticks,
            actual_start_ticks = actual_start,
            %name,
            "refusing to signal a changed AT-SPI owner generation"
        );
        return Ok(());
    }
    ops.terminate_process(owner_pid)
}

fn recorded_owner_matches(expected: &ProcessIdentity, owner_pid: u32, start_ticks: u64) -> bool {
    expected.pid == owner_pid && expected.start_ticks == start_ticks
}

fn preserves_direct_registry(previous: &AtspiSessionState, current: &AtspiSessionState) -> bool {
    previous.registry_direct
        && previous.version == current.version
        && previous.display == current.display
        && previous.xauthority == current.xauthority
        && previous.session_bus_address == current.session_bus_address
        && previous.accessibility_bus_address == current.accessibility_bus_address
        && previous.launcher == current.launcher
        && previous.registry == current.registry
}

fn previous_direct_registry_candidate(
    previous: &AtspiSessionState,
    env: &IsolatedAtspiEnv,
    accessibility_bus_address: &str,
    launcher: &ProcessIdentity,
    registry_pid: u32,
) -> bool {
    previous.registry_direct
        && previous.version == STATE_VERSION
        && previous.display == env.display
        && previous.xauthority == env.xauthority
        && previous.session_bus_address == env.session_bus_address
        && previous.accessibility_bus_address == accessibility_bus_address
        && previous.launcher == *launcher
        && previous.registry.pid == registry_pid
}

#[cfg(test)]
mod tests;
