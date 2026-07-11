//! macOS backend.
//!
//! Volume: macOS has no simple public C API for the master volume that's as
//! convenient as Core Audio's on Windows, but `osascript` (AppleScript) can
//! read and write it in one line, and it's what most little menu-bar
//! utilities do under the hood. We shell out to `osascript`.
//!
//! Media keys: play/pause and next/previous track are not ordinary keyboard
//! keys -- they're "media keys" delivered as `NSEventTypeSystemDefined`
//! events carrying an NX_KEYTYPE_* code. There's no public, documented way
//! to synthesize them, but the technique below (build an NSEvent of that
//! type via AppKit, then post its underlying CGEvent with `CGEventPost`) is
//! the long-standing, widely used approach (the same trick used by many
//! open-source media-key utilities). Because it goes through AppKit/Quartz
//! rather than a private framework, it keeps working across macOS releases,
//! but it does rely on undocumented event fields, so Apple could in
//! principle change this in a future release.
//!
//! Sending synthetic input events requires the built app to be granted
//! Accessibility permission (System Settings > Privacy & Security >
//! Accessibility). See the README for details.

use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::os::raw::c_void;
use std::process::Command;

// NX_KEYTYPE_* values from <IOKit/hidsystem/ev_keymap.h>.
const NX_KEYTYPE_PLAY: i64 = 16;
const NX_KEYTYPE_NEXT: i64 = 17;

// NSEventTypeSystemDefined.
const NS_EVENT_TYPE_SYSTEM_DEFINED: u64 = 14;
// kCGHIDEventTap -- inserts the event at the same level real hardware
// media keys arrive at. This is the tap location used by essentially
// every reference implementation of this technique.
const K_CG_HID_EVENT_TAP: u32 = 0;

#[repr(C)]
struct NsPoint {
    x: f64,
    y: f64,
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventPost(tap: u32, event: *mut c_void);
}

/// Build and post one "aux control key" event (key-down, then key-up) for
/// the given NX_KEYTYPE_* code.
unsafe fn post_aux_key(key_code: i64) {
    let cls = class!(NSEvent);
    let location = NsPoint { x: 0.0, y: 0.0 };

    // (key_state, modifierFlags) pairs: 0xa = down, 0xb = up.
    for &(key_state, modifier_flags) in &[(0xa_i64, 0xa00_u64), (0xb_i64, 0xb00_u64)] {
        let data1 = (key_code << 16) | (key_state << 8);
        let event: *mut Object = msg_send![cls,
            otherEventWithType: NS_EVENT_TYPE_SYSTEM_DEFINED
            location: location
            modifierFlags: modifier_flags
            timestamp: 0.0_f64
            windowNumber: 0_i64
            context: std::ptr::null_mut::<Object>()
            subtype: 8_i16
            data1: data1
            data2: -1_i64
        ];
        if event.is_null() {
            continue;
        }
        let cg_event: *mut c_void = msg_send![event, CGEvent];
        if !cg_event.is_null() {
            CGEventPost(K_CG_HID_EVENT_TAP, cg_event);
        }
    }
}

/// Simulate the hardware "Play/Pause" multimedia key.
pub fn send_pause() {
    unsafe { post_aux_key(NX_KEYTYPE_PLAY) };
}

/// Simulate the hardware "Next Track" multimedia key.
pub fn send_next() {
    unsafe { post_aux_key(NX_KEYTYPE_NEXT) };
}

/// Current system output volume, 0.0 ..= 1.0.
pub fn get_volume() -> f32 {
    let output = Command::new("osascript")
        .args(["-e", "output volume of (get volume settings)"])
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<f32>()
            .map(|v| (v / 100.0).clamp(0.0, 1.0))
            .unwrap_or(0.5),
        _ => 0.5,
    }
}

/// Set the system output volume, 0.0 ..= 1.0.
pub fn set_volume(v: f32) {
    let percent = (v.clamp(0.0, 1.0) * 100.0).round() as i32;
    let _ = Command::new("osascript")
        .args(["-e", &format!("set volume output volume {percent}")])
        .output();
}
