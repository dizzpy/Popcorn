// Wrappers around Tauri invoke/listen. The contract is docs/API.md:
// arguments are camelCase, returned fields are snake_case.

const tauri = () => window.__TAURI__;
const invoke = (command, args) => tauri().core.invoke(command, args);

export const listen = (event, handler) =>
  tauri().event.listen(event, ({ payload }) => handler(payload));

export const fileUrl = (path) => (path ? tauri().core.convertFileSrc(path) : null);

/** Failed commands reject with a friendly string. */
export const errorMessage = (error) =>
  typeof error === 'string' ? error : (error?.message ?? 'Something went wrong.');

export const api = {
  appInfo: () => invoke('app_info'),
  getSettings: () => invoke('get_settings'),
  pickMediaFolder: () => invoke('pick_media_folder'),
  finishOnboarding: () => invoke('finish_onboarding'),
  setTmdbKey: (key) => invoke('set_tmdb_key', { key }),
  testTmdbKey: (key) => invoke('test_tmdb_key', { key }),
  rescan: () => invoke('rescan'),
  listTitles: () => invoke('list_titles'),
  getTitle: (id) => invoke('get_title', { id }),
  continueWatching: () => invoke('continue_watching'),
  searchLibrary: (query) => invoke('search_library', { query }),
  searchEpisodes: (query) => invoke('search_episodes', { query }),
  setWatched: (fileId, watched) => invoke('set_watched', { fileId, watched }),
  setTitleWatched: (titleId, watched) => invoke('set_title_watched', { titleId, watched }),
  revealFile: (fileId) => invoke('reveal_file', { fileId }),
  openExternal: (target) => invoke('open_external', { target }),

  tmdbSearch: (kind, query) => invoke('tmdb_search', { kind, query }),
  fixMatch: (titleId, tmdbId) => invoke('fix_match', { titleId, tmdbId }),
  refreshTitle: (titleId) => invoke('refresh_title', { titleId }),
  refreshAllMetadata: () => invoke('refresh_all_metadata'),
  clearImageCache: () => invoke('clear_image_cache'),

  playerOpen: (fileId, startAt) => invoke('player_open', { fileId, startAt }),
  playerClose: () => invoke('player_close'),
  playerTogglePause: () => invoke('player_toggle_pause'),
  playerSeek: (seconds, relative) => invoke('player_seek', { seconds, relative }),
  playerSetVolume: (volume) => invoke('player_set_volume', { volume }),
  playerToggleMute: () => invoke('player_toggle_mute'),
  playerSetAudio: (trackId) => invoke('player_set_audio', { trackId }),
  playerSetSubtitle: (trackId) => invoke('player_set_subtitle', { trackId }),
  playerAddSubtitle: () => invoke('player_add_subtitle'),
  playerToggleFullscreen: () => invoke('player_toggle_fullscreen'),
  playerExitFullscreen: () => invoke('player_exit_fullscreen'),
  playerSetSubtitleLift: (fraction) => invoke('player_set_subtitle_lift', { fraction }),
};
