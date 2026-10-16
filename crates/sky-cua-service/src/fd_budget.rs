//! File-descriptor headroom and leak evidence for the long-lived daemon.
//!
//! The daemon inherits the desktop's soft `RLIMIT_NOFILE` (commonly 1024), and
//! an fd pile-up once exhausted it within minutes, leaving no record of what
//! was open. Startup raises the soft limit to the hard limit; a periodic census
//! logs the open-fd breakdown by kind once the count is elevated, so the next
//! incident names its leak.

use std::collections::BTreeMap;
use std::time::Duration;

use tracing::{info, warn};

const CENSUS_INTERVAL: Duration = Duration::from_secs(60);
const CENSUS_WARN_THRESHOLD: usize = 512;
const CENSUS_TOP_KINDS: usize = 8;

/// Raise the soft `RLIMIT_NOFILE` to the hard limit and log the outcome.
/// Best-effort; launched programs get the inherited limit back via
/// `sky_cua_platform::fd_hygiene::restore_inherited_nofile_limit`.
pub(crate) fn raise_nofile_soft_limit() {
    match sky_cua_platform::fd_hygiene::raise_nofile_soft_limit() {
        Ok(Some((previous, raised))) => {
            info!(previous, raised, "raised RLIMIT_NOFILE soft limit");
        }
        Ok(None) => {}
        Err(error) => warn!(%error, "failed to raise RLIMIT_NOFILE soft limit"),
    }
}

/// Spawn the periodic fd census (Linux only; needs `/proc/self/fd`).
pub(crate) fn spawn_fd_census() -> Option<tokio::task::JoinHandle<()>> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    Some(tokio::spawn(async {
        let mut interval = tokio::time::interval(CENSUS_INTERVAL);
        loop {
            interval.tick().await;
            let Ok(Some((total, kinds))) =
                tokio::task::spawn_blocking(|| fd_census(CENSUS_WARN_THRESHOLD)).await
            else {
                continue;
            };
            warn!(total, kinds = %format_kinds(&kinds), "elevated open file descriptor count");
        }
    }))
}

/// Count open descriptors; resolve and classify their link targets only when
/// the count reaches `threshold`, so the common healthy case costs one
/// directory listing.
fn fd_census(threshold: usize) -> Option<(usize, BTreeMap<String, usize>)> {
    let fd_dir = std::path::Path::new("/proc/self/fd");
    let total = std::fs::read_dir(fd_dir).ok()?.count();
    if total < threshold {
        return None;
    }
    let mut kinds = BTreeMap::new();
    for entry in std::fs::read_dir(fd_dir).ok()?.flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        *kinds.entry(fd_kind(&target.to_string_lossy())).or_insert(0) += 1;
    }
    Some((total, kinds))
}

/// Collapse an fd link target into a leak-identifying kind: `socket`, `pipe`,
/// `anon_inode:[eventfd]`, or the parent directory of a path.
fn fd_kind(target: &str) -> String {
    if target.starts_with("anon_inode:") || target.starts_with("/memfd:") {
        return target
            .split_once(" (deleted)")
            .map_or(target, |(head, _)| head)
            .to_owned();
    }
    if let Some((prefix, _)) = target.split_once(":[") {
        return prefix.to_owned();
    }
    match target.rsplit_once('/') {
        Some(("", _)) => "/".to_owned(),
        Some((dir, _)) => format!("{dir}/"),
        None => target.to_owned(),
    }
}

fn format_kinds(kinds: &BTreeMap<String, usize>) -> String {
    let mut ranked: Vec<_> = kinds.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    ranked
        .into_iter()
        .take(CENSUS_TOP_KINDS)
        .map(|(kind, count)| format!("{kind}={count}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fd_kind_groups_by_leak_identity() {
        assert_eq!(fd_kind("socket:[12345]"), "socket");
        assert_eq!(fd_kind("pipe:[999]"), "pipe");
        assert_eq!(fd_kind("anon_inode:[eventfd]"), "anon_inode:[eventfd]");
        assert_eq!(fd_kind("/memfd:pw-buffer (deleted)"), "/memfd:pw-buffer");
        assert_eq!(fd_kind("/dev/dri/renderD128"), "/dev/dri/");
        assert_eq!(fd_kind("/null"), "/");
    }

    #[test]
    fn format_kinds_ranks_by_count() {
        let kinds = BTreeMap::from([
            ("pipe".to_owned(), 2),
            ("socket".to_owned(), 900),
            ("/dev/".to_owned(), 2),
        ]);
        assert_eq!(format_kinds(&kinds), "socket=900 /dev/=2 pipe=2");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn census_counts_this_process_only_above_threshold() {
        assert!(fd_census(usize::MAX).is_none());
        let (total, kinds) = fd_census(0).unwrap();
        assert!(total >= 3);
        // The census's own listing fds can come and go between the passes.
        assert!(kinds.values().sum::<usize>().abs_diff(total) <= 2);
    }
}
