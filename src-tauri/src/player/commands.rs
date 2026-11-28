//! The player commands of `docs/API.md`. All are `async`, which keeps them
//! off the main thread; opening and closing wait for it.

use std::path::Path;
use std::sync::Arc;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use super::{Opened, Player, PlayerHost};
use crate::error::{AppError, AppResult};

const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt"];

#[tauri::command(async)]
pub fn player_open(
    app: AppHandle,
    host: State<'_, PlayerHost>,
    file_id: i64,
    start_at: Option<f64>,
) -> AppResult<Opened> {
    host.open(&app, file_id, start_at)
}

#[tauri::command(async)]
pub fn player_close(app: AppHandle, host: State<'_, PlayerHost>) {
    host.close(&app);
}

#[tauri::command(async)]
pub fn player_toggle_pause(host: State<'_, PlayerHost>) -> AppResult<()> {
    host.require()?.mpv.toggle_pause()
}

#[tauri::command(async)]
pub fn player_seek(host: State<'_, PlayerHost>, seconds: f64, relative: bool) -> AppResult<()> {
    host.require()?.mpv.seek(seconds, relative)
}

#[tauri::command(async)]
pub fn player_set_volume(host: State<'_, PlayerHost>, volume: f64) -> AppResult<()> {
    host.require()?.mpv.set_volume(volume)
}

#[tauri::command(async)]
pub fn player_toggle_mute(host: State<'_, PlayerHost>) -> AppResult<()> {
    host.require()?.mpv.toggle_mute()
}

#[tauri::command(async)]
pub fn player_set_audio(host: State<'_, PlayerHost>, track_id: i64) -> AppResult<()> {
    host.require()?.mpv.set_audio(track_id)
}

#[tauri::command(async)]
pub fn player_set_subtitle(host: State<'_, PlayerHost>, track_id: Option<i64>) -> AppResult<()> {
    host.require()?.mpv.set_subtitle(track_id)
}

/// The file is picked here, so the webview never names a path.
#[tauri::command(async)]
pub fn player_add_subtitle(app: AppHandle, host: State<'_, PlayerHost>) -> AppResult<()> {
    let session = host.require()?;
    let mut dialog = app
        .dialog()
        .file()
        .add_filter("Subtitles", SUBTITLE_EXTENSIONS);
    if let Some(folder) = Path::new(&session.path).parent() {
        dialog = dialog.set_directory(folder);
    }
    let Some(picked) = dialog.blocking_pick_file() else {
        return Ok(());
    };
    let path = picked
        .into_path()
        .ok()
        .and_then(|path| path.to_str().map(str::to_owned))
        .ok_or_else(|| AppError::Player("that file cannot be opened".into()))?;
    // Another file may have started while the picker was open.
    if !Arc::ptr_eq(&session, &host.require()?) {
        return Ok(());
    }
    session.mpv.add_subtitle(&path)
}

/// Keeps subtitles clear of the controls while they are shown.
#[tauri::command(async)]
pub fn player_set_subtitle_lift(host: State<'_, PlayerHost>, fraction: f64) -> AppResult<()> {
    host.require()?.mpv.set_subtitle_lift(fraction)
}

/// Sets full screen to `wanted(current)` on whichever window shows the video.
/// Returns the state before and after.
fn change_fullscreen(
    app: &AppHandle,
    host: &PlayerHost,
    wanted: impl FnOnce(bool) -> bool,
) -> AppResult<(bool, bool)> {
    let surface = host.session().and_then(|session| session.surface());
    if let Some(surface) = surface.filter(|surface| !surface.is_embedded()) {
        let current = surface.is_window_fullscreen();
        let next = wanted(current);
        if next != current {
            surface.set_window_fullscreen(app, next)?;
        }
        return Ok((current, next));
    }
    let window = app.get_webview_window("main").ok_or(AppError::NotFound)?;
    let current = window.is_fullscreen()?;
    let next = wanted(current);
    if next != current {
        // The macOS animation runs on after this returns.
        window.set_fullscreen(next)?;
    }
    Ok((current, next))
}

#[tauri::command(async)]
pub fn player_toggle_fullscreen(app: AppHandle, host: State<'_, PlayerHost>) -> AppResult<bool> {
    change_fullscreen(&app, &host, |current| !current).map(|(_, now)| now)
}

#[tauri::command(async)]
pub fn player_exit_fullscreen(app: AppHandle, host: State<'_, PlayerHost>) -> AppResult<bool> {
    change_fullscreen(&app, &host, |_| false).map(|(was, _)| was)
}
