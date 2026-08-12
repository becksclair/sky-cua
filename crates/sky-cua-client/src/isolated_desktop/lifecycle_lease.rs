//! Per-display interprocess serialization for isolated-desktop lifecycle work.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use anyhow::{Context, Result, anyhow};

use super::sky_cua_runtime_dir;

/// Lease serializing one display's complete start/reuse/stop transition.
///
/// The lock file is deliberately persistent: unlinking it could let two
/// processes lock different inodes for the same display.
pub(super) struct IsolatedDesktopLifecycleLease {
    _file: File,
}

impl IsolatedDesktopLifecycleLease {
    pub(super) fn acquire(display_number: u32) -> Result<Self> {
        let directory = sky_cua_runtime_dir().ok_or_else(|| {
            anyhow!("cannot lock isolated desktop lifecycle because XDG_RUNTIME_DIR is unset")
        })?;
        std::fs::create_dir_all(&directory).with_context(|| {
            format!(
                "failed to create isolated desktop lifecycle directory {}",
                directory.display()
            )
        })?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        let path = directory.join(format!("isolated-desktop-{display_number}.lifecycle.lock"));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| {
                format!(
                    "failed to open isolated desktop lifecycle lease {}",
                    path.display()
                )
            })?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        loop {
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result == 0 {
                return Ok(Self { _file: file });
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINTR) {
                return Err(error).with_context(|| {
                    format!(
                        "failed to acquire isolated desktop lifecycle lease {}",
                        path.display()
                    )
                });
            }
        }
    }
}
