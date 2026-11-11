//! macOS stale-daemon replacement: socket peer generation, exact-generation
//! termination, and rebuilt-binary staleness, exercised against real child
//! processes. The children are copies of this test binary, so their kernel
//! command name is whatever file name the copy is given.

use std::os::unix::fs::MetadataExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;

use sky_cua_platform::process::darwin;

use super::*;

const SLEEPER_ENV: &str = "SKY_CUA_IDENTITY_SLEEPER";
const SLEEPER_TEST: &str =
    "service_launcher::tests::macos_lifecycle::identity_sleeper_helper_process";

/// Helper mode: when re-executed with [`SLEEPER_ENV`], sleep until signalled.
#[test]
fn identity_sleeper_helper_process() {
    if std::env::var_os(SLEEPER_ENV).is_some() {
        thread::sleep(Duration::from_secs(30));
    }
}

/// Run a copy of this test binary named `name` in sleeper mode and return it
/// with its exact generation once the kernel reports the expected name.
fn spawn_named_sleeper(dir: &Path, name: &str) -> (Child, darwin::ProcessGeneration) {
    let copy = dir.join(name);
    fs::copy(std::env::current_exe().expect("current test exe"), &copy).expect("copy test binary");
    let child = Command::new(&copy)
        .args(["--exact", SLEEPER_TEST])
        .env(SLEEPER_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn named sleeper");
    let deadline = Instant::now() + Duration::from_secs(5);
    let generation = loop {
        if let Some(info) = darwin::process_info(child.id())
            && name.starts_with(&info.comm)
        {
            break Some(info.generation);
        }
        if Instant::now() >= deadline {
            break None;
        }
        thread::sleep(Duration::from_millis(10));
    };
    match generation {
        Some(generation) => (child, generation),
        None => {
            stop(child);
            panic!("sleeper never exec'd as {name}");
        }
    }
}

fn write_lock_owner(socket_path: &Path, pid: u32) {
    fs::write(
        crate::daemon_singleton::socket_lock_path(socket_path),
        format!("{pid}\n"),
    )
    .expect("write lock owner");
}

fn stop(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn terminates_the_exact_service_generation_with_sigterm() {
    let temp_dir = test_temp_dir("mac-term");
    let socket_path = temp_dir.join("service.sock");
    let (mut child, generation) = spawn_named_sleeper(&temp_dir, "sky-cua-service");
    write_lock_owner(&socket_path, child.id());

    let outcome = ServiceEndpoint::Unix(socket_path)
        .terminate_fresh_stale_owner(PeerProcess {
            pid: child.id(),
            generation: Some(generation),
        })
        .expect("exact service generation is terminated");

    assert_eq!(outcome, StaleOwnerTermination::Signalled);
    let status = child.wait().expect("reap terminated sleeper");
    assert_eq!(status.signal(), Some(libc::SIGTERM));
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn never_signals_a_different_generation_of_the_same_pid() {
    let temp_dir = test_temp_dir("mac-recycled");
    let socket_path = temp_dir.join("service.sock");
    let (mut child, generation) = spawn_named_sleeper(&temp_dir, "sky-cua-service");
    write_lock_owner(&socket_path, child.id());
    let earlier_holder = darwin::ProcessGeneration {
        started_at_micros: generation.started_at_micros - 1,
        ..generation
    };

    let outcome = ServiceEndpoint::Unix(socket_path)
        .terminate_fresh_stale_owner(PeerProcess {
            pid: child.id(),
            generation: Some(earlier_holder),
        })
        .expect("a recycled pid is a lifecycle race, not a failure");

    assert_eq!(outcome, StaleOwnerTermination::GenerationChanged);
    assert!(
        child.try_wait().expect("inspect sleeper").is_none(),
        "a process holding a recycled pid was signalled"
    );
    stop(child);
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn refuses_to_signal_a_generation_that_is_not_the_service() {
    let temp_dir = test_temp_dir("mac-other");
    let socket_path = temp_dir.join("service.sock");
    let (mut child, generation) = spawn_named_sleeper(&temp_dir, "unrelated-helper");
    write_lock_owner(&socket_path, child.id());

    let error = ServiceEndpoint::Unix(socket_path)
        .terminate_fresh_stale_owner(PeerProcess {
            pid: child.id(),
            generation: Some(generation),
        })
        .expect_err("a non-service process must not be signalled");

    assert!(
        error.to_string().contains("no longer identifies"),
        "unexpected error: {error}"
    );
    assert!(child.try_wait().expect("inspect sleeper").is_none());
    stop(child);
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn refuses_replacement_without_an_exact_generation() {
    let temp_dir = test_temp_dir("mac-nogen");
    let socket_path = temp_dir.join("service.sock");
    let (mut child, _) = spawn_named_sleeper(&temp_dir, "sky-cua-service");
    write_lock_owner(&socket_path, child.id());

    let error = ServiceEndpoint::Unix(socket_path)
        .terminate_fresh_stale_owner(PeerProcess {
            pid: child.id(),
            generation: None,
        })
        .expect_err("a pid without its generation must not be signalled");

    assert!(
        error.to_string().contains("exact peer generation"),
        "unexpected error: {error}"
    );
    assert!(child.try_wait().expect("inspect sleeper").is_none());
    stop(child);
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn health_connection_records_the_listener_process_generation() {
    let temp_dir = test_temp_dir("mac-peer");
    let socket_path = temp_dir.join("service.sock");
    let ready = temp_dir.join("ready");
    let observed = temp_dir.join("observed");
    let server = spawn_stale_server_helper(&socket_path, &ready, &observed, usize::MAX);
    wait_for_test_path(&ready, Duration::from_secs(5));

    let stream = ServiceEndpoint::Unix(socket_path)
        .connect()
        .expect("connect to helper listener");
    let peer = stream.peer_process().expect("peer process");

    assert_eq!(peer.pid, server.id());
    assert_eq!(peer.generation, darwin::process_generation(server.id()));
    assert_ne!(peer.pid, std::process::id());
    drop(stream);
    stop(server);
    let _ = fs::remove_dir_all(temp_dir);
}

/// Set `path`'s modification time to `modified`; this also stamps a fresh
/// inode change time, as cargo's no-relink re-clone of a binary does.
fn set_modified(path: &Path, modified: std::time::SystemTime) {
    fs::File::options()
        .write(true)
        .open(path)
        .expect("open binary")
        .set_modified(modified)
        .expect("set binary modification time");
}

#[test]
fn a_service_binary_modified_after_daemon_start_marks_it_stale() {
    let temp_dir = test_temp_dir("mac-rebuilt");
    let binary = temp_dir.join("built-before");
    fs::write(&binary, b"old build").expect("write binary built before start");
    let (daemon, generation) = spawn_named_sleeper(&temp_dir, "sky-cua-service");
    assert_eq!(
        service_binary_rebuilt_detail(&binary, &generation),
        None,
        "a binary older than the daemon is current"
    );

    let rebuilt = temp_dir.join("built-after");
    fs::write(&rebuilt, b"new build").expect("write binary rebuilt after start");
    let detail = service_binary_rebuilt_detail(&rebuilt, &generation)
        .expect("a binary modified after daemon start is stale");
    assert!(detail.contains(&format!("daemon pid {}", generation.pid)));

    assert_eq!(
        service_binary_rebuilt_detail(&temp_dir.join("missing"), &generation),
        None,
        "an uninspectable binary never marks a daemon stale"
    );
    stop(daemon);
    assert_eq!(
        service_binary_rebuilt_detail(&rebuilt, &generation),
        None,
        "an exited generation is not stale, it is gone"
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn a_binary_only_recloned_after_daemon_start_is_not_stale() {
    let temp_dir = test_temp_dir("mac-recloned");
    let original = temp_dir.join("built-before");
    fs::write(&original, b"same build").expect("write binary built before start");
    let built_at = fs::metadata(&original)
        .and_then(|metadata| metadata.modified())
        .expect("read build modification time");
    let (daemon, generation) = spawn_named_sleeper(&temp_dir, "sky-cua-service");

    let recloned = temp_dir.join("recloned-after");
    fs::write(&recloned, b"same build").expect("write binary recloned after start");
    set_modified(&recloned, built_at);

    let metadata = fs::metadata(&recloned).expect("inspect recloned binary");
    let changed_at_micros = u64::try_from(metadata.ctime()).expect("ctime after epoch") * 1_000_000
        + u64::try_from(metadata.ctime_nsec()).expect("ctime nsec") / 1_000;
    assert!(
        changed_at_micros > generation.started_at_micros,
        "fixture: the inode change time must follow the daemon start"
    );
    assert_eq!(
        service_binary_rebuilt_detail(&recloned, &generation),
        None,
        "a new inode change time with an older modification time is not a rebuild"
    );
    stop(daemon);
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn rebuilt_binary_never_marks_a_non_service_peer_stale() {
    let temp_dir = test_temp_dir("mac-rebuilt-other");
    let (other, generation) = spawn_named_sleeper(&temp_dir, "unrelated-helper");
    let rebuilt = temp_dir.join("built-after");
    fs::write(&rebuilt, b"new build").expect("write binary rebuilt after start");

    assert_eq!(service_binary_rebuilt_detail(&rebuilt, &generation), None);
    stop(other);
    let _ = fs::remove_dir_all(temp_dir);
}
