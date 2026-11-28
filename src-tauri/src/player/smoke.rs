//! Debug builds only: plays through the real app and logs what happened.
//!
//! ```sh
//! POPCORN_DATA_DIR=/tmp/popcorn-data POPCORN_SMOKE_MEDIA=/folder/of/clips \
//!   cargo run
//! ```
//!
//! It never runs against the real data folder: without `POPCORN_DATA_DIR` it
//! refuses to start. The folder needs two or more clips, each longer than two
//! minutes. Add `POPCORN_FORCE_FALLBACK=1` to exercise the fallback window.
//! The app quits when it is done. `POPCORN_SMOKE_QUIT_PLAYING=app` or
//! `=window` instead quits, or closes the main window, during playback.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Listener, Manager};

use super::{Player, PlayerHost};
use crate::db::queries;
use crate::error::{AppError, AppResult};
use crate::state::{unix_now, AppState};
use crate::{jobs, library, progress, settings};

const EVENTS: &[&str] = &[
    "player://state",
    "player://time",
    "player://tracks",
    "player://ended",
    "player://error",
    "player://closed",
    "library://updated",
];

pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("POPCORN_DATA_DIR").map(PathBuf::from)
}

pub fn start(app: &AppHandle) {
    let Some(media) = std::env::var_os("POPCORN_SMOKE_MEDIA").map(PathBuf::from) else {
        return;
    };
    if data_dir().is_none() {
        log::error!("smoke: set POPCORN_DATA_DIR to a scratch folder first");
        return;
    }
    for name in EVENTS {
        app.listen_any(*name, move |event| {
            log::info!("smoke event {name} {}", event.payload());
        });
    }
    let app = app.clone();
    thread::spawn(move || {
        match run(&app, media) {
            Ok(()) => log::info!("smoke: finished"),
            Err(error) => log::error!("smoke: {error}"),
        }
        app.exit(0);
    });
}

fn pause(millis: u64) {
    thread::sleep(Duration::from_millis(millis));
}

fn report(app: &AppHandle, label: &str) {
    let host = app.state::<PlayerHost>();
    let Some(session) = host.session() else {
        log::info!("smoke [{label}] no session");
        return;
    };
    let read = |name| session.mpv.get_string(name).unwrap_or_default();
    log::info!(
        "smoke [{label}] time={} pause={} hwdec={} video-out={} vo-drops={} avsync={}",
        read("time-pos"),
        read("pause"),
        read("hwdec-current"),
        read("current-vo"),
        read("frame-drop-count"),
        read("avsync"),
    );
    if let Some(surface) = session.surface() {
        log::info!("smoke [{label}] surface: {}", surface.describe(app));
    }
    {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = app.run_on_main_thread(move || {
            let _ = sender.send(super::now_playing::describe());
        });
        if let Ok(text) = receiver.recv_timeout(Duration::from_secs(2)) {
            log::info!("smoke [{label}] now playing: {text}");
        }
    }
    log::info!("smoke [{label}] keeping the display awake: {}", awake());
}

/// Whether macOS currently holds our display-sleep assertion.
fn awake() -> bool {
    let ours = format!("pid {}(", std::process::id());
    std::process::Command::new("/usr/bin/pmset")
        .args(["-g", "assertions"])
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line.contains(&ours) && line.contains("PreventUserIdleDisplaySleep"))
        })
}

/// Presses keys in the fallback window, then closes it with Escape.
fn fallback_keys(app: &AppHandle) -> AppResult<()> {
    let host = app.state::<PlayerHost>();
    let surface = host.require()?.surface().ok_or(AppError::NotFound)?;
    for (code, characters, label) in [
        (49, c" ", "SPACE"),
        (49, c" ", "SPACE again"),
        (124, c"\u{f703}", "RIGHT"),
        (3, c"f", "f"),
        (46, c"m", "m"),
        (46, c"m", "m again"),
        (3, c"f", "f again"),
    ] {
        surface.press_key(app, code, characters);
        pause(900);
        log::info!("smoke: pressed {label}");
        report(app, label);
        let full = surface.is_window_fullscreen();
        log::info!("smoke: window full screen: {full}");
    }
    surface.press_key(app, 53, c"\x1b");
    pause(1500);
    log::info!(
        "smoke: after Escape, session open: {}, awake: {}",
        host.session().is_some(),
        awake()
    );
    Ok(())
}

fn stored(app: &AppHandle, label: &str, file_id: i64) -> AppResult<()> {
    let state = app.state::<AppState>();
    let db = state.db();
    let saved = progress::get(&db, file_id)?;
    let size: (Option<i64>, Option<i64>) = db.query_row(
        "SELECT width, height FROM files WHERE id = ?1",
        [file_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    log::info!("smoke [{label}] db file {file_id}: {saved:?} size={size:?}");
    Ok(())
}

fn run(app: &AppHandle, media: PathBuf) -> AppResult<()> {
    let state = app.state::<AppState>();
    let folder = media.to_str().ok_or(AppError::FolderUnavailable)?;
    settings::set(&state.db(), settings::MEDIA_FOLDER, folder)?;
    let walk = library::scan::walk(&media, |_| {})?;
    state.with_db(|db| library::scan::apply(db, walk, unix_now()))?;
    jobs::start_scan(app);
    let files: Vec<i64> = {
        let db = state.db();
        let mut statement = db.prepare("SELECT id FROM files WHERE missing = 0 ORDER BY path")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let [first, second, ..] = files[..] else {
        return Err(AppError::NotFound);
    };
    // A resume point that must survive opening and an immediate close.
    progress::save(&state.db(), second, 65.0, Some(150.0), 1)?;
    pause(1500);

    // Let the video show through, as the real UI does while playing.
    if let Some(window) = app.get_webview_window("main") {
        window.eval(
            "document.documentElement.classList.add('playing');\
             for (const e of [document.documentElement, document.body]) \
             e.style.background = 'transparent';",
        )?;
    }
    let host = app.state::<PlayerHost>();

    let opened = host.open(app, second, Some(65.0))?;
    host.close(app);
    log::info!("smoke: closed at once, embedded={}", opened.embedded);
    stored(app, "after instant close", second)?;
    queries::playback_item(&state.db(), second)?;

    host.open(app, first, None)?;
    pause(7500);
    report(app, "playing 7.5s");
    stored(app, "periodic save", first)?;
    let mpv = host.require()?.mpv.clone();
    mpv.toggle_pause()?;
    pause(700);
    stored(app, "saved on pause", first)?;
    report(app, "paused");
    mpv.seek(80.0, false)?;
    pause(700);
    stored(app, "saved on seek", first)?;
    mpv.toggle_pause()?;
    pause(1500);
    drop(mpv);

    log::info!("smoke: switching while playing");
    host.open(app, second, Some(100.0))?;
    pause(2500);
    report(app, "second file");
    stored(app, "old file after switch", first)?;
    stored(app, "new file", second)?;

    // To the end, for ended and watched.
    host.require()?.mpv.seek(147.0, false)?;
    pause(5000);
    report(app, "at end");
    stored(app, "watched at end", second)?;

    log::info!("smoke: reopening after the end");
    host.open(app, second, None)?;
    pause(2000);
    report(app, "reopened");

    let window = app.get_webview_window("main").ok_or(AppError::NotFound)?;
    if opened.embedded {
        let size = window.inner_size()?;
        window.set_size(tauri::PhysicalSize::new(
            size.width - 200,
            size.height - 100,
        ))?;
        pause(1500);
        report(app, "after resize");
    }
    if !opened.embedded {
        fallback_keys(app)?;
        host.open(app, second, None)?;
    }
    if let Ok(how) = std::env::var("POPCORN_SMOKE_QUIT_PLAYING") {
        // Leave mid-playback, before the first periodic save: only the quit
        // and window hooks can store this position.
        host.open(app, first, Some(30.0))?;
        pause(2000);
        stored(app, "before quit", first)?;
        if how == "window" {
            window.close()?;
            pause(3000);
        }
        return Ok(());
    }
    // Leave it on screen long enough to be inspected from outside.
    pause(
        std::env::var("POPCORN_SMOKE_HOLD_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            * 1000,
    );
    host.close(app);
    report(app, "closed");
    stored(app, "final", first)?;
    pause(1000);
    Ok(())
}
