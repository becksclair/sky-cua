use super::*;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sky-cua-service-log-writer-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(log: &RotatingLog, bytes: &[u8]) {
    let mut handle = log.make_writer();
    handle.write_all(bytes).unwrap();
}

#[test]
fn appends_to_existing_log_without_rotation() {
    let dir = temp_dir("append");
    let path = dir.join("daemon-service.log");
    std::fs::write(&path, b"first\n").unwrap();

    let log = RotatingLog::with_cap(path.clone(), 1024);
    write(&log, b"second\n");

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
    assert!(!old_path_for(&path).exists());
}

#[test]
fn rotates_once_the_cap_is_exceeded_and_starts_fresh() {
    let dir = temp_dir("rotate");
    let path = dir.join("daemon-service.log");
    let cap = 64;

    let log = RotatingLog::with_cap(path.clone(), cap);
    // Cross the cap; the next event triggers rotation.
    write(&log, &vec![b'x'; (cap + 1) as usize]);
    write(&log, b"post-rotation\n");

    let rotated = std::fs::read(old_path_for(&path)).unwrap();
    assert_eq!(rotated.len(), (cap + 1) as usize);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "post-rotation\n");
}

#[test]
fn oversized_existing_log_rotates_at_open() {
    let dir = temp_dir("open-rotate");
    let path = dir.join("daemon-service.log");
    let cap = 32;
    let seed = vec![b'y'; (cap + 1) as usize];
    std::fs::write(&path, &seed).unwrap();

    let log = RotatingLog::with_cap(path.clone(), cap);
    write(&log, b"fresh\n");

    let rotated = std::fs::read(old_path_for(&path)).unwrap();
    assert_eq!(rotated.len(), seed.len());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh\n");
}

#[test]
fn rotation_replaces_a_previous_old_generation() {
    let dir = temp_dir("regen");
    let path = dir.join("daemon-service.log");
    let cap = 16;
    std::fs::write(old_path_for(&path), b"ancient\n").unwrap();

    let log = RotatingLog::with_cap(path.clone(), cap);
    write(&log, &vec![b'z'; (cap + 1) as usize]);
    write(&log, b"new\n");

    // The oversized current generation replaced the ancient .old.
    let rotated = std::fs::read(old_path_for(&path)).unwrap();
    assert_eq!(rotated.len(), (cap + 1) as usize);
}

#[test]
fn degraded_writer_recovers_when_the_path_becomes_writable() {
    // A transient open/write failure must not silence file logging for the
    // daemon's lifetime: each degraded event retries the open.
    let dir = temp_dir("recover");
    let path = dir.join("missing-parent").join("daemon-service.log");

    let log = RotatingLog::with_cap(path.clone(), 1024);
    write(&log, b"lost-to-stderr\n");
    assert!(!path.exists());

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    write(&log, b"recovered\n");

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "recovered\n");
}

#[test]
fn failed_reopen_after_rotation_stays_within_the_hard_cap() {
    // Regression: an EMFILE storm let rotation rename the log to `.old`
    // and then fail to reopen; the stderr fallback (fd 2 = the `.old`
    // inode) grew it without bound. A directory at the log path makes the
    // reopen fail the same way.
    let dir = temp_dir("reopen-fails");
    let path = dir.join("daemon-service.log");
    let cap = 64;
    let log = RotatingLog::with_cap(path.clone(), cap);
    write(&log, &vec![b'a'; (cap + 1) as usize]);
    std::fs::rename(&path, old_path_for(&path)).unwrap();
    std::fs::create_dir(&path).unwrap();

    let event = vec![b'b'; 10];
    for _ in 0..1000 {
        write(&log, &event);
    }

    let old_len = std::fs::metadata(old_path_for(&path)).unwrap().len();
    assert!(
        old_len <= cap * HARD_CAP_ROTATIONS + event.len() as u64,
        "rotated generation grew to {old_len}"
    );
    assert!(log.inner.lock().unwrap().dropped_events > 0);

    std::fs::remove_dir(&path).unwrap();
    log.inner.lock().unwrap().rotate_retry_at = None;
    write(&log, b"recovered\n");
    let fresh = std::fs::read_to_string(&path).unwrap();
    assert!(fresh.starts_with("{\"type\":\"sky_cua_log_events_dropped\",\"count\":"));
    assert!(fresh.ends_with("recovered\n"));
}

#[cfg(unix)]
#[test]
fn missing_path_with_failing_reopen_stays_rate_limited() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("missing-unreopenable");
    let path = dir.join("daemon-service.log");
    let log = RotatingLog::with_cap(path.clone(), 65536);
    write(&log, b"first\n");
    std::fs::rename(&path, old_path_for(&path)).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    {
        let mut guard = log.inner.lock().unwrap();
        guard.inode_last_check = Instant::now() - Duration::from_secs(11);
    }

    write(&log, b"second\n");
    let checked_at = log.inner.lock().unwrap().inode_last_check;
    write(&log, b"third\n");
    let rechecked_at = log.inner.lock().unwrap().inode_last_check;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

    assert!(checked_at.elapsed() < Duration::from_secs(10));
    assert_eq!(checked_at, rechecked_at, "check must not re-fire per event");
    assert!(!path.exists());
}

#[test]
fn failing_rename_keeps_live_log_and_notice_under_the_hard_cap() {
    let dir = temp_dir("rename-fails");
    let path = dir.join("daemon-service.log");
    let cap = 64;
    let blocker = old_path_for(&path);
    std::fs::create_dir(&blocker).unwrap();
    std::fs::write(blocker.join("keep"), b"x").unwrap();

    let log = RotatingLog::with_cap(path.clone(), cap);
    for _ in 0..(cap * 10) {
        write(&log, b"0123456789");
    }

    let len = std::fs::metadata(&path).unwrap().len();
    assert!(
        len <= cap * HARD_CAP_ROTATIONS + 10,
        "live log grew to {len}"
    );
}

#[test]
fn contended_rotation_drops_events_past_the_hard_cap() {
    let dir = temp_dir("contended-hard-cap");
    let path = dir.join("daemon-service.log");
    let cap = 64;
    let log = RotatingLog::with_cap(path.clone(), cap);
    let holder = OpenOptions::new().append(true).open(&path).unwrap();
    holder.lock().unwrap();

    for _ in 0..1000 {
        write(&log, b"0123456789");
    }
    assert!(std::fs::metadata(&path).unwrap().len() <= cap * HARD_CAP_ROTATIONS + 10);
    assert!(!old_path_for(&path).exists());

    drop(holder);
    log.inner.lock().unwrap().rotate_retry_at = None;
    write(&log, b"after\n");
    assert!(old_path_for(&path).exists());
    let fresh = std::fs::read_to_string(&path).unwrap();
    assert!(fresh.starts_with("{\"type\":\"sky_cua_log_events_dropped\""));
}

#[test]
fn failed_rotation_is_retried_at_most_once_per_interval() {
    let dir = temp_dir("rotate-throttle");
    let path = dir.join("daemon-service.log");
    let cap = 64;
    let log = RotatingLog::with_cap(path.clone(), cap);
    let holder = OpenOptions::new().append(true).open(&path).unwrap();
    holder.lock().unwrap();

    write(&log, &vec![b'a'; (cap + 1) as usize]);
    write(&log, b"b\n");
    let retry_at = log.inner.lock().unwrap().rotate_retry_at;
    assert!(
        retry_at.is_some(),
        "a contended rotation must arm the throttle"
    );

    drop(holder);
    write(&log, b"c\n");
    assert!(
        !old_path_for(&path).exists(),
        "rotation must wait out the retry interval"
    );
    assert_eq!(log.inner.lock().unwrap().rotate_retry_at, retry_at);

    log.inner.lock().unwrap().rotate_retry_at = Some(Instant::now());
    write(&log, b"d\n");
    assert!(old_path_for(&path).exists());
    assert_eq!(log.inner.lock().unwrap().rotate_retry_at, None);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "d\n");
}

#[test]
fn stderr_sharing_the_file_is_charged_against_its_hard_cap() {
    let dir = temp_dir("shared-stderr");
    let path = dir.join("daemon-service.log");
    let cap = 64;
    let hard_cap = cap * HARD_CAP_ROTATIONS;
    let log = RotatingLog::with_cap(path.clone(), cap);
    {
        let mut guard = log.inner.lock().unwrap();
        // Model production: fd 2 is this file, which is nearly full, and
        // further file writes and reopens both fail.
        guard.stderr_shares_file = true;
        guard.bytes = hard_cap - 20;
        guard.file = Some(File::open(&path).unwrap());
        guard.path = dir.join("missing-parent").join("daemon-service.log");
    }

    for _ in 0..50 {
        write(&log, b"0123456789");
    }

    let guard = log.inner.lock().unwrap();
    assert!(guard.file.is_none());
    // The fallback continues from the file's byte count (hard_cap - 20), so
    // only ~20 more bytes (2 events) reach the shared inode and the other
    // 48 are dropped. Restarting from zero would admit ~26 events.
    assert!(guard.stderr_bytes <= hard_cap + 10);
    assert!(
        guard.dropped_events >= 45,
        "only {} of 50 events dropped",
        guard.dropped_events
    );
}

#[test]
fn stderr_fallback_is_budgeted() {
    let dir = temp_dir("stderr-budget");
    let path = dir.join("missing-parent").join("daemon-service.log");
    let log = RotatingLog::with_cap(path, 16);
    for _ in 0..100 {
        write(&log, b"0123456789");
    }
    let inner = log.inner.lock().unwrap();
    assert!(inner.stderr_bytes <= 16 + 10);
    assert!(inner.dropped_events > 0);
}

#[test]
fn unwritable_path_degrades_without_panicking() {
    // A path whose parent does not exist cannot be opened; the writer must
    // degrade to stderr rather than panic or block.
    let dir = temp_dir("degrade");
    let path = dir.join("missing-parent").join("daemon-service.log");

    let log = RotatingLog::with_cap(path.clone(), 64);
    // Must not panic.
    write(&log, b"event\n");

    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn rotating_log_reopens_after_external_rename() {
    let dir = temp_dir("rename-reopen");
    let path = dir.join("daemon-service.log");
    let renamed = dir.join("daemon-service.log.renamed");

    let log = RotatingLog::with_cap(path.clone(), 65536);

    // Write 1024 times. Each write checks `inode_write_count < 1024` before
    // incrementing, so the check never fires during the loop — at the end
    // `inode_write_count` is 1024 but the threshold never triggered. Push
    // the counter to 1023 and age the wall-clock so the next write fires
    // the wall-clock check.
    for _ in 0..1024 {
        write(&log, b"x\n");
    }
    {
        let mut guard = log.inner.lock().unwrap();
        guard.inode_write_count = 1023;
        guard.inode_last_check = Instant::now() - Duration::from_secs(11);
    }

    // Verify the inode check correctly detects the stale handle
    // and reopens.
    std::fs::rename(&path, &renamed).unwrap();

    // This write triggers the inode check, detects the stale inode,
    // and reopens a fresh handle at the original path.
    write(&log, b"post-rename\n");

    let old_data = std::fs::read_to_string(&renamed).unwrap();
    assert_eq!(old_data.lines().filter(|l| *l == "x").count(), 1024);

    let new_data = std::fs::read_to_string(&path).unwrap();
    assert_eq!(new_data, "post-rename\n");
}

#[cfg(unix)]
#[test]
fn rotating_log_reopens_at_low_event_rate() {
    let dir = temp_dir("low-rate-reopen");
    let path = dir.join("daemon-service.log");
    let renamed = dir.join("daemon-service.log.renamed");

    let log = RotatingLog::with_cap(path.clone(), 65536);
    write(&log, b"first\n");

    // Push the last-check timestamp far enough back that the wall-clock
    // branch triggers on the next write regardless of the write count.
    {
        let mut guard = log.inner.lock().unwrap();
        guard.inode_last_check = Instant::now() - Duration::from_secs(11);
    }

    std::fs::rename(&path, &renamed).unwrap();
    write(&log, b"post-rename\n");

    let old_data = std::fs::read_to_string(&renamed).unwrap();
    assert_eq!(old_data, "first\n");

    let new_data = std::fs::read_to_string(&path).unwrap();
    assert_eq!(new_data, "post-rename\n");
}

#[cfg(unix)]
#[test]
fn rotating_log_revalidates_before_first_post_quiet_write() {
    let dir = temp_dir("pre-quiet");
    let path = dir.join("daemon-service.log");
    let renamed = dir.join("daemon-service.log.renamed");

    let log = RotatingLog::with_cap(path.clone(), 65536);
    write(&log, b"before\n");

    // Age last-check so the wall-clock branch fires on the next write.
    {
        let mut guard = log.inner.lock().unwrap();
        guard.inode_last_check = Instant::now() - Duration::from_secs(11);
    }

    std::fs::rename(&path, &renamed).unwrap();
    write(&log, b"after\n");

    // The "after" write should have landed in a fresh file at the original
    // path, proving the check ran before the append.
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(after, "after\n");
    assert!(
        std::fs::read_to_string(&renamed)
            .unwrap()
            .contains("before")
    );
}

#[cfg(unix)]
#[test]
fn rotating_log_handles_path_temporarily_missing() {
    let dir = temp_dir("missing-path");
    let path = dir.join("daemon-service.log");

    let log = RotatingLog::with_cap(path.clone(), 65536);
    write(&log, b"existing\n");
    assert!(path.exists());

    // Remove the file entirely (no rename — the path disappears).
    std::fs::remove_file(&path).unwrap();

    // Force an inode check on the next write.
    {
        let mut guard = log.inner.lock().unwrap();
        guard.inode_write_count = 1023;
        guard.inode_last_check = Instant::now() - Duration::from_secs(11);
    }

    // The check sees NotFound → reopens a fresh file at the path.
    write(&log, b"recreated\n");

    assert!(path.exists());
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "recreated\n");
}
