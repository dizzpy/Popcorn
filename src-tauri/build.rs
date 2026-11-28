/// Every app command must be listed here so that it gets its own permission,
/// which `capabilities/default.json` then has to grant explicitly.
const COMMANDS: &[&str] = &[
    "app_info",
    "get_settings",
    "pick_media_folder",
    "finish_onboarding",
    "set_tmdb_key",
    "test_tmdb_key",
    "rescan",
    "list_titles",
    "get_title",
    "continue_watching",
    "search_library",
    "search_episodes",
    "set_watched",
    "set_title_watched",
    "reveal_file",
    "open_external",
    "tmdb_search",
    "fix_match",
    "refresh_title",
    "refresh_all_metadata",
    "clear_image_cache",
    "player_open",
    "player_close",
    "player_toggle_pause",
    "player_seek",
    "player_set_volume",
    "player_toggle_mute",
    "player_set_audio",
    "player_set_subtitle",
    "player_add_subtitle",
    "player_toggle_fullscreen",
    "player_exit_fullscreen",
    "player_set_subtitle_lift",
];

fn main() {
    let manifest = tauri_build::AppManifest::new().commands(COMMANDS);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("failed to run tauri-build");
}
