# Fade & Skip

A tiny always-on-top window with three buttons:

- **Fade → Pause → Next**: the original one-button flow — fades the system
  volume out, sends Play/Pause, restores the volume, then sends Next Track.
- **Fade → Pause**: fades the volume out and sends Play/Pause, then stops
  (leaving the volume at silence). Useful when you want to pause now and
  decide later whether to skip.
- **Watch current track**: starts watching the media session you're playing.
  When the current track ends and the *next* track begins, the app instantly
  mutes, pauses that new track, then restores your volume — so the next track
  is never heard. This is for when you start a song and want playback to stop
  once it finishes rather than advancing. Click the button again to stop
  watching. (Windows only — uses the System Media Transport Controls.)

The full flow (Fade → Pause → Next) is just the two smaller steps run
back-to-back.

The fade length is adjustable with the slider in the window (0-15 seconds)
and is remembered between launches.

## Building

You need a normal Rust toolchain (install via [rustup.rs](https://rustup.rs)
if you don't have one — `rustc 1.75+`, `cargo`).

```
cargo build --release
```

- **Windows** binary: `target\release\fade-and-skip.exe`
- **macOS** binary: `target/release/fade-and-skip`

This must be built *on* the target OS (Windows binaries on Windows, macOS
binaries on macOS) — there's no cross-compilation setup included, since the
volume/media-key code links against OS-specific frameworks
(Core Audio + user32 on Windows, AppKit/Quartz on macOS).

## macOS: grant Accessibility permission

Simulating the Play/Pause and Next Track keys requires macOS's Accessibility
permission (this is standard for anything that synthesizes input events, not
specific to this app). The first time it doesn't seem to work:

1. Open **System Settings → Privacy & Security → Accessibility**
2. Add the built `fade-and-skip` binary (or your terminal, if you're running
   it with `cargo run`) and enable the toggle
3. Try again

Volume fading on macOS goes through `osascript` (AppleScript), which is how
most small menu-bar utilities read/write the system volume — there's no
lightweight public C API for it the way there is on Windows.

## Windows notes

Volume is controlled via the Core Audio `IAudioEndpointVolume` interface on
the default playback device — the same one driving the system volume
slider/flyout, so you'll see the fade reflected there too. Media keys are
sent with `SendInput`, so any app listening for hardware media keys
(Spotify, the browser, Windows Media Player, etc.) will receive them
normally. No special permissions are needed.

## Customizing

- **Fade smoothness**: `FADE_STEPS` in `src/main.rs` controls how many
  discrete volume steps make up the fade (default 30).
- **Pause/restore timing**: `run_sequence` in `src/main.rs` has two
  `150ms` pauses (one after sending Pause, one after restoring volume)
  to give the target app a moment to register each action. Tune these if
  you find it too snappy or too slow for your setup.
- **Window size**: the `ViewportBuilder` in `main()` (three buttons need a
  slightly taller window than the original single button).

## Project layout

```
src/
  main.rs              window, button, and the fade/pause/restore/next sequence
  config.rs            persists the fade-length setting to disk (JSON)
  platform/
    mod.rs             picks the right backend for the current OS
    windows_impl.rs     Core Audio + SendInput
    macos_impl.rs       NSEvent/CGEventPost + osascript
    linux_impl.rs        bonus/best-effort backend (pactl + playerctl), not
                          part of the original request but free to add
```

## A note on the macOS media-key technique

There's no public, documented API to simulate the Play/Pause/Next media
keys on macOS. The approach used here — building an `NSEvent` of type
`SystemDefined` carrying an `NX_KEYTYPE_*` code, then posting its
underlying `CGEvent` — is the long-standing technique used by many
small open-source media-key utilities. It relies on undocumented event
fields, so it's possible (if unlikely) that a future macOS release
changes this.
