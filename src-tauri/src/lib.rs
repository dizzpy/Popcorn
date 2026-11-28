mod commands;
mod db;
mod error;
mod jobs;
mod library;
mod logger;
mod menu;
mod player;
mod progress;
mod settings;
mod state;
mod tmdb;

use tauri::Manager;

use crate::state::AppState;

pub fn run() {
    logger::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(player::PlayerHost::default())
        .on_window_event(|window, event| {
            player::on_window_event(window, event);
            commands::on_window_event(window, event);
        })
        .setup(|app| {
            let data_dir = app.path().data_dir()?.join("Popcorn");
            #[cfg(debug_assertions)]
            let data_dir = player::smoke::data_dir().unwrap_or(data_dir);
            std::fs::create_dir_all(data_dir.join("images"))?;
            let db = db::open(&data_dir.join("popcorn.db"))?;
            app.manage(AppState::new(db, data_dir));
            if let Err(error) = jobs::start_watcher(app.handle()) {
                log::warn!("not watching the media folder: {error}");
            }
            jobs::start_metadata_workers(app.handle());
            // Now, as well as after the scan, which fails if the folder is away.
            jobs::queue_metadata(app.handle());
            jobs::start_scan(app.handle());
            if let Err(error) = menu::install(app.handle()) {
                log::warn!("could not build the menu bar: {error}");
            }
            player::install(app.handle());
            #[cfg(debug_assertions)]
            player::smoke::start(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_settings,
            commands::pick_media_folder,
            commands::finish_onboarding,
            commands::set_tmdb_key,
            commands::test_tmdb_key,
            commands::rescan,
            commands::list_titles,
            commands::get_title,
            commands::continue_watching,
            commands::search_library,
            commands::search_episodes,
            commands::set_watched,
            commands::set_title_watched,
            commands::reveal_file,
            commands::open_external,
            commands::tmdb_search,
            commands::fix_match,
            commands::refresh_title,
            commands::refresh_all_metadata,
            commands::clear_image_cache,
            player::commands::player_open,
            player::commands::player_close,
            player::commands::player_toggle_pause,
            player::commands::player_seek,
            player::commands::player_set_volume,
            player::commands::player_toggle_mute,
            player::commands::player_set_audio,
            player::commands::player_set_subtitle,
            player::commands::player_add_subtitle,
            player::commands::player_toggle_fullscreen,
            player::commands::player_exit_fullscreen,
            player::commands::player_set_subtitle_lift,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start Popcorn")
        .run(|app, event| player::on_run_event(app, &event));
}
