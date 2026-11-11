//! Per-process facts for window owners: executable, bundle identifier, and
//! the front-most process.
//!
//! Nothing here reads AppKit state that is refreshed by the main run loop
//! (`NSWorkspace.runningApplications`, `NSWorkspace.frontmostApplication`,
//! `NSRunningApplication.isActive`): the daemon has no running main run loop,
//! so those values freeze at their first read. Executables come from the
//! kernel (`proc_pidpath`), bundle identifiers from the bundle on disk, and
//! the front process from a synchronous Launch Services query.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use objc2_core_foundation::{CFBundle, CFURL};

/// Executable path and bundle identifier of one process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ProcessInfo {
    pub executable: Option<String>,
    pub bundle_id: Option<String>,
}

/// Looks up `pid`'s executable and bundle identifier. Bundle identifiers are
/// cached per executable path, so a steady-state call costs one syscall.
pub(super) fn process_info(pid: i32) -> ProcessInfo {
    let Some(executable) = executable_path(pid) else {
        return ProcessInfo::default();
    };
    let bundle_id = cached_bundle_id(&executable);
    ProcessInfo {
        executable: Some(executable),
        bundle_id,
    }
}

fn executable_path(pid: i32) -> Option<String> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `buffer` is writable for the length passed.
    let len = unsafe {
        libc::proc_pidpath(
            pid,
            buffer.as_mut_ptr().cast(),
            u32::try_from(buffer.len()).ok()?,
        )
    };
    let len = usize::try_from(len).ok().filter(|&len| len > 0)?;
    buffer.truncate(len);
    String::from_utf8(buffer).ok()
}

fn cached_bundle_id(executable: &str) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(cached) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(executable)
    {
        return cached.clone();
    }
    let bundle_id = bundle_path_for_executable(executable).and_then(read_bundle_id);
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(executable.to_owned(), bundle_id.clone());
    bundle_id
}

/// The bundle directory that contains `executable` in the standard
/// `<bundle>/Contents/MacOS/<name>` layout, or `None` for a bare executable.
pub(super) fn bundle_path_for_executable(executable: &str) -> Option<&Path> {
    let macos = Path::new(executable).parent()?;
    let contents = macos.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents")
        .then(|| contents.parent())
        .flatten()
}

fn read_bundle_id(bundle: &Path) -> Option<String> {
    let url = CFURL::from_directory_path(bundle)?;
    let bundle = CFBundle::new(None, Some(&url))?;
    bundle
        .identifier()
        .map(|identifier| identifier.to_string())
        .filter(|identifier| !identifier.is_empty())
}

/// The pid of the front-most (active) application, as Launch Services
/// currently reports it.
///
/// `GetFrontProcess` is a synchronous query of the Launch Services front
/// application, the same state that drives the menu bar. Unlike
/// `NSWorkspace.frontmostApplication` it is not a cached property updated by
/// run-loop notifications, so it stays current in a process that never runs
/// its main run loop. It needs no TCC grant.
pub(super) fn front_process_pid() -> Option<i32> {
    let mut psn = ProcessSerialNumber {
        high_long_of_psn: 0,
        low_long_of_psn: 0,
    };
    let mut pid: libc::pid_t = 0;
    // SAFETY: both out-pointers are valid for writes; the calls only fill them.
    let ok = unsafe { GetFrontProcess(&mut psn) == 0 && GetProcessPID(&psn, &mut pid) == 0 };
    (ok && pid > 0).then_some(pid)
}

#[repr(C)]
struct ProcessSerialNumber {
    high_long_of_psn: u32,
    low_long_of_psn: u32,
}

// Process Manager calls are deprecated in the SDK headers in favour of
// `NSWorkspace`, whose replacements depend on a running main run loop (see
// the module documentation). They remain exported by HIServices.
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn GetFrontProcess(psn: *mut ProcessSerialNumber) -> i16;
    fn GetProcessPID(psn: *const ProcessSerialNumber, pid: *mut libc::pid_t) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_path_follows_the_standard_layout() {
        assert_eq!(
            bundle_path_for_executable("/Applications/Safari.app/Contents/MacOS/Safari"),
            Some(Path::new("/Applications/Safari.app"))
        );
        assert_eq!(
            bundle_path_for_executable(
                "/Applications/Foo.app/Contents/Frameworks/Foo Helper.app/Contents/MacOS/Foo Helper"
            ),
            Some(Path::new(
                "/Applications/Foo.app/Contents/Frameworks/Foo Helper.app"
            ))
        );
    }

    #[test]
    fn bare_executables_have_no_bundle() {
        assert_eq!(bundle_path_for_executable("/usr/bin/python3"), None);
        assert_eq!(bundle_path_for_executable("/Contents/MacOS"), None);
        assert_eq!(bundle_path_for_executable("tool"), None);
    }
}
