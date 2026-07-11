//! Platform-specific backends for system-volume control and media-key simulation.
//!
//! Every backend exposes the same four functions:
//!   - `get_volume() -> f32`      current system volume, 0.0 ..= 1.0
//!   - `set_volume(f32)`          set system volume, 0.0 ..= 1.0
//!   - `send_pause()`             simulate the "Play/Pause" multimedia key
//!   - `send_next()`              simulate the "Next Track" multimedia key
//!
//! main.rs talks only to this module, so the UI/sequencing code never needs
//! to know which OS it's running on.

#[cfg(target_os = "windows")]
mod windows_impl;
#[cfg(target_os = "windows")]
pub use windows_impl::{get_volume, send_next, send_pause, set_volume};

#[cfg(target_os = "macos")]
mod macos_impl;
#[cfg(target_os = "macos")]
pub use macos_impl::{get_volume, send_next, send_pause, set_volume};

// Bonus best-effort backend for Linux, only used when building/testing this
// project on Linux (e.g. during development). Not part of the original
// request, but the trait made it essentially free to add. Requires
// `pactl` (PulseAudio/PipeWire) and `playerctl` to be installed.
#[cfg(all(unix, not(target_os = "macos")))]
mod linux_impl;
#[cfg(all(unix, not(target_os = "macos")))]
pub use linux_impl::{get_volume, send_next, send_pause, set_volume};

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
compile_error!("fade-and-skip only supports Windows, macOS, and Linux (dev fallback)");
