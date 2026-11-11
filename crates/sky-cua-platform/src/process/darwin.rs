//! Darwin process-identity primitives for lifecycle decisions that must never
//! signal the wrong process.
//!
//! Darwin has no `/proc`, `SO_PEERCRED`, or pidfd. The native equivalents are
//! `getsockopt(SOL_LOCAL, LOCAL_PEERPID)` for the pid behind a Unix socket,
//! `proc_pidinfo(PROC_PIDTBSDINFO)` for the kernel-recorded start time and
//! command name, and `proc_pidpath` for the executable path. A pid plus its
//! start time is a [`ProcessGeneration`]: a recycled pid always belongs to a
//! later process with a different start time, so the pair names exactly one
//! process for the life of the boot.

use std::ffi::{CStr, OsString};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// One exact process generation: a pid and the kernel start time of the
/// process currently (or formerly) holding it, at microsecond resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessGeneration {
    pub pid: u32,
    /// Start time as microseconds since the Unix epoch.
    pub started_at_micros: u64,
}

impl ProcessGeneration {
    /// The start time as a wall-clock instant, for comparison with file times.
    #[must_use]
    pub fn started_at(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_micros(self.started_at_micros)
    }
}

/// Kernel BSD information for one live process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub generation: ProcessGeneration,
    /// Effective uid.
    pub uid: u32,
    /// `p_comm`: the executable file name at exec, truncated to 16 bytes.
    pub comm: String,
    /// `p_name`: the untruncated (32-byte) process name.
    pub name: String,
}

/// Read the BSD info of `pid`, or `None` when the pid does not name a live
/// process this caller may inspect.
#[must_use]
pub fn process_info(pid: u32) -> Option<ProcessInfo> {
    let raw_pid = libc::c_int::try_from(pid).ok().filter(|pid| *pid > 0)?;
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: the kernel writes at most `size` bytes into the buffer.
    let written = unsafe {
        libc::proc_pidinfo(
            raw_pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // SAFETY: zero-initialised plain-old-data fully written by the kernel.
    let info = unsafe { info.assume_init() };
    if info.pbi_pid != pid {
        return None;
    }
    Some(ProcessInfo {
        generation: ProcessGeneration {
            pid,
            started_at_micros: start_time_micros(info.pbi_start_tvsec, info.pbi_start_tvusec),
        },
        uid: info.pbi_uid,
        comm: c_chars_to_string(&info.pbi_comm),
        name: c_chars_to_string(&info.pbi_name),
    })
}

/// The current generation of `pid`, or `None` when no such process exists.
#[must_use]
pub fn process_generation(pid: u32) -> Option<ProcessGeneration> {
    process_info(pid).map(|info| info.generation)
}

/// The executable path the kernel records for `pid`.
#[must_use]
pub fn process_executable_path(pid: u32) -> Option<PathBuf> {
    let raw_pid = libc::c_int::try_from(pid).ok().filter(|pid| *pid > 0)?;
    let mut buffer = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the kernel writes at most `buffer.len()` bytes.
    let written =
        unsafe { libc::proc_pidpath(raw_pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    let written = usize::try_from(written).ok().filter(|len| *len > 0)?;
    buffer.truncate(written);
    Some(PathBuf::from(OsString::from_vec(buffer)))
}

/// The pid the kernel recorded for the peer of a connected Unix stream socket
/// (`LOCAL_PEERPID`). For a client socket this is the process that owned the
/// listening socket when the connection was accepted into its backlog.
#[must_use]
pub fn local_peer_pid(socket: &impl AsRawFd) -> Option<u32> {
    let mut pid: libc::pid_t = 0;
    let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: `pid` and `length` describe a valid, correctly sized buffer.
    let result = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&raw mut pid).cast(),
            &raw mut length,
        )
    };
    if result != 0 || length as usize != std::mem::size_of::<libc::pid_t>() {
        return None;
    }
    u32::try_from(pid).ok().filter(|pid| *pid > 1)
}

fn start_time_micros(seconds: u64, micros: u64) -> u64 {
    seconds.saturating_mul(1_000_000).saturating_add(micros)
}

fn c_chars_to_string(raw: &[libc::c_char]) -> String {
    // SAFETY: c_char and u8 have identical layout.
    let bytes = unsafe { std::slice::from_raw_parts(raw.as_ptr().cast::<u8>(), raw.len()) };
    match CStr::from_bytes_until_nul(bytes) {
        Ok(name) => name.to_string_lossy().into_owned(),
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::process::{Command, Stdio};

    use super::*;

    #[test]
    fn own_process_info_is_stable_and_in_the_past() {
        let pid = std::process::id();
        let first = process_info(pid).expect("own process info");
        let second = process_info(pid).expect("own process info again");
        assert_eq!(first.generation, second.generation);
        assert_eq!(first.generation.pid, pid);
        // SAFETY: geteuid has no preconditions.
        assert_eq!(first.uid, unsafe { libc::geteuid() });
        assert!(first.generation.started_at() <= SystemTime::now());
        let exe = std::env::current_exe().expect("current exe");
        let exe_name = exe.file_name().expect("exe name").to_string_lossy();
        assert!(exe_name.starts_with(&first.comm), "comm {:?}", first.comm);
    }

    #[test]
    fn executable_path_matches_current_exe() {
        let reported = process_executable_path(std::process::id()).expect("own exe path");
        let expected = std::env::current_exe().expect("current exe");
        assert_eq!(
            reported.canonicalize().expect("canonical reported"),
            expected.canonicalize().expect("canonical expected")
        );
    }

    #[test]
    fn missing_and_invalid_pids_have_no_identity() {
        assert_eq!(process_info(0), None);
        assert_eq!(process_info(u32::MAX), None);
        assert_eq!(process_executable_path(0), None);
    }

    #[test]
    fn distinct_processes_have_distinct_generations_and_exit_clears_identity() {
        let mut child = Command::new("/bin/sleep")
            .arg("5")
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        let generation = process_generation(pid).expect("child generation");
        assert_ne!(generation, process_generation(std::process::id()).unwrap());
        assert_eq!(
            process_info(pid).expect("child info").comm,
            "sleep",
            "comm names the exec'd file"
        );
        child.kill().expect("kill child");
        child.wait().expect("reap child");
        assert_eq!(process_generation(pid), None);
    }

    #[test]
    fn local_peer_pid_names_the_socket_owner() {
        let (left, right) = UnixStream::pair().expect("socket pair");
        assert_eq!(local_peer_pid(&left), Some(std::process::id()));
        assert_eq!(local_peer_pid(&right), Some(std::process::id()));
    }

    #[test]
    fn c_chars_stop_at_the_first_nul_and_tolerate_a_full_buffer() {
        let raw: Vec<libc::c_char> = b"abc\0def"
            .iter()
            .map(|byte| *byte as libc::c_char)
            .collect();
        assert_eq!(c_chars_to_string(&raw), "abc");
        let full: Vec<libc::c_char> = b"0123456789abcdef"
            .iter()
            .map(|byte| *byte as libc::c_char)
            .collect();
        assert_eq!(c_chars_to_string(&full), "0123456789abcdef");
    }

    #[test]
    fn start_time_combines_seconds_and_microseconds() {
        assert_eq!(start_time_micros(2, 5), 2_000_005);
        assert_eq!(start_time_micros(u64::MAX, 1), u64::MAX);
    }
}
