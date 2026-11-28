//! All `#[tauri::command]` functions except the player's.
//!
//! `async` keeps them off the main thread. The contract with the UI is
//! described in `docs/API.md`.

use std::process::Command;

use serde::Serialize;
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, State, Window, WindowEvent};
use tauri_plugin_dialog::DialogExt;

use crate::db::metadata;
use crate::db::queries::{self, ContinueItem, TitleDetails, TitleSummary};
use crate::error::{AppError, AppResult};
use crate::library::group::TitleKind;
use crate::state::{unix_now, write, AppState};
use crate::tmdb::cache::ImageCache;
use crate::tmdb::client::{self, Client};
use crate::tmdb::fetch;
use crate::tmdb::refresh::{self, Mode, Outcome};
use crate::{jobs, library, progress, settings};

#[derive(Serialize)]
pub struct AppInfo {
    version: &'static str,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[derive(Serialize)]
pub struct Settings {
    media_folder: Option<String>,
    has_tmdb_key: bool,
    onboarded: bool,
}

/// The key itself is never sent back to the UI.
#[tauri::command(async)]
pub fn get_settings(state: State<'_, AppState>) -> AppResult<Settings> {
    let db = state.db();
    Ok(Settings {
        media_folder: settings::get(&db, settings::MEDIA_FOLDER)?,
        has_tmdb_key: settings::get(&db, settings::TMDB_KEY)?.is_some(),
        onboarded: settings::get(&db, settings::ONBOARDED)?.is_some(),
    })
}

/// Opens the native folder picker. The path is chosen and saved here, so the
/// webview can never point the library somewhere by itself.
#[tauri::command(async)]
pub fn pick_media_folder(app: AppHandle, state: State<'_, AppState>) -> AppResult<Option<String>> {
    let Some(picked) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .ok()
        .and_then(|path| path.to_str().map(str::to_owned))
        .ok_or(AppError::FolderUnavailable)?;
    use_media_folder(&app, &state, &path)?;
    Ok(Some(path))
}

/// Saves the folder, then watches and scans it.
fn use_media_folder(app: &AppHandle, state: &AppState, path: &str) -> AppResult<()> {
    settings::set(&state.db(), settings::MEDIA_FOLDER, path)?;
    jobs::start_watcher(app)?;
    jobs::start_scan(app);
    Ok(())
}

#[derive(Clone, Serialize)]
struct DropPayload {
    /// While dragging: whether something is over the window.
    over: bool,
    /// After a drop: the folder now in use.
    path: Option<String>,
    error: Option<String>,
}

/// A folder dragged onto the welcome screen becomes the media folder. It is
/// taken from the native drop, so the web view never names a path; and only
/// before onboarding is finished, so a stray drop later changes nothing.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::DragDrop(drag) = event else {
        return;
    };
    let app = window.app_handle().clone();
    let drag = drag.clone();
    // Reading settings touches the database, which the main thread avoids.
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let onboarded = settings::get(&state.db(), settings::ONBOARDED)
            .ok()
            .flatten()
            .is_some_and(|value| value == "1");
        if onboarded {
            return;
        }
        let payload = match drag {
            DragDropEvent::Enter { .. } | DragDropEvent::Over { .. } => DropPayload {
                over: true,
                path: None,
                error: None,
            },
            DragDropEvent::Leave => DropPayload {
                over: false,
                path: None,
                error: None,
            },
            DragDropEvent::Drop { paths, .. } => dropped(&app, &state, &paths),
            _ => return,
        };
        if let Err(error) = app.emit("onboarding://drop", payload) {
            log::warn!("could not report the drop: {error}");
        }
    });
}

fn dropped(app: &AppHandle, state: &AppState, paths: &[std::path::PathBuf]) -> DropPayload {
    let failed = |message: &str| DropPayload {
        over: false,
        path: None,
        error: Some(message.to_owned()),
    };
    let Some(folder) = paths.iter().find(|path| path.is_dir()) else {
        return failed("Drop a folder, not a file.");
    };
    let Some(path) = folder.to_str() else {
        return failed("That folder cannot be opened.");
    };
    match use_media_folder(app, state, path) {
        Ok(()) => DropPayload {
            over: false,
            path: Some(path.to_owned()),
            error: None,
        },
        Err(error) => failed(&error.to_string()),
    }
}

#[tauri::command(async)]
pub fn finish_onboarding(state: State<'_, AppState>) -> AppResult<()> {
    settings::set(&state.db(), settings::ONBOARDED, "1")
}

/// An empty key removes the saved one.
#[tauri::command(async)]
pub fn set_tmdb_key(app: AppHandle, state: State<'_, AppState>, key: String) -> AppResult<()> {
    let key = key.trim();
    // Anything still waiting belongs to the old key.
    state.metadata_queue.reset();
    if key.is_empty() {
        return settings::remove(&state.db(), settings::TMDB_KEY);
    }
    {
        let db = state.db();
        settings::set(&db, settings::TMDB_KEY, key)?;
        metadata::retry_unmatched(&db)?;
    }
    jobs::queue_metadata(&app);
    Ok(())
}

/// Tests the given key, or the saved one when none is given.
#[tauri::command(async)]
pub fn test_tmdb_key(state: State<'_, AppState>, key: Option<String>) -> AppResult<()> {
    match key {
        Some(key) if !key.trim().is_empty() => Client::new(&key).test_key(),
        _ => saved_client(&state)?.test_key(),
    }
}

fn saved_client(state: &AppState) -> AppResult<Client> {
    let key = settings::get(&state.db(), settings::TMDB_KEY)?.ok_or(AppError::MissingTmdbKey)?;
    Ok(Client::new(&key))
}

#[derive(Serialize)]
pub struct TmdbSearchHit {
    tmdb_id: i64,
    title: String,
    year: Option<i64>,
    poster_url: Option<String>,
}

/// For the "Fix match" dialog.
#[tauri::command(async)]
pub fn tmdb_search(
    state: State<'_, AppState>,
    kind: String,
    query: String,
) -> AppResult<Vec<TmdbSearchHit>> {
    let kind = match kind.as_str() {
        "movie" => TitleKind::Movie,
        "show" => TitleKind::Show,
        _ => return Err(AppError::NotFound),
    };
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let results = fetch::search(&saved_client(&state)?, kind, query, None)?;
    Ok(results
        .into_iter()
        .map(|result| TmdbSearchHit {
            tmdb_id: result.tmdb_id,
            title: result.title,
            year: result.year,
            poster_url: result.poster.as_deref().and_then(client::poster_url),
        })
        .collect())
}

/// Uses the chosen TMDB entry from now on. Nothing changes if fetching it fails.
#[tauri::command(async)]
pub fn fix_match(
    app: AppHandle,
    state: State<'_, AppState>,
    title_id: i64,
    tmdb_id: i64,
) -> AppResult<()> {
    refetch(&app, &state, title_id, Mode::Fix(tmdb_id))
}

#[tauri::command(async)]
pub fn refresh_title(app: AppHandle, state: State<'_, AppState>, title_id: i64) -> AppResult<()> {
    refetch(&app, &state, title_id, Mode::Manual)
}

fn refetch(app: &AppHandle, state: &AppState, title_id: i64, mode: Mode) -> AppResult<()> {
    match refresh::refresh(state, &saved_client(state)?, title_id, mode)? {
        Outcome::Stored => jobs::notify_library_updated(app),
        // The title disappeared while its new match was being fetched.
        Outcome::Skipped if metadata::title_kind(&state.db(), title_id)?.is_none() => {
            return Err(AppError::NotFound);
        }
        Outcome::Skipped | Outcome::NoMatch => {}
    }
    Ok(())
}

#[tauri::command(async)]
pub fn refresh_all_metadata(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    {
        let db = state.db();
        if settings::get(&db, settings::TMDB_KEY)?.is_none() {
            return Err(AppError::MissingTmdbKey);
        }
        metadata::mark_all_stale(&db)?;
    }
    jobs::queue_metadata(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn clear_image_cache(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    // So that only the fetches already running have to be waited for.
    state.metadata_queue.reset();
    {
        let _images = write(&state.image_gate);
        // Paths go first: a failure can then only leave unused files behind.
        metadata::forget_images(&mut state.db())?;
        ImageCache::new(&state.data_dir).clear()?;
    }
    jobs::notify_library_updated(&app);
    jobs::queue_metadata(&app);
    Ok(())
}

#[tauri::command]
pub fn rescan(app: AppHandle) {
    jobs::start_scan(&app);
}

#[tauri::command(async)]
pub fn list_titles(state: State<'_, AppState>) -> AppResult<Vec<TitleSummary>> {
    queries::list_titles(&state.db())
}

#[tauri::command(async)]
pub fn get_title(state: State<'_, AppState>, id: i64) -> AppResult<TitleDetails> {
    queries::title_details(&state.db(), id)
}

#[tauri::command(async)]
pub fn continue_watching(state: State<'_, AppState>) -> AppResult<Vec<ContinueItem>> {
    queries::continue_watching(&state.db())
}

#[tauri::command(async)]
pub fn search_library(state: State<'_, AppState>, query: String) -> AppResult<Vec<i64>> {
    let documents = queries::search_documents(&state.db())?;
    Ok(library::search::search(&query, &documents))
}

#[tauri::command(async)]
pub fn search_episodes(state: State<'_, AppState>, query: String) -> AppResult<Vec<ContinueItem>> {
    queries::search_episodes(&state.db(), &query)
}

#[tauri::command(async)]
pub fn set_watched(
    app: AppHandle,
    state: State<'_, AppState>,
    file_id: i64,
    watched: bool,
) -> AppResult<()> {
    progress::set_watched(&state.db(), file_id, watched, unix_now())?;
    jobs::notify_library_updated(&app);
    Ok(())
}

/// Marks a movie, or every episode of a show.
#[tauri::command(async)]
pub fn set_title_watched(
    app: AppHandle,
    state: State<'_, AppState>,
    title_id: i64,
    watched: bool,
) -> AppResult<()> {
    {
        let db = state.db();
        for file_id in queries::title_file_ids(&db, title_id)? {
            progress::set_watched(&db, file_id, watched, unix_now())?;
        }
    }
    jobs::notify_library_updated(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn reveal_file(state: State<'_, AppState>, file_id: i64) -> AppResult<()> {
    let path = queries::file_path(&state.db(), file_id)?;
    #[cfg(target_os = "macos")]
    Command::new("/usr/bin/open").arg("-R").arg(path).status()?;
    Ok(())
}

/// Only these fixed pages can be opened from the UI.
#[tauri::command(async)]
pub fn open_external(target: String) -> AppResult<()> {
    let url = match target.as_str() {
        "tmdb_api" => "https://www.themoviedb.org/settings/api",
        "tmdb" => "https://www.themoviedb.org",
        "github" => "https://github.com/dizzpy/popcorn",
        _ => return Err(AppError::NotFound),
    };
    #[cfg(target_os = "macos")]
    Command::new("/usr/bin/open").arg(url).status()?;
    Ok(())
}
