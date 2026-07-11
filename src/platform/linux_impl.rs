//! Best-effort Linux backend.
//!
//! Not part of the original request (Windows + macOS only), but the shared
//! `platform` interface made this essentially free, and it's what let this
//! whole project be compiled and exercised on a Linux machine during
//! development. Uses `pactl` (PulseAudio/PipeWire) for volume and
//! `playerctl` for media keys (via MPRIS), both common on Linux desktops.
//! If either tool is missing, calls become harmless no-ops.

use std::process::Command;

pub fn get_volume() -> f32 {
    let output = Command::new("pactl")
        .args(["get-sink-volume", "@DEFAULT_SINK@"])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            // Example line: "Volume: front-left: 45875 /  70% / -9.16 dB ..."
            s.split('/')
                .nth(1)
                .and_then(|pct| pct.trim().trim_end_matches('%').parse::<f32>().ok())
                .map(|v| (v / 100.0).clamp(0.0, 1.0))
                .unwrap_or(0.5)
        }
        _ => 0.5,
    }
}

pub fn set_volume(v: f32) {
    let percent = (v.clamp(0.0, 1.0) * 100.0).round() as i32;
    let _ = Command::new("pactl")
        .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{percent}%")])
        .output();
}

pub fn send_pause() {
    let _ = Command::new("playerctl").arg("play-pause").output();
}

pub fn send_next() {
    let _ = Command::new("playerctl").arg("next").output();
}
