//! The built-in video player, on libmpv.
//!
//! Each file gets its own mpv core. Switching files closes the old session
//! (saving its position) before the next one exists, so a position or an
//! event can never be attributed to the wrong file.

pub mod commands;
#[cfg(target_os = "macos")]
#[path = "embed_macos.rs"]
mod embed;
#[cfg(not(target_os = "macos"))]
#[path = "embed_unsupported.rs"]
mod embed;
mod ffi;
#[cfg(target_os = "macos")]
mod gl;
mod keys;
mod model;
mod mpv;
#[cfg(target_os = "macos")]
mod now_playing;
#[cfg(not(target_os = "macos"))]
#[path = "now_playing_unsupported.rs"]
mod now_playing;
#[cfg(target_os = "macos")]
mod objc;
mod options;
mod session;
#[cfg(debug_assertions)]
pub mod smoke;
mod store;
mod tracker;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;
use tauri::{AppHandle, Manager, RunEvent, Window, WindowEvent};

use self::keys::KeyAction;
use self::mpv::Mpv;
use self::session::Session;
use crate::db::queries::{self, PlaybackItem};
use crate::error::{AppError, AppResult};
use crate::jobs;
use crate::state::{lock, AppState};

/// What the app asks of a video engine.
pub trait Player {
    /// Starts playing a local file, from `start_at` seconds if given.
    fn load(&self, path: &str, start_at: Option<f64>) -> AppResult<()>;
    fn toggle_pause(&self) -> AppResult<()>;
    fn seek(&self, seconds: f64, relative: bool) -> AppResult<()>;
    /// 0 to 100.
    fn set_volume(&self, volume: f64) -> AppResult<()>;
    fn toggle_mute(&self) -> AppResult<()>;
    fn set_audio(&self, track_id: i64) -> AppResult<()>;
    /// `None` turns subtitles off.
    fn set_subtitle(&self, track_id: Option<i64>) -> AppResult<()>;
    /// Loads a subtitle file and selects it.
    fn add_subtitle(&self, path: &str) -> AppResult<()>;
    /// Raises subtitles above controls that cover `fraction` of the height.
    fn set_subtitle_lift(&self, fraction: f64) -> AppResult<()>;
}

/// mpv's default bottom margin, in its scaled pixels (720 to the height).
const SUBTITLE_MARGIN: f64 = 22.0;
const SCALED_HEIGHT: f64 = 720.0;

fn finite(value: f64) -> AppResult<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(AppError::Player("that is not a number".into()))
    }
}

impl Player for Mpv {
    fn load(&self, path: &str, start_at: Option<f64>) -> AppResult<()> {
        if let Some(start) = start_at.filter(|s| s.is_finite() && *s > 0.0) {
            self.set("start", &format!("{start:.3}"))?;
        }
        self.set("pause", "no")?;
        self.command(&["loadfile", path, "replace"])
    }

    fn toggle_pause(&self) -> AppResult<()> {
        self.command(&["cycle", "pause"])
    }

    fn seek(&self, seconds: f64, relative: bool) -> AppResult<()> {
        let seconds = format!("{:.3}", finite(seconds)?);
        let mode = if relative { "relative" } else { "absolute" };
        self.command(&["seek", &seconds, mode])
    }

    fn set_volume(&self, volume: f64) -> AppResult<()> {
        self.set("volume", &finite(volume)?.clamp(0.0, 100.0).to_string())
    }

    fn toggle_mute(&self) -> AppResult<()> {
        self.command(&["cycle", "mute"])
    }

    fn set_audio(&self, track_id: i64) -> AppResult<()> {
        self.set("aid", &track_id.to_string())
    }

    fn set_subtitle(&self, track_id: Option<i64>) -> AppResult<()> {
        let value = track_id.map_or_else(|| "no".to_owned(), |id| id.to_string());
        self.set("sid", &value)
    }

    fn add_subtitle(&self, path: &str) -> AppResult<()> {
        self.command(&["sub-add", path, "select"]).map_err(|error| {
            log::warn!("could not load a subtitle file: {error}");
            AppError::Player("that subtitle file could not be loaded".into())
        })
    }

    fn set_subtitle_lift(&self, fraction: f64) -> AppResult<()> {
        let lift = finite(fraction)?.clamp(0.0, 0.5) * SCALED_HEIGHT;
        let margin = (SUBTITLE_MARGIN + lift).round();
        self.command_async(&["set", "sub-margin-y", &margin.to_string()])
    }
}

/// Sets up the parts of the player that live as long as the app. Call on the
/// main thread.
pub fn install(app: &AppHandle) {
    now_playing::install(app);
}

fn now_playing_info(item: &PlaybackItem) -> now_playing::Info {
    let episode = match (item.season, item.episode) {
        (Some(season), Some(episode)) => {
            let code = format!("S{season} E{episode}");
            Some(match &item.episode_name {
                Some(name) => format!("{code} · {name}"),
                None => code,
            })
        }
        _ => None,
    };
    now_playing::Info {
        file_id: item.file_id,
        title: item.title.clone(),
        subtitle: episode,
        poster: item.poster.clone(),
        has_next: item.next.is_some(),
    }
}

#[derive(Debug, Serialize)]
pub struct Opened {
    /// False when the video is in the separate fallback window.
    pub embedded: bool,
    pub item: PlaybackItem,
}

/// Volume survives from one file to the next.
#[derive(Clone, Copy)]
struct Sound {
    volume: f64,
    muted: bool,
}

pub struct PlayerHost {
    /// Serializes opening and closing. The main thread never takes it,
    /// because whoever holds it may be waiting for the main thread.
    gate: Mutex<()>,
    /// Only ever locked for a moment.
    current: Mutex<Option<Arc<Session>>>,
    sound: Mutex<Sound>,
}

impl Default for PlayerHost {
    fn default() -> Self {
        Self {
            gate: Mutex::new(()),
            current: Mutex::new(None),
            sound: Mutex::new(Sound {
                volume: 100.0,
                muted: false,
            }),
        }
    }
}

impl PlayerHost {
    fn session(&self) -> Option<Arc<Session>> {
        lock(&self.current).clone()
    }

    fn require(&self) -> AppResult<Arc<Session>> {
        self.session()
            .ok_or_else(|| AppError::Player("nothing is playing".into()))
    }

    pub fn open(&self, app: &AppHandle, file_id: i64, start_at: Option<f64>) -> AppResult<Opened> {
        let _gate = lock(&self.gate);
        let item = queries::playback_item(&app.state::<AppState>().db(), file_id)?;
        if !Path::new(&item.path).is_file() {
            return Err(AppError::Player(
                "the file is not there any more. Is the drive connected?".into(),
            ));
        }
        // Saves the old file's position before anything of the new one exists.
        if self.close_current(app) {
            jobs::notify_library_updated(app);
        }

        let sound = *lock(&self.sound);
        let volume = sound.volume.to_string();
        let mute = if sound.muted { "yes" } else { "no" };
        let mpv = Mpv::new(&[("volume", &volume), ("mute", mute)])?;
        let session = Arc::new(Session::new(Arc::clone(&mpv), file_id, item.path.clone()));
        // The surface (and its render context) must exist before loading.
        let surface = self.create_surface(app, &session)?;
        let embedded = surface.is_embedded();
        session.set_surface(surface);

        let started = session
            .watch(app)
            .and_then(|()| mpv.load(&item.path, start_at));
        if let Err(error) = started {
            session.close(app, false);
            return Err(error);
        }
        *lock(&self.current) = Some(session);
        now_playing::begin(now_playing_info(&item));
        crate::menu::set_playing(app, true, item.next.is_some());
        log::info!("playing file {file_id} (embedded: {embedded})");
        Ok(Opened { embedded, item })
    }

    /// Embeds the video in the main window, or opens the fallback window.
    fn create_surface(
        &self,
        app: &AppHandle,
        session: &Arc<Session>,
    ) -> AppResult<embed::VideoSurface> {
        match embed::VideoSurface::embed(app, &session.mpv) {
            Ok(surface) => {
                // The HTML controls replace mpv's own on-screen display.
                session.mpv.set("osd-level", "0")?;
                Ok(surface)
            }
            Err(error) => {
                log::warn!("cannot embed the video, using a separate window: {error}");
                let keys = fallback_keys(app.clone(), Arc::downgrade(session));
                let surface = embed::VideoSurface::open_window(app, &session.mpv, keys)?;
                // mpv's built-in bar and messages stand in for controls.
                session.mpv.set("osd-level", "1")?;
                Ok(surface)
            }
        }
    }

    /// Closes the current session, if any. The caller holds the gate.
    fn close_current(&self, app: &AppHandle) -> bool {
        let session = lock(&self.current).take();
        session.is_some_and(|session| self.finish(app, &session, false))
    }

    fn finish(&self, app: &AppHandle, session: &Session, on_main_thread: bool) -> bool {
        if let (Some(volume), mute) = (session.mpv.get_f64("volume"), session.mpv.get_flag("mute"))
        {
            *lock(&self.sound) = Sound {
                volume,
                muted: mute,
            };
        }
        now_playing::end(session.file_id);
        crate::menu::set_playing(app, false, false);
        session.close(app, on_main_thread)
    }

    pub fn close(&self, app: &AppHandle) {
        let _gate = lock(&self.gate);
        if self.close_current(app) {
            jobs::notify_library_updated(app);
        }
    }

    /// Playback ended from the native side: the fallback window was closed, or
    /// mpv quit. Does nothing if `session` is no longer the current one.
    pub fn close_from_native(&self, app: &AppHandle, session: &Arc<Session>) {
        let _gate = lock(&self.gate);
        if !self
            .session()
            .is_some_and(|current| Arc::ptr_eq(&current, session))
        {
            return;
        }
        if self.close_current(app) {
            session::emit_closed(app, session.file_id);
            jobs::notify_library_updated(app);
        }
    }

    /// The app is closing. Called on the main thread, so it takes no gate.
    fn shutdown(&self, app: &AppHandle) {
        let session = lock(&self.current).take();
        if let Some(session) = session {
            self.finish(app, &session, true);
        }
    }
}

/// Handles the fallback window's keys. Runs on the main thread, so it only
/// queues work.
fn fallback_keys(app: AppHandle, session: std::sync::Weak<Session>) -> embed::KeyHandler {
    Arc::new(move |code, characters, modifiers| {
        let Some(session) = session.upgrade() else {
            return;
        };
        match keys::key_action(code, characters, modifiers) {
            KeyAction::Close => {
                let app = app.clone();
                thread::spawn(move || {
                    app.state::<PlayerHost>().close_from_native(&app, &session);
                });
            }
            KeyAction::ToggleFullscreen => {
                let app = app.clone();
                thread::spawn(move || {
                    if let Some(surface) = session.surface() {
                        if let Err(error) = surface.toggle_window_fullscreen(&app) {
                            log::warn!("could not toggle full screen: {error}");
                        }
                    }
                });
            }
            KeyAction::Forward(key) => {
                if let Err(error) = session.mpv.command_async(&["keypress", &key]) {
                    log::debug!("key not delivered: {error}");
                }
            }
            KeyAction::Ignore => {}
        }
    })
}

/// Saves progress when the main window is closed, and follows its scale.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }
    let app = window.app_handle();
    match event {
        WindowEvent::CloseRequested { .. } => app.state::<PlayerHost>().shutdown(app),
        WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            let session = app.state::<PlayerHost>().session();
            if let Some(surface) = session.and_then(|session| session.surface()) {
                surface.set_scale(app, *scale_factor);
            }
        }
        _ => {}
    }
}

/// Saves progress when the app quits, e.g. with Cmd+Q.
pub fn on_run_event(app: &AppHandle, event: &RunEvent) {
    if matches!(event, RunEvent::Exit) {
        app.state::<PlayerHost>().shutdown(app);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod engine_tests;
