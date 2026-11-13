//! Filesystem locations shared by tests that bind Unix sockets.

use std::path::PathBuf;

/// Base directory for test Unix-socket fixtures.
///
/// Linux keeps the process temp directory. On macOS `$TMPDIR` is a per-user
/// directory about 50 bytes deep, which leaves too little of the 104-byte
/// `sun_path` for the fixtures' descriptive directory and socket names, so
/// those fixtures live under `/tmp` instead.
pub(crate) fn socket_temp_root() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    }
}
