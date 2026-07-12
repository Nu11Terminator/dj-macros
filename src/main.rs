#![windows_subsystem = "windows"]

mod config;
mod platform;

use config::Config;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Number of discrete volume steps used while fading out. More steps means
/// a smoother fade at the cost of slightly more syscalls; 30 is plenty
/// smooth for fades from well under a second up to many seconds.
const FADE_STEPS: u32 = 30;

/// Which of the three button actions to run on the worker thread.
enum Action {
    /// Fade out -> pause -> restore volume -> next track (the original button).
    Full,
    /// Fade out -> pause, then stop (volume is left faded to silence).
    FadePause,
    /// Restore the pre-fade volume -> next track (the complement to FadePause).
    RestoreNext,
}

/// Status text + a busy flag, shared between the UI thread and the
/// background worker thread that runs the fade/pause/restore/next sequence.
/// `saved_volume` holds the pre-fade volume captured by the FadePause action
/// so the RestoreNext action can restore it later.
#[derive(Clone)]
struct SharedStatus {
    text: Arc<Mutex<String>>,
    busy: Arc<AtomicBool>,
    saved_volume: Arc<Mutex<Option<f32>>>,
}

impl SharedStatus {
    fn new() -> Self {
        Self {
            text: Arc::new(Mutex::new("Ready".to_string())),
            busy: Arc::new(AtomicBool::new(false)),
            saved_volume: Arc::new(Mutex::new(None)),
        }
    }

    fn set(&self, s: impl Into<String>) {
        if let Ok(mut t) = self.text.lock() {
            *t = s.into();
        }
    }

    fn get(&self) -> String {
        self.text.lock().map(|t| t.clone()).unwrap_or_default()
    }
}

struct App {
    config: Config,
    status: SharedStatus,
    /// True while the track-end watcher is active (the "Pause after"
    /// toggle). Shared with the watcher thread, which clears it when a track
    /// ends so the UI can reflect that watching has stopped.
    watching: Arc<AtomicBool>,
    /// Owns the watcher thread; dropping it stops watching. Wrapped in Arc so
    /// the toggle handler and the auto-cleanup in `update` can both reach it.
    watch_handle: Arc<Mutex<Option<platform::WatchHandle>>>,
    /// When false, only the slider, "Smooth advance", and an expand button are
    /// shown. When true, the full set of controls is shown.
    expanded: bool,
    /// Last expanded state we sized for, so we only resize on a real toggle.
    last_expanded: bool,
}

impl App {
    fn new() -> Self {
        Self {
            config: Config::load(),
            status: SharedStatus::new(),
            watching: Arc::new(AtomicBool::new(false)),
            watch_handle: Arc::new(Mutex::new(None)),
            expanded: false,
            last_expanded: false,
        }
    }

    /// Kick off a sequence on a background thread, so the UI never freezes
    /// while we sleep between volume steps. Ignored if a sequence is already
    /// running.
    fn trigger(&self, ctx: &egui::Context, action: Action) {
        if self.status.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let fade_seconds = self.config.fade_seconds;
        let status = self.status.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            run_sequence(action, fade_seconds, &status, &ctx);
            status.busy.store(false, Ordering::SeqCst);
            status.set("Ready");
            ctx.request_repaint();
        });
    }
}

/// Fade the system volume from `original` down to silence over `fade_seconds`.
fn fade_out(fade_seconds: f32, original: f32, status: &SharedStatus, ctx: &egui::Context) {
    status.set("Fading out...");
    ctx.request_repaint();
    if fade_seconds > 0.0 {
        let step_delay = Duration::from_secs_f32(fade_seconds / FADE_STEPS as f32);
        for i in (0..=FADE_STEPS).rev() {
            let fraction = i as f32 / FADE_STEPS as f32;
            platform::set_volume(original * fraction);
            thread::sleep(step_delay);
        }
    } else {
        platform::set_volume(0.0);
    }
}

/// Send the "Play/Pause" media key, with a short lead-in and follow-up delay
/// so the target app registers the key before we touch the volume again.
fn pause(status: &SharedStatus, ctx: &egui::Context) {
    status.set("Pausing playback...");
    ctx.request_repaint();
    thread::sleep(Duration::from_millis(100));
    platform::send_pause();
    thread::sleep(Duration::from_millis(250));
}

/// Restore the system volume to `original`.
fn restore(original: f32, status: &SharedStatus, ctx: &egui::Context) {
    status.set("Restoring volume...");
    ctx.request_repaint();
    platform::set_volume(original);
    thread::sleep(Duration::from_millis(150));
}

/// Send the "Next Track" media key.
fn next(status: &SharedStatus, ctx: &egui::Context) {
    status.set("Skipping to next track...");
    ctx.request_repaint();
    platform::send_next();
}

/// Run one of the three button actions.
fn run_sequence(action: Action, fade_seconds: f32, status: &SharedStatus, ctx: &egui::Context) {
    match action {
        Action::Full => {
            let original = platform::get_volume();
            fade_out(fade_seconds, original, status, ctx);
            pause(status, ctx);
            restore(original, status, ctx);
            next(status, ctx);
        }
        Action::FadePause => {
            let original = platform::get_volume();
            // Remember the pre-fade volume so RestoreNext can bring it back.
            if let Ok(mut saved) = status.saved_volume.lock() {
                *saved = Some(original);
            }
            fade_out(fade_seconds, original, status, ctx);
            pause(status, ctx);
            // Intentionally leave the volume faded to silence.
        }
        Action::RestoreNext => {
            let original = status
                .saved_volume
                .lock()
                .ok()
                .and_then(|mut saved| saved.take())
                .unwrap_or_else(|| platform::get_volume());
            restore(original, status, ctx);
            next(status, ctx);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Padding is provided by the frame below (15px on every side), so the
        // root window itself needs no extra margin.
        ctx.style_mut(|s| {
            s.spacing.window_margin = egui::Margin::ZERO;
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            // 15px of padding around all content, via the frame's inner margin.
            egui::Frame::none()
                .inner_margin(egui::Margin::same(15.0))
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                        // Fade length slider (shown in both views).
                        ui.allocate_ui(egui::vec2(220.0, 20.0), |ui| {
                            ui.horizontal_centered(|ui| {
                                ui.label("Fade length:");
                                let mut secs = self.config.fade_seconds;
                                let slider = egui::Slider::new(&mut secs, 0.0..=15.0)
                                    .suffix(" s")
                                    .fixed_decimals(1);
                                if ui.add(slider).changed() {
                                    self.config.fade_seconds = secs;
                                    self.config.save();
                                }
                            });
                        });

                        ui.add_space(12.0);

                        let busy = self.status.busy.load(Ordering::SeqCst);

                        // "Smooth advance" is always visible.
                        let sa_btn = egui::Button::new(egui::RichText::new("Smooth advance").size(15.0))
                            .min_size(egui::vec2(220.0, 32.0));
                        if ui.add_enabled(!busy, sa_btn).clicked() {
                            self.trigger(ctx, Action::Full);
                        }
                        ui.add_space(6.0);

                        if self.expanded {
                            let fade_btn = egui::Button::new(egui::RichText::new("Fade").size(15.0))
                                .min_size(egui::vec2(220.0, 32.0));
                            if ui.add_enabled(!busy, fade_btn).clicked() {
                                self.trigger(ctx, Action::FadePause);
                            }
                            ui.add_space(6.0);

                            let resume_btn = egui::Button::new(egui::RichText::new("Resume").size(15.0))
                                .min_size(egui::vec2(220.0, 32.0));
                            if ui.add_enabled(!busy, resume_btn).clicked() {
                                self.trigger(ctx, Action::RestoreNext);
                            }
                            ui.add_space(6.0);

                            // Reclaim the watcher handle if the watcher finished on its
                            // own (track ended) -- it signals that by clearing `watching`.
                            if !self.watching.load(Ordering::SeqCst) {
                                if let Ok(mut h) = self.watch_handle.lock() {
                                    if h.is_some() {
                                        *h = None;
                                    }
                                }
                            }

                            let watching_now = self.watching.load(Ordering::SeqCst);
                            let watch_label = if watching_now {
                                "Stop watching"
                            } else {
                                "Pause after"
                            };
                            let watch_btn = egui::Button::new(egui::RichText::new(watch_label).size(15.0))
                                .min_size(egui::vec2(220.0, 32.0));
                            if ui.add(watch_btn).clicked() {
                                if watching_now {
                                    // User turned it off: drop the handle to stop the thread.
                                    if let Ok(mut h) = self.watch_handle.lock() {
                                        *h = None;
                                    }
                                    self.watching.store(false, Ordering::SeqCst);
                                    self.status.set("Watch stopped");
                                } else {
                                    let status = self.status.clone();
                                    let ctx2 = ctx.clone();
                                    let watching = self.watching.clone();
                                    let handle = platform::watch_track_end(Box::new(move || {
                                        status.set("Track ended — playback stopped");
                                        watching.store(false, Ordering::SeqCst);
                                        ctx2.request_repaint();
                                    }));
                                    match handle {
                                        Some(h) => {
                                            if let Ok(mut slot) = self.watch_handle.lock() {
                                                *slot = Some(h);
                                            }
                                            self.watching.store(true, Ordering::SeqCst);
                                            self.status.set("Watching for track end...");
                                        }
                                        None => {
                                            self.status.set("Track-end watching isn't available here");
                                        }
                                    }
                                }
                                ctx.request_repaint();
                            }

                            ui.add_space(6.0);

                            // Status line -- the label that reflects the "Pause after"
                            // watcher state (and other action feedback). Wrapped to a
                            // fixed width so long text wraps instead of widening the
                            // window.
                            ui.allocate_ui(egui::vec2(220.0, 18.0), |ui| {
                                ui.label(egui::RichText::new(self.status.get()).weak());
                            });
                            ui.add_space(6.0);
                        }

                        // Expand / collapse toggle.
                        let toggle_label = if self.expanded { "Collapse ▴" } else { "Expand ▾" };
                        let toggle_btn = egui::Button::new(egui::RichText::new(toggle_label).size(13.0))
                            .min_size(egui::vec2(220.0, 28.0));
                        if ui.add(toggle_btn).clicked() {
                            self.expanded = !self.expanded;
                            ctx.request_repaint();
                        }
                    });
                });
        });

        // Size the window to its content plus 15px padding on every side. The
        // size is a pure function of the collapsed/expanded state (no measured
        // feedback), so it can never drift or grow in a loop. We only issue a
        // resize when the state actually toggles.
        let content_w = 220.0;
        let content_h = if self.expanded { 251.0 } else { 108.0 };
        let target = egui::vec2(content_w + 30.0, content_h + 30.0);
        if self.expanded != self.last_expanded {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(target));
            self.last_expanded = self.expanded;
        }

        if self.status.busy.load(Ordering::SeqCst) {
            // Keep repainting while the worker thread is running so the
            // status label updates smoothly even without external events.
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([250.0, 138.0])
            .with_min_inner_size([250.0, 138.0])
            .with_always_on_top()
            .with_resizable(false)
            .with_title("Fade & Skip"),
        ..Default::default()
    };

    eframe::run_native(
        "Fade & Skip",
        options,
        Box::new(|_cc| Box::new(App::new())),
    )
}
