//! Native macOS desktop backend for the shared Computer Use contract.
//!
//! Pure modules (`coords`, `keymap`, `types`, `window_match`) build on every
//! target so their unit tests run in any workspace gate; everything that
//! touches macOS frameworks is gated on `target_os = "macos"`.

pub mod coords;
pub mod keymap;
pub mod types;
pub mod window_match;

#[cfg(target_os = "macos")]
mod ax;
#[cfg(target_os = "macos")]
mod backend;
#[cfg(target_os = "macos")]
mod capture;
#[cfg(target_os = "macos")]
mod displays;
#[cfg(target_os = "macos")]
mod input;
#[cfg(target_os = "macos")]
mod permissions;
#[cfg(target_os = "macos")]
mod windows;

#[cfg(target_os = "macos")]
pub use backend::MacosDesktopBackend;
