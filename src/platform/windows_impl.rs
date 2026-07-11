//! Windows backend.
//!
//! Volume is controlled via the Core Audio "IAudioEndpointVolume" interface
//! on the default render (playback) device -- this is the same interface
//! that drives the OS master-volume slider, so our fade is visible there too.
//!
//! Media keys are simulated with `SendInput`, using the standard
//! VK_MEDIA_PLAY_PAUSE / VK_MEDIA_NEXT_TRACK virtual keys. Any app that
//! listens for the hardware play/pause/next keys (Spotify, iTunes/Music,
//! the browser, etc.) will receive these exactly as if a keyboard sent them.

use std::cell::RefCell;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE,
};

/// Holds the live COM interface plus whether *we* are the ones who need to
/// uninitialize COM on this thread when it exits.
struct ComVolume {
    endpoint_volume: IAudioEndpointVolume,
    we_initialized_com: bool,
}

impl Drop for ComVolume {
    fn drop(&mut self) {
        if self.we_initialized_com {
            unsafe { CoUninitialize() };
        }
    }
}

// One lazily-created COM connection per OS thread. We always run the fade
// sequence on a single dedicated worker thread (spawned from the UI), so in
// practice this initializes once per button click and is torn down cleanly
// when that thread finishes.
thread_local! {
    static VOLUME: RefCell<Option<ComVolume>> = RefCell::new(None);
}

fn init_endpoint() -> windows::core::Result<ComVolume> {
    unsafe {
        // If this thread already has COM initialized in a different
        // concurrency model (e.g. by a GUI toolkit), CoInitializeEx returns
        // RPC_E_CHANGED_MODE. That's not fatal -- COM is still usable on
        // this thread, we just shouldn't call CoUninitialize ourselves.
        let we_initialized_com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let endpoint_volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;

        Ok(ComVolume {
            endpoint_volume,
            we_initialized_com,
        })
    }
}

fn with_endpoint<T>(f: impl FnOnce(&IAudioEndpointVolume) -> windows::core::Result<T>) -> Option<T> {
    VOLUME.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            match init_endpoint() {
                Ok(v) => *slot = Some(v),
                Err(_) => return None,
            }
        }
        slot.as_ref().and_then(|v| f(&v.endpoint_volume).ok())
    })
}

/// Current system (master, default playback device) volume, 0.0 ..= 1.0.
pub fn get_volume() -> f32 {
    with_endpoint(|ep| unsafe { ep.GetMasterVolumeLevelScalar() }).unwrap_or(0.5)
}

/// Set the system volume, 0.0 ..= 1.0.
pub fn set_volume(v: f32) {
    let v = v.clamp(0.0, 1.0);
    with_endpoint(|ep| unsafe { ep.SetMasterVolumeLevelScalar(v, std::ptr::null()) });
}

fn send_vk(vk: VIRTUAL_KEY) {
    unsafe {
        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        SendInput(&mut [down], std::mem::size_of::<INPUT>() as i32);
        SendInput(&mut [up], std::mem::size_of::<INPUT>() as i32);
    }
}

/// Simulate the hardware "Play/Pause" multimedia key.
pub fn send_pause() {
    send_vk(VK_MEDIA_PLAY_PAUSE);
}

/// Simulate the hardware "Next Track" multimedia key.
pub fn send_next() {
    send_vk(VK_MEDIA_NEXT_TRACK);
}
