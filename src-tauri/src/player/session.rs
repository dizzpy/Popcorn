//! One file being played: its mpv core, its surface, its progress, and the
//! thread that turns mpv events into UI events and database writes.
//!
//! Threading rules that keep shutdown free of deadlocks:
//! * nothing ever joins the event thread, and the main thread never waits for
//!   it;
//! * `tracker` is locked before the database, never the other way round;
//! * `events` is never waited for on the main thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::embed::{VideoSurface, WakeLock};
use super::model::{self, PlaybackState, Throttle, Tracks};
use super::mpv::{Event, EventReader, Mpv};
use super::now_playing;
use super::store;
use super::tracker::{Save, Tracker};
use super::PlayerHost;
use crate::jobs;
use crate::state::{lock, unix_now, AppState};

const TIME_EVERY: Duration = Duration::from_millis(250);
const WAIT_SECS: f64 = 0.2;
/// An end of file before this share of the duration means the file broke off
/// (a pulled drive, a partial copy), not that it was watched.
const REAL_END_RATIO: f64 = 0.9;

const OBSERVED: &[&str] = &[
    "time-pos",
    "duration",
    "pause",
    "paused-for-cache",
    "eof-reached",
    "volume",
    "mute",
    "track-list",
    "aid",
    "sid",
    "video-params",
];

pub struct Session {
    pub mpv: Arc<Mpv>,
    pub file_id: i64,
    pub path: String,
    tracker: Mutex<Tracker>,
    surface: Mutex<Option<VideoSurface>>,
    awake: Mutex<Option<WakeLock>>,
    /// Held while emitting. Once `stopped` is set under it, this session can
    /// never send another event, so nothing of an old file reaches the UI
    /// after the next file's events.
    events: Mutex<()>,
    stopped: AtomicBool,
}

#[derive(Clone, Copy, PartialEq, Serialize)]
struct StatePayload {
    state: PlaybackState,
    volume: f64,
    muted: bool,
}

#[derive(Clone, Serialize)]
struct TimePayload {
    position: f64,
    duration: Option<f64>,
}

#[derive(Clone, Serialize)]
struct FilePayload {
    file_id: i64,
}

#[derive(Clone, Serialize)]
struct ErrorPayload {
    message: String,
}

fn emit_now<T: Serialize + Clone>(app: &AppHandle, event: &str, payload: T) {
    if let Err(error) = app.emit(event, payload) {
        log::warn!("could not emit {event}: {error}");
    }
}

/// Told to the UI when playback closes from the native side.
pub fn emit_closed(app: &AppHandle, file_id: i64) {
    emit_now(app, "player://closed", FilePayload { file_id });
}

impl Session {
    pub fn new(mpv: Arc<Mpv>, file_id: i64, path: String) -> Self {
        Self {
            mpv,
            file_id,
            path,
            tracker: Mutex::new(Tracker::new(file_id)),
            surface: Mutex::new(None),
            awake: Mutex::new(None),
            events: Mutex::new(()),
            stopped: AtomicBool::new(false),
        }
    }

    pub fn set_surface(&self, surface: VideoSurface) {
        *lock(&self.surface) = Some(surface);
    }

    /// A handle to the surface, if the session still has one. The lock is not
    /// kept, because using a surface can mean waiting for the main thread.
    pub fn surface(&self) -> Option<VideoSurface> {
        lock(&self.surface).clone()
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Starts reading mpv's events. Call before loading the file.
    pub fn watch(self: &Arc<Self>, app: &AppHandle) -> crate::error::AppResult<()> {
        let reader = self
            .mpv
            .event_reader()
            .ok_or_else(|| crate::error::AppError::Player("already watching".into()))?;
        for name in OBSERVED {
            self.mpv.observe(name)?;
        }
        let (app, session) = (app.clone(), Arc::clone(self));
        thread::spawn(move || EventLoop::new(app, session).run(reader));
        Ok(())
    }

    fn emit<T: Serialize + Clone>(&self, app: &AppHandle, event: &str, payload: T) {
        let _events = lock(&self.events);
        if !self.is_stopped() {
            emit_now(app, event, payload);
        }
    }

    /// Marks the session stopped. False if it already was.
    fn stop(&self, on_main_thread: bool) -> bool {
        // The main thread only does this while the window is closing, when
        // event order no longer matters, and it must never wait for a thread
        // that might be waiting for it.
        let _events = (!on_main_thread).then(|| lock(&self.events));
        !self.stopped.swap(true, Ordering::SeqCst)
    }

    /// Reads the position straight from mpv, for the moments that matter.
    fn refresh_position(&self, tracker: &mut Tracker) {
        if let Some(position) = self.mpv.get_f64("time-pos") {
            tracker.set_position(position);
        }
    }

    /// Lets the tracker decide on a save, and writes it while the tracker is
    /// still locked, so saves reach the database in order.
    fn save_with(&self, app: &AppHandle, decide: impl FnOnce(&mut Tracker) -> Option<Save>) {
        let mut tracker = lock(&self.tracker);
        let Some(save) = decide(&mut tracker) else {
            return;
        };
        let state = app.state::<AppState>();
        let result = store::save(&state.db(), &save, unix_now());
        drop(tracker);
        match result {
            Ok(true) => jobs::notify_library_updated(app),
            Ok(false) => {}
            Err(error) => log::warn!("could not save progress: {error}"),
        }
    }

    /// The final save and the end of this session. Safe to call twice; only
    /// the first call does anything and returns true.
    pub fn close(&self, app: &AppHandle, on_main_thread: bool) -> bool {
        if !self.stop(on_main_thread) {
            return false;
        }
        self.save_with(app, |tracker| {
            if tracker.is_ready() {
                self.refresh_position(tracker);
            }
            tracker.close(Instant::now())
        });
        // Silences the audio at once; the core goes when its last user does.
        if let Err(error) = self.mpv.command_async(&["set", "pause", "yes"]) {
            log::debug!("could not pause on close: {error}");
        }
        // Bound first so that the lock is released before waiting on the main
        // thread, which may itself want this lock while quitting.
        let surface = lock(&self.surface).take();
        if let Some(surface) = surface {
            surface.close(app);
        }
        *lock(&self.awake) = None;
        self.mpv.wakeup();
        true
    }
}

struct EventLoop {
    app: AppHandle,
    session: Arc<Session>,
    time_throttle: Throttle,
    /// The position changed since it was last sent.
    time_dirty: bool,
    time_forced: bool,
    state_dirty: bool,
    sent_state: Option<StatePayload>,
    sent_tracks: Option<Tracks>,
    sent_size: Option<(i64, i64)>,
    started: bool,
    seeking: bool,
    at_end: bool,
}

impl EventLoop {
    fn new(app: AppHandle, session: Arc<Session>) -> Self {
        Self {
            app,
            session,
            time_throttle: Throttle::new(TIME_EVERY),
            time_dirty: false,
            time_forced: false,
            state_dirty: true,
            sent_state: None,
            sent_tracks: None,
            sent_size: None,
            started: false,
            seeking: false,
            at_end: false,
        }
    }

    fn run(mut self, mut reader: EventReader) {
        loop {
            let event = reader.wait(WAIT_SECS);
            if self.session.is_stopped() {
                break;
            }
            match event {
                Event::Shutdown => {
                    // mpv quit by itself, e.g. a quit key in its bindings.
                    self.app
                        .state::<PlayerHost>()
                        .close_from_native(&self.app, &self.session);
                    break;
                }
                Event::Failed(reason) => {
                    log::warn!("playback failed: {reason}");
                    let message = format!("This file could not be played ({reason}).");
                    self.session
                        .emit(&self.app, "player://error", ErrorPayload { message });
                }
                Event::FileLoaded => {
                    self.update_duration();
                    self.update_tracks();
                }
                Event::Seek => self.seeking = true,
                Event::PlaybackRestart => self.playback_restarted(),
                Event::PropertyChange(name) => self.property_changed(&name),
                Event::Idle | Event::Other => {}
            }
            self.tick();
        }
    }

    fn playback_restarted(&mut self) {
        let after_seek = self.seeking && self.started;
        self.seeking = false;
        self.started = true;
        self.update_duration();
        let session = &self.session;
        session.save_with(&self.app, |tracker| {
            tracker.start(Instant::now());
            session.refresh_position(tracker);
            // The first start is not saved: nothing has been watched yet.
            after_seek
                .then(|| tracker.checkpoint(Instant::now()))
                .flatten()
        });
        self.time_dirty = true;
        self.time_forced = true;
        self.state_dirty = true;
    }

    fn property_changed(&mut self, name: &str) {
        match name {
            "time-pos" => {
                let mut tracker = lock(&self.session.tracker);
                if !tracker.is_closed() {
                    self.session.refresh_position(&mut tracker);
                    self.time_dirty = true;
                }
            }
            "duration" => self.update_duration(),
            "pause" => {
                if self.session.mpv.get_flag("pause") {
                    let session = &self.session;
                    session.save_with(&self.app, |tracker| {
                        if tracker.is_ready() {
                            session.refresh_position(tracker);
                        }
                        tracker.checkpoint(Instant::now())
                    });
                    self.time_dirty = true;
                    self.time_forced = true;
                }
                self.state_dirty = true;
            }
            "eof-reached" => self.end_changed(),
            "paused-for-cache" | "volume" | "mute" => self.state_dirty = true,
            "track-list" | "aid" | "sid" => self.update_tracks(),
            "video-params" => self.update_size(),
            _ => {}
        }
    }

    fn update_duration(&self) {
        if let Some(duration) = self.session.mpv.get_f64("duration") {
            lock(&self.session.tracker).set_duration(duration);
        }
    }

    fn end_changed(&mut self) {
        let at_end = self.session.mpv.get_flag("eof-reached");
        if at_end == self.at_end {
            return;
        }
        self.at_end = at_end;
        self.state_dirty = true;
        let mut tracker = lock(&self.session.tracker);
        if !at_end {
            tracker.set_at_end(false);
            return;
        }
        if !tracker.is_ready() {
            return;
        }
        self.session.refresh_position(&mut tracker);
        let broke_off = matches!(
            (tracker.position(), tracker.duration()),
            (Some(position), Some(duration)) if position < duration * REAL_END_RATIO
        );
        if broke_off {
            drop(tracker);
            let message = "Playback stopped before the end of the file. \
                           Is the drive still connected?"
                .to_owned();
            self.session
                .emit(&self.app, "player://error", ErrorPayload { message });
            return;
        }
        if !tracker.set_at_end(true) {
            return;
        }
        let file_id = tracker.file_id();
        let state = self.app.state::<AppState>();
        let result = store::finish(&state.db(), file_id, tracker.duration(), unix_now());
        drop(tracker);
        match result {
            Ok(()) => jobs::notify_library_updated(&self.app),
            Err(error) => log::warn!("could not mark as watched: {error}"),
        }
        self.session
            .emit(&self.app, "player://ended", FilePayload { file_id });
    }

    fn update_tracks(&mut self) {
        let Some(json) = self.session.mpv.get_string("track-list") else {
            return;
        };
        match model::parse_tracks(&json) {
            Ok(tracks) => {
                // An empty list only means the file is not open yet.
                if tracks != Tracks::default() && self.sent_tracks.as_ref() != Some(&tracks) {
                    self.session
                        .emit(&self.app, "player://tracks", tracks.clone());
                    self.sent_tracks = Some(tracks);
                }
            }
            Err(error) => log::warn!("unreadable track list: {error}"),
        }
    }

    fn update_size(&mut self) {
        let side = |name| {
            self.session
                .mpv
                .get_string(name)
                .and_then(|value| value.parse::<i64>().ok())
                .filter(|value| *value > 0)
        };
        let (Some(width), Some(height)) = (side("video-params/w"), side("video-params/h")) else {
            return;
        };
        if self.sent_size == Some((width, height)) {
            return;
        }
        self.sent_size = Some((width, height));
        let state = self.app.state::<AppState>();
        let result = store::set_dimensions(&state.db(), self.session.file_id, width, height);
        if let Err(error) = result {
            log::warn!("could not store the video size: {error}");
        }
    }

    /// Work that depends on time or on several properties at once.
    fn tick(&mut self) {
        let now = Instant::now();
        self.session
            .save_with(&self.app, |tracker| tracker.periodic(now));
        self.send_time(now);
        if std::mem::take(&mut self.state_dirty) {
            self.send_state();
        }
    }

    fn send_time(&mut self, now: Instant) {
        if !self.time_dirty {
            return;
        }
        let payload = {
            let tracker = lock(&self.session.tracker);
            tracker.position().map(|position| TimePayload {
                position,
                duration: tracker.duration(),
            })
        };
        let Some(payload) = payload else {
            return;
        };
        // A held-back update stays dirty and goes out on a later pass.
        if self.time_throttle.pass(now, self.time_forced) {
            self.time_dirty = false;
            self.time_forced = false;
            self.session.emit(&self.app, "player://time", payload);
        }
    }

    fn send_state(&mut self) {
        let mpv = &self.session.mpv;
        let paused = mpv.get_flag("pause");
        let ready = lock(&self.session.tracker).is_ready();
        let state = StatePayload {
            state: model::playback_state(ready, paused, mpv.get_flag("paused-for-cache")),
            volume: mpv.get_f64("volume").unwrap_or(100.0),
            muted: mpv.get_flag("mute"),
        };
        let playing = !paused && !self.at_end;
        self.keep_awake(playing);
        // Also after a seek, which marks the state dirty: the widget only
        // extrapolates, so every jump has to be told.
        let progress = {
            let tracker = lock(&self.session.tracker);
            tracker
                .position()
                .map(|position| (position, tracker.duration()))
        };
        if let Some((position, duration)) = progress {
            now_playing::update(self.session.file_id, position, duration, playing);
        }
        if self.sent_state != Some(state) {
            self.sent_state = Some(state);
            self.session.emit(&self.app, "player://state", state);
        }
    }

    fn keep_awake(&self, wanted: bool) {
        let mut awake = lock(&self.session.awake);
        // Checked under the lock so that a closed session never takes one.
        if wanted && !self.session.is_stopped() {
            awake.get_or_insert_with(WakeLock::new);
        } else {
            *awake = None;
        }
    }
}
