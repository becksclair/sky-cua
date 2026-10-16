use super::*;

#[tokio::test]
async fn maintenance_tick_fires_despite_continuous_accept_errors() {
    // Model the accept loop: another arm is ready far more often than the
    // housekeeping period. A per-iteration sleep would never fire here.
    let mut maintenance = maintenance_interval_every(Duration::from_millis(20));
    // Generous deadline so a loaded machine cannot flake the test; a
    // per-iteration sleep never ticks and fails only by hitting it.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut ticks = 0;
    while ticks < 3 && tokio::time::Instant::now() < deadline {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(2)) => {}
            _ = maintenance.tick() => ticks += 1,
        }
    }
    assert_eq!(
        ticks, 3,
        "maintenance ticked {ticks} times before the deadline"
    );
}

#[cfg(unix)]
#[test]
fn strict_mode_requires_codex_ingress_while_hybrid_can_report_degraded() {
    assert!(codex_ingress_bind_failure_is_fatal(
        crate::browser::BrowserControlMode::Strict
    ));
    assert!(!codex_ingress_bind_failure_is_fatal(
        crate::browser::BrowserControlMode::Hybrid
    ));
    assert!(!codex_ingress_bind_failure_is_fatal(
        crate::browser::BrowserControlMode::Legacy
    ));
}

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
    let temp_dir = std::env::temp_dir().join(format!("sky-cua-drain-test-{}", std::process::id()));
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
    let second = acquire_singleton_lock(&socket_path).expect("contended probe should not error");
    assert!(second.is_none(), "second daemon must not acquire the lock");

    drop(first);
    let third =
        acquire_singleton_lock(&socket_path).expect("post-release acquisition should not error");
    assert!(third.is_some(), "lock must be reacquirable after release");

    drop(third);
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn idle_timeout_requires_no_active_connections() {
    assert!(!idle_timed_out(
        IDLE_TIMEOUT + Duration::from_secs(1),
        1,
        false
    ));
    assert!(idle_timed_out(IDLE_TIMEOUT, 0, false));
}

#[test]
fn idle_timeout_is_disabled_while_direct_listener_is_active() {
    // The third argument is captured runtime presence, not a fresh config
    // lookup; later config edits or parse failures cannot change this.
    assert!(!idle_timed_out(
        IDLE_TIMEOUT + Duration::from_secs(1),
        0,
        true
    ));
    assert!(idle_timed_out(IDLE_TIMEOUT, 0, false));
}

#[tokio::test]
async fn connection_tracking_returns_to_idle_without_cleanup_io() {
    let connections = ConnectionTracker::default();
    connections.register().await;
    connections.register().await;
    connections.unregister().await;
    assert!(!connections.is_idle().await);
    connections.unregister().await;
    assert!(connections.is_idle().await);
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
    let valid_request = serde_json::to_string(&ServiceRequest::AgentCursorStatus)
        .expect("encode agent cursor status request");
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
    serde_json::from_str::<sky_cua_platform::model::ServiceResponse>(second_response.trim_end())
        .expect("second response is still well-formed JSON");

    write_half
        .shutdown()
        .await
        .expect("shut down client write half");
    drop(write_half);
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
    match serde_json::from_str::<sky_cua_platform::model::ServiceResponse>(response_line.trim_end())
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
