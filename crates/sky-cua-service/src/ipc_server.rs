use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use sky_cua_platform::model::{ServiceRequest, ServiceResponse};
#[cfg(windows)]
use sky_cua_platform::service_tcp_addr;
#[cfg(unix)]
use sky_cua_platform::{SERVICE_SOCKET_PATH_ENV, service_socket_path};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
#[cfg(windows)]
use tokio::net::{TcpListener, TcpStream};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

use crate::daemon::ServiceDaemon;

const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Cap on a single IPC line (request or response). Screenshots travel
/// base64-inline in responses; requests are small. 64MiB is generous
/// headroom while still bounding memory against a wedged/malicious peer.
const MAX_IPC_LINE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Default)]
struct ConnectionTracker {
    active_connections: tokio::sync::Mutex<usize>,
}

impl ConnectionTracker {
    async fn register(&self) {
        let mut active_connections = self.active_connections.lock().await;
        *active_connections += 1;
    }

    async fn unregister_and_cleanup_if_idle(&self, daemon: &ServiceDaemon) {
        self.unregister_and_cleanup_with(|| daemon.hide_agent_cursor_after_last_client())
            .await;
    }

    async fn unregister_and_cleanup_with<F, Fut>(&self, cleanup: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let mut active_connections = self.active_connections.lock().await;
        *active_connections = active_connections.saturating_sub(1);
        if *active_connections == 0 {
            cleanup().await;
        }
    }

    async fn is_idle(&self) -> bool {
        *self.active_connections.lock().await == 0
    }
}

#[cfg(unix)]
pub async fn run_service() -> Result<()> {
    let socket_path = service_socket_path();
    let socket_path_overridden = std::env::var_os(SERVICE_SOCKET_PATH_ENV).is_some();
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
        if !socket_path_overridden {
            set_owner_only_permissions(parent)?;
        }
    }

    // Singleton guard: only the flock holder may unlink and bind the socket
    // path. Without it, concurrent spawns stomp a healthy daemon's socket and
    // an orphaned daemon's shutdown deletes the current owner's socket file,
    // leaving windows where new clients fail to connect and hosts lose the
    // whole tool surface for a turn.
    let singleton_lock = match acquire_singleton_lock(&socket_path)? {
        Some(lock) => lock,
        None => {
            info!(
                "another sky-cua-service instance owns {}; exiting so clients use it",
                socket_path.display()
            );
            return Ok(());
        }
    };

    if socket_path.exists() {
        let _ = tokio::fs::remove_file(&socket_path).await;
    }

    let mut listener = UnixListener::bind(&socket_path)?;
    // The socket peer is the authorization boundary for privileged requests
    // (session presence can unlock the desktop), so restrict the inode itself
    // even when the path override or temp-dir fallback skips the parent
    // tightening above.
    set_socket_owner_only_permissions(&socket_path)?;
    let daemon = Arc::new(ServiceDaemon::new(socket_path.clone()).await?);
    let _overlay_watchdog = daemon.spawn_overlay_idle_watchdog();
    let _session_presence_watchdog = daemon.spawn_session_presence_watchdog();
    let _scrcpy_liveness_watchdog = daemon.spawn_scrcpy_liveness_watchdog();
    let _phone_overlay_idle_watchdog = daemon.spawn_phone_overlay_idle_watchdog();
    let connections = Arc::new(ConnectionTracker::default());
    info!("sky-cua-service listening on {}", socket_path.display());

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        spawn_connection_handler(stream, &daemon, &connections).await;
                    }
                    Err(error) => {
                        warn!("sky-cua IPC accept error: {error}");
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                // Recreate the socket path if something unlinked it (a stale
                // pre-singleton daemon's shutdown, runtime-dir cleanup). The
                // existing listener keeps serving connected clients, but new
                // clients get ENOENT until the path exists again; holding the
                // singleton lock makes re-binding safe.
                if !socket_path.exists() {
                    match UnixListener::bind(&socket_path) {
                        Ok(replacement) => {
                            if let Err(error) = set_socket_owner_only_permissions(&socket_path) {
                                warn!(
                                    "failed to restrict re-bound socket permissions at {}: {error}",
                                    socket_path.display()
                                );
                            }
                            let displaced = std::mem::replace(&mut listener, replacement);
                            // Clients that completed connect(2) against the
                            // unlinked inode are still queued on the old
                            // listener; serve them instead of dropping them.
                            let drained =
                                drain_pending_connections(&displaced, &daemon, &connections).await;
                            warn!(
                                "sky-cua-service socket file disappeared at {}; re-bound the listener (served {drained} queued connection(s))",
                                socket_path.display()
                            );
                        }
                        Err(error) => {
                            warn!(
                                "sky-cua-service socket file disappeared at {} and re-binding failed: {error}; new clients will fail to connect until the daemon restarts",
                                socket_path.display()
                            );
                        }
                    }
                }

                if service_idle_timed_out(&daemon, &connections).await {
                    info!("sky-cua-service idle timeout reached; exiting");
                    break;
                }
            }
            _ = shutdown_signal() => {
                info!("sky-cua-service shutdown signal received; exiting");
                break;
            }
        }
    }

    // Unload this process's persistent KWin focus watcher so it does not
    // keep firing callbacks at a dead bus name after the daemon exits.
    #[cfg(target_os = "linux")]
    sky_cua_linux::kwin_script::shutdown().await;

    // Holding the singleton lock proves this daemon still owns the socket
    // path, so removing it here cannot delete a successor's socket.
    let _ = tokio::fs::remove_file(&socket_path).await;
    drop(singleton_lock);
    Ok(())
}

#[cfg(unix)]
async fn spawn_connection_handler(
    stream: UnixStream,
    daemon: &Arc<ServiceDaemon>,
    connections: &Arc<ConnectionTracker>,
) {
    let daemon = daemon.clone();
    connections.register().await;
    let connections = connections.clone();
    tokio::spawn(async move {
        if let Err(error) = handle_connection(stream, daemon.clone()).await {
            warn!("sky-cua IPC connection error: {error}");
        }
        connections.unregister_and_cleanup_if_idle(&daemon).await;
    });
}

/// Serve connections already queued on a displaced listener without waiting
/// for new ones. Bounded so a connect flood cannot pin the select loop.
#[cfg(unix)]
async fn drain_pending_connections(
    listener: &UnixListener,
    daemon: &Arc<ServiceDaemon>,
    connections: &Arc<ConnectionTracker>,
) -> usize {
    const MAX_DRAINED_CONNECTIONS: usize = 32;
    let mut drained = 0;
    while drained < MAX_DRAINED_CONNECTIONS {
        match tokio::time::timeout(Duration::ZERO, listener.accept()).await {
            Ok(Ok((stream, _))) => {
                spawn_connection_handler(stream, daemon, connections).await;
                drained += 1;
            }
            Ok(Err(_)) | Err(_) => break,
        }
    }
    drained
}

/// Try to take the exclusive daemon lock next to the socket path.
///
/// Returns `Ok(None)` when another live daemon holds the lock. The lock file
/// is intentionally never removed: unlinking it would let a third daemon
/// acquire a fresh lock on the recreated path while the old lock holder still
/// runs, which is the stomping bug this guard exists to prevent.
#[cfg(unix)]
fn acquire_singleton_lock(socket_path: &std::path::Path) -> Result<Option<std::fs::File>> {
    use std::io::Write;
    use std::os::unix::io::AsRawFd;

    let mut lock_name = socket_path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("service.sock"));
    lock_name.push(".lock");
    let lock_path = socket_path.with_file_name(lock_name);
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    match nonblocking_flock_outcome(|| {
        let result = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    })? {
        FlockOutcome::Acquired => {
            lock_file.set_len(0)?;
            (&lock_file).write_all(std::process::id().to_string().as_bytes())?;
            Ok(Some(lock_file))
        }
        FlockOutcome::Held => Ok(None),
    }
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum FlockOutcome {
    Acquired,
    Held,
}

/// Drive a non-blocking flock attempt to a terminal outcome, retrying EINTR:
/// a signal can interrupt even `LOCK_NB`, and that must not abort daemon
/// startup.
#[cfg(unix)]
fn nonblocking_flock_outcome(
    mut attempt: impl FnMut() -> std::io::Result<()>,
) -> Result<FlockOutcome> {
    loop {
        match attempt() {
            Ok(()) => return Ok(FlockOutcome::Acquired),
            Err(error) => match error.raw_os_error() {
                Some(libc::EWOULDBLOCK) => return Ok(FlockOutcome::Held),
                Some(libc::EINTR) => continue,
                _ => return Err(error.into()),
            },
        }
    }
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    match signal(SignalKind::terminate()) {
        Ok(mut signal) => {
            signal.recv().await;
        }
        Err(error) => {
            warn!("failed to install SIGTERM handler: {error}");
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(windows)]
pub async fn run_service() -> Result<()> {
    let bind_addr = service_tcp_addr();
    let listener = TcpListener::bind(&bind_addr).await?;
    let local_addr = listener.local_addr()?.to_string();
    let daemon = Arc::new(ServiceDaemon::new(local_addr.clone().into()).await?);
    let _overlay_watchdog = daemon.spawn_overlay_idle_watchdog();
    let _session_presence_watchdog = daemon.spawn_session_presence_watchdog();
    let _scrcpy_liveness_watchdog = daemon.spawn_scrcpy_liveness_watchdog();
    let _phone_overlay_idle_watchdog = daemon.spawn_phone_overlay_idle_watchdog();
    let connections = Arc::new(ConnectionTracker::default());
    info!("sky-cua-service listening on {}", local_addr);

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        let daemon = daemon.clone();
                        connections.register().await;
                        let connections = connections.clone();
                        tokio::spawn(async move {
                            if let Err(error) = handle_connection(stream, daemon.clone()).await {
                                warn!("sky-cua IPC connection error: {error}");
                            }
                            connections.unregister_and_cleanup_if_idle(&daemon).await;
                        });
                    }
                    Err(error) => {
                        warn!("sky-cua IPC accept error: {error}");
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                if service_idle_timed_out(&daemon, &connections).await {
                    info!("sky-cua-service idle timeout reached; exiting");
                    break;
                }
            }
        }
    }

    Ok(())
}

async fn service_idle_timed_out(daemon: &ServiceDaemon, connections: &ConnectionTracker) -> bool {
    connections.is_idle().await
        && daemon.idle_for().await >= IDLE_TIMEOUT
        // An idle exit kills the browser heartbeat keepalive, and the
        // extension then detaches the debugger from every tab; stay alive
        // while a browser session lingers (see browser/activity.rs).
        && !crate::browser::browser_session_lingering()
}

#[cfg(test)]
fn idle_timed_out(idle_for: Duration, active_connections: usize) -> bool {
    active_connections == 0 && idle_for >= IDLE_TIMEOUT
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(unix)]
fn set_socket_owner_only_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(unix)]
async fn handle_connection(stream: UnixStream, daemon: Arc<ServiceDaemon>) -> Result<()> {
    handle_stream(stream, daemon).await
}

#[cfg(windows)]
async fn handle_connection(stream: TcpStream, daemon: Arc<ServiceDaemon>) -> Result<()> {
    handle_stream(stream, daemon).await
}

async fn handle_stream<S>(stream: S, daemon: Arc<ServiceDaemon>) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    loop {
        let mut line = String::new();
        let read = {
            let mut limited = (&mut reader).take(MAX_IPC_LINE_BYTES);
            limited.read_line(&mut line).await?
        };
        if read == 0 {
            return Ok(());
        }
        if read as u64 == MAX_IPC_LINE_BYTES && !line.ends_with('\n') {
            // The stream is unsynchronized past this point (we stopped mid
            // frame, not at a line boundary), so closing the connection here
            // is correct, unlike the malformed-JSON case below.
            let response = oversized_request_response();
            write_response(&mut writer, &response).await?;
            return Ok(());
        }
        let response = match serde_json::from_str::<ServiceRequest>(line.trim_end()) {
            Ok(request) => daemon.handle(request).await,
            Err(error) => malformed_request_response(&error),
        };
        write_response(&mut writer, &response).await?;
    }
}

async fn write_response<W>(writer: &mut W, response: &ServiceResponse) -> Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let encoded = serde_json::to_vec(response)?;
    writer.write_all(&encoded).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

fn malformed_request_response(error: &serde_json::Error) -> ServiceResponse {
    ServiceResponse::Error {
        ok: false,
        code: "SKY_CUA_INVALID_REQUEST".to_string(),
        message: format!("failed to parse sky-cua IPC request as JSON: {error}"),
        session_id: None,
        turn_id: None,
        retry: None,
    }
}

fn oversized_request_response() -> ServiceResponse {
    ServiceResponse::Error {
        ok: false,
        code: "SKY_CUA_FRAME_TOO_LARGE".to_string(),
        message: format!(
            "sky-cua IPC request line exceeded {MAX_IPC_LINE_BYTES} bytes without a newline"
        ),
        session_id: None,
        turn_id: None,
        retry: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn flock_retries_eintr_until_a_terminal_outcome() {
        let mut results = vec![
            Err(std::io::Error::from_raw_os_error(libc::EINTR)),
            Err(std::io::Error::from_raw_os_error(libc::EINTR)),
            Ok(()),
        ]
        .into_iter();
        let outcome = nonblocking_flock_outcome(|| results.next().expect("attempt"))
            .expect("retry loop should reach the Ok attempt");
        assert_eq!(outcome, FlockOutcome::Acquired);

        let mut results = vec![
            Err(std::io::Error::from_raw_os_error(libc::EINTR)),
            Err(std::io::Error::from_raw_os_error(libc::EWOULDBLOCK)),
        ]
        .into_iter();
        let outcome = nonblocking_flock_outcome(|| results.next().expect("attempt"))
            .expect("held lock is a terminal outcome");
        assert_eq!(outcome, FlockOutcome::Held);

        let mut results = vec![Err(std::io::Error::from_raw_os_error(libc::EACCES))].into_iter();
        assert!(nonblocking_flock_outcome(|| results.next().expect("attempt")).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn drain_serves_connections_queued_on_a_displaced_listener() {
        let temp_dir =
            std::env::temp_dir().join(format!("sky-cua-drain-test-{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).expect("create test temp dir");
        let socket_path = temp_dir.join("drain.sock");
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).expect("bind test listener");
        let queued_one =
            std::os::unix::net::UnixStream::connect(&socket_path).expect("queue first client");
        let queued_two =
            std::os::unix::net::UnixStream::connect(&socket_path).expect("queue second client");

        let daemon = Arc::new(ServiceDaemon::new_for_tests().expect("daemon"));
        let connections = Arc::new(ConnectionTracker::default());
        let drained = drain_pending_connections(&listener, &daemon, &connections).await;

        drop(queued_one);
        drop(queued_two);
        let _ = std::fs::remove_dir_all(&temp_dir);
        assert_eq!(drained, 2, "both queued connections must be served");
    }

    #[cfg(unix)]
    #[test]
    fn singleton_lock_excludes_second_daemon_until_released() {
        let temp_dir =
            std::env::temp_dir().join(format!("sky-cua-singleton-test-{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).expect("create test temp dir");
        let socket_path = temp_dir.join("service.sock");

        let first = acquire_singleton_lock(&socket_path)
            .expect("first acquisition should not error")
            .expect("first acquisition should win the lock");
        let lock_path = socket_path.with_file_name("service.sock.lock");
        assert_eq!(
            std::fs::read_to_string(&lock_path)
                .expect("lock file should contain owner pid")
                .trim(),
            std::process::id().to_string()
        );
        let second =
            acquire_singleton_lock(&socket_path).expect("contended probe should not error");
        assert!(second.is_none(), "second daemon must not acquire the lock");

        drop(first);
        let third = acquire_singleton_lock(&socket_path)
            .expect("post-release acquisition should not error");
        assert!(third.is_some(), "lock must be reacquirable after release");

        drop(third);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn idle_timeout_requires_no_active_connections() {
        assert!(!idle_timed_out(IDLE_TIMEOUT + Duration::from_secs(1), 1));
        assert!(idle_timed_out(IDLE_TIMEOUT, 0));
    }

    #[tokio::test]
    async fn cursor_cleanup_keeps_cursor_visible_while_another_connection_is_active() {
        let daemon = ServiceDaemon::new_for_tests().expect("daemon");
        let connections = ConnectionTracker::default();
        let state = sky_cua_platform::model::AgentCursorState {
            visible: true,
            sequence: 0,
            model_point: None,
            native_point: Some(sky_cua_platform::model::AgentCursorPoint {
                x: 10.0,
                y: 20.0,
                coordinate_space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
                mapping_id: None,
            }),
            snapshot_id: None,
            source_action: Some(sky_cua_platform::model::ActionName::Click),
            updated_at_ms: 0,
        };
        let _ = daemon
            .handle(ServiceRequest::SetAgentCursor { state })
            .await;
        connections.register().await;
        connections.register().await;
        connections.unregister_and_cleanup_if_idle(&daemon).await;

        match daemon.handle(ServiceRequest::AgentCursorStatus).await {
            sky_cua_platform::model::ServiceResponse::AgentCursorStatus {
                state: Some(state),
                ..
            } => assert!(state.visible),
            other => panic!("unexpected response: {other:?}"),
        }

        connections.unregister_and_cleanup_if_idle(&daemon).await;
        match daemon.handle(ServiceRequest::AgentCursorStatus).await {
            sky_cua_platform::model::ServiceResponse::AgentCursorStatus {
                state: Some(state),
                ..
            } => assert!(!state.visible),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn connection_registration_waits_for_final_cleanup() {
        let connections = Arc::new(ConnectionTracker::default());
        connections.register().await;
        let cleanup_started = Arc::new(tokio::sync::Notify::new());
        let release_cleanup = Arc::new(tokio::sync::Notify::new());

        let cleanup_connections = connections.clone();
        let cleanup_started_signal = cleanup_started.clone();
        let release_cleanup_signal = release_cleanup.clone();
        let cleanup_task = tokio::spawn(async move {
            cleanup_connections
                .unregister_and_cleanup_with(|| async {
                    cleanup_started_signal.notify_one();
                    release_cleanup_signal.notified().await;
                })
                .await;
        });
        cleanup_started.notified().await;

        let register_connections = connections.clone();
        let register_completed = Arc::new(tokio::sync::Notify::new());
        let register_completed_signal = register_completed.clone();
        let register_task = tokio::spawn(async move {
            register_connections.register().await;
            register_completed_signal.notify_one();
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(25), register_completed.notified())
                .await
                .is_err(),
            "register must wait while final cleanup holds the connection lock"
        );

        release_cleanup.notify_one();
        cleanup_task.await.expect("cleanup task");
        tokio::time::timeout(Duration::from_secs(1), register_completed.notified())
            .await
            .expect("register after cleanup");
        register_task.await.expect("register task");
        assert!(!connections.is_idle().await);
    }

    #[tokio::test]
    async fn malformed_json_line_gets_error_response_and_connection_survives() {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let daemon = Arc::new(ServiceDaemon::new_for_tests().expect("daemon"));
        let handle = tokio::spawn(handle_stream(server, daemon));

        let (mut read_half, mut write_half) = tokio::io::split(client);
        let mut reader = BufReader::new(&mut read_half);

        write_half
            .write_all(b"not valid json\n")
            .await
            .expect("write malformed line");

        let mut first_response = String::new();
        reader
            .read_line(&mut first_response)
            .await
            .expect("read error response");
        match serde_json::from_str::<sky_cua_platform::model::ServiceResponse>(
            first_response.trim_end(),
        )
        .expect("parse error response")
        {
            sky_cua_platform::model::ServiceResponse::Error { code, .. } => {
                assert_eq!(code, "SKY_CUA_INVALID_REQUEST");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // The connection must still be usable after a malformed frame.
        let valid_request =
            serde_json::to_string(&ServiceRequest::Health).expect("encode health request");
        write_half
            .write_all(valid_request.as_bytes())
            .await
            .expect("write valid request");
        write_half.write_all(b"\n").await.expect("write newline");

        let mut second_response = String::new();
        reader
            .read_line(&mut second_response)
            .await
            .expect("read second response");
        serde_json::from_str::<sky_cua_platform::model::ServiceResponse>(
            second_response.trim_end(),
        )
        .expect("second response is still well-formed JSON");

        write_half
            .shutdown()
            .await
            .expect("shut down client write half");
        handle
            .await
            .expect("handle_stream task should not panic")
            .expect("handle_stream should return Ok after client closes");
    }

    #[tokio::test]
    async fn oversized_line_gets_error_response_and_connection_closes() {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let daemon = Arc::new(ServiceDaemon::new_for_tests().expect("daemon"));
        let handle = tokio::spawn(handle_stream(server, daemon));

        let (mut read_half, mut write_half) = tokio::io::split(client);
        let writer_task = tokio::spawn(async move {
            // Send more than MAX_IPC_LINE_BYTES without ever sending a
            // newline, so the reader hits the cap mid-frame.
            let chunk = vec![b'a'; 1024 * 1024];
            let mut sent: u64 = 0;
            while sent <= MAX_IPC_LINE_BYTES {
                if write_half.write_all(&chunk).await.is_err() {
                    break;
                }
                sent += chunk.len() as u64;
            }
        });

        let mut reader = BufReader::new(&mut read_half);
        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .await
            .expect("read oversized-line error response");
        match serde_json::from_str::<sky_cua_platform::model::ServiceResponse>(
            response_line.trim_end(),
        )
        .expect("parse error response")
        {
            sky_cua_platform::model::ServiceResponse::Error { code, .. } => {
                assert_eq!(code, "SKY_CUA_FRAME_TOO_LARGE");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        handle
            .await
            .expect("handle_stream task should not panic")
            .expect("handle_stream should close cleanly after an oversized frame");
        let _ = writer_task.await;
    }
}
