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
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

// --- Track-end watching (SMTC) ---
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionPlaybackStatus, GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
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

// --- Track-end watching via the System Media Transport Controls (SMTC) ---
//
// SMTC is the API media apps (Spotify, Apple Music, the Movies & TV app,
// browser players that opt in, etc.) use to publish what they're playing.
// By subscribing to a session's PlaybackInfoChanged / MediaPropertiesChanged
// events we learn the instant the current track stops or a *different* track
// starts -- without polling the audio meter. When we see a new track start,
// we instantly mute, send pause, then restore the volume, so the next track
// is never heard.

/// Stops watching when dropped (signals the worker thread and joins it).
pub struct WatchHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, AtomicOrdering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Begin watching the current media session. `on_ended` is invoked (on the
/// watcher thread) the moment a *different* track starts playing than the one
/// that was playing when watching began. Returns `None` if SMTC is
/// unavailable on this machine.
pub fn watch_track_end(on_ended: Box<dyn Fn() + Send + 'static>) -> Option<WatchHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();

    let thread = thread::spawn(move || {
        let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();

        let manager = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync() {
            Ok(op) => match op.get() {
                Ok(m) => m,
                Err(_) => {
                    if initialized {
                        unsafe { RoUninitialize() };
                    }
                    return;
                }
            },
            Err(_) => {
                if initialized {
                    unsafe { RoUninitialize() };
                }
                return;
            }
        };
        let session = match manager.GetCurrentSession() {
            Ok(s) => s,
            Err(_) => {
                if initialized {
                    unsafe { RoUninitialize() };
                }
                return;
            }
        };

        // Remember what's playing now so we can detect a *different* track.
        let mut baseline = current_title(&session);

        let (tx, rx) = mpsc::channel::<()>();
        let tx_playback = tx.clone();
        let tx_media = tx.clone();

        // Handlers just ping our worker thread; all real work (reading state,
        // deciding, acting) happens here so we never issue async WinRT calls
        // from inside a WinRT callback.
        let playback_handler = TypedEventHandler::new(move |_s, _a| {
            let _ = tx_playback.send(());
            Ok(())
        });
        let _playback_token = session.PlaybackInfoChanged(Some(&playback_handler));

        let media_handler = TypedEventHandler::new(move |_s, _a| {
            let _ = tx_media.send(());
            Ok(())
        });
        let _media_token = session.MediaPropertiesChanged(Some(&media_handler));

        while !stop_thread.load(AtomicOrdering::SeqCst) {
            match rx.recv_timeout(Duration::from_millis(300)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            let status = session
                .GetPlaybackInfo()
                .ok()
                .and_then(|i| i.PlaybackStatus().ok());

            if let Some(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing) = status {
                let title = current_title(&session);
                if title.is_empty() {
                    // Nothing was playing when we started; adopt this track as
                    // the one to watch instead of reacting to it.
                    baseline = title;
                } else if title != baseline {
                    // A new track started: silence it, pause it, restore volume.
                    react_to_new_track(&session);
                    on_ended();
                    break;
                }
            }
        }

        if initialized {
            unsafe { RoUninitialize() };
        }
    });

    Some(WatchHandle {
        stop,
        thread: Some(thread),
    })
}

fn current_title(session: &GlobalSystemMediaTransportControlsSession) -> String {
    session
        .TryGetMediaPropertiesAsync()
        .ok()
        .and_then(|op| op.get().ok())
        .and_then(|props| props.Title().ok())
        .map(|h| h.to_string())
        .unwrap_or_default()
}

/// Instantly mute, pause the (new) track, then restore the user's volume so
/// the paused track stays inaudible but the slider returns to normal.
fn react_to_new_track(session: &GlobalSystemMediaTransportControlsSession) {
    let original = get_volume();
    set_volume(0.0);

    let paused = match session.TryPauseAsync() {
        Ok(op) => op.get().unwrap_or(false),
        Err(_) => false,
    };

    let paused = if paused {
        true
    } else {
        send_pause();
        // Give the media key a moment, then verify it actually paused.
        thread::sleep(Duration::from_millis(250));
        session
            .GetPlaybackInfo()
            .ok()
            .and_then(|i| i.PlaybackStatus().ok())
            .map(|s| {
                s == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused
                    || s == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Stopped
            })
            .unwrap_or(false)
    };

    // Only restore the volume if we actually stopped playback; otherwise leave
    // it muted so nothing blasts out.
    set_volume(if paused { original } else { 0.0 });
}
