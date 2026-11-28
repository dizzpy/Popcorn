# UI ↔ Rust API

This document describes the interface between Popcorn's web UI (`ui/`) and its Rust backend
(`src-tauri/`): the commands the UI can call, the data they return, and the events the backend
sends.

The Rust structs are the source of truth for field types. Most live in
[`src-tauri/src/db/queries.rs`](../src-tauri/src/db/queries.rs); the rest are linked where they
appear.

## Contents

- [Conventions](#conventions)
- [Commands](#commands)
  - [App and settings](#app-and-settings)
  - [Library](#library)
  - [Watch state](#watch-state)
  - [Metadata (TMDB)](#metadata-tmdb)
  - [Player](#player)
- [Data types](#data-types)
- [Events](#events)
- [Embedded playback in the UI](#embedded-playback-in-the-ui)
- [Adding a command](#adding-a-command)

## Conventions

### Calling the backend

The app enables `withGlobalTauri`, so the UI uses the global `window.__TAURI__` object and has no
npm dependencies. Command and event wrappers live in [`ui/js/api.js`](../ui/js/api.js).

| Purpose | API |
|---|---|
| Call a command | `window.__TAURI__.core.invoke(name, args)` |
| Load a local image | `window.__TAURI__.core.convertFileSrc(path)` |
| Subscribe to an event | `window.__TAURI__.event.listen(name, handler)` |

### Naming

- Command **arguments** are `camelCase` in JavaScript (`fileId`, `startAt`). Tauri maps them to
  the `snake_case` Rust parameters.
- **Returned fields** and event payloads are `snake_case` (`file_id`, `poster_path`).

### Types

The tables below use TypeScript-style notation. `T | null` means the field may be `null`. Every
ID is an integer. Times and durations are in seconds unless stated otherwise.

### Errors

Every command that can fail rejects its promise with a **single, user-facing message string**
(for example `"Could not reach TMDB. Check your internet connection."`), ready to show in a toast.
The messages are defined in [`src-tauri/src/error.rs`](../src-tauri/src/error.rs).

### Images

Every `*_path` image field (`poster_path`, `backdrop_path`, `still_path`, `profile_path`) is an
**absolute path to a cached file** on disk, or `null`. Load it with `convertFileSrc`. The asset
protocol may only read `~/Library/Application Support/Popcorn/images/`.

The one exception is `poster_url` in [`tmdb_search`](#metadata-tmdb), which is a full
`https://image.tmdb.org` URL, because search results are not cached.

### Permissions

Each command has its own permission, generated from the list in
[`src-tauri/build.rs`](../src-tauri/build.rs). The main window can only call commands granted in
[`src-tauri/capabilities/default.json`](../src-tauri/capabilities/default.json).

## Commands

### App and settings

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `app_info` | | `{ version: string }` | The app version from `Cargo.toml`. |
| `get_settings` | | [`Settings`](#settings) | |
| `pick_media_folder` | | `string \| null` | Opens the native folder picker. Saves the chosen folder and starts a scan. `null` if cancelled. |
| `finish_onboarding` | | | Marks first-run setup as complete. |
| `set_tmdb_key` | `key: string` | | Saves the key. An empty string removes it. Queues any missing or stale metadata. |
| `test_tmdb_key` | `key: string \| null` | | Resolves if TMDB accepts the key. `null` tests the saved key. |

The saved TMDB key is never returned to the UI. `get_settings` only reports whether one exists.

The media folder is always chosen through the native picker in Rust, so the UI can never set an
arbitrary path.

### Library

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `rescan` | | | Starts a scan of the media folder in the background. Progress arrives as [`scan://progress`](#events). |
| `list_titles` | | [`TitleSummary[]`](#titlesummary) | All movies and shows, sorted by title. |
| `get_title` | `id: number` | [`TitleDetails`](#titledetails) | |
| `continue_watching` | | [`ContinueItem[]`](#continueitem) | Most recent first. `position` is `0` for a next episode that has not been started. |
| `search_library` | `query: string` | `number[]` | Matching title IDs, best match first. |
| `search_episodes` | `query: string` | [`ContinueItem[]`](#continueitem) | Up to 12 episodes whose name matches. |
| `reveal_file` | `fileId: number` | | Shows the file in Finder. |
| `open_external` | `target: "tmdb_api" \| "tmdb" \| "github"` | | Opens one of a fixed set of web pages in the default browser. Any other value is rejected. |

### Watch state

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `set_watched` | `fileId: number, watched: boolean` | | Marks one file (a movie or an episode) as watched or unwatched. |
| `set_title_watched` | `titleId: number, watched: boolean` | | Marks every file of a title. |

Playback progress is saved by the backend while a file plays. The UI never writes progress
itself.

### Metadata (TMDB)

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `tmdb_search` | `kind: "movie" \| "show", query: string` | [`TmdbSearchHit[]`](#tmdbsearchhit) | Used by the **Fix match** dialog. |
| `fix_match` | `titleId: number, tmdbId: number` | | Links a title to a different TMDB entry. Resolves once the new metadata is stored. |
| `refresh_title` | `titleId: number` | | Downloads a title's metadata again. Resolves when done. |
| `refresh_all_metadata` | | | Queues every title for a refresh and returns immediately. |
| `clear_image_cache` | | | Deletes all cached images and queues them to be downloaded again. |

Saving a TMDB key and finishing a scan also queue any missing or stale metadata. A failed request
stores nothing, so cached data is never replaced by a partial result.

### Player

| Command | Arguments | Returns | Notes |
|---|---|---|---|
| `player_open` | `fileId: number, startAt: number \| null` | [`Opened`](#opened) | `startAt` is the start position in seconds; `null` plays from the beginning. Calling it while a file is playing switches files (used for "next episode"). |
| `player_close` | | | Stops playback and saves progress. |
| `player_toggle_pause` | | | |
| `player_seek` | `seconds: number, relative: boolean` | | Seeks by `seconds` if `relative`, otherwise to `seconds`. |
| `player_set_volume` | `volume: number` | | `0`–`100`. |
| `player_toggle_mute` | | | |
| `player_set_audio` | `trackId: number` | | A track `id` from [`player://tracks`](#events). |
| `player_set_subtitle` | `trackId: number \| null` | | `null` turns subtitles off. |
| `player_add_subtitle` | | | Opens a native file picker (`.srt`, `.ass`, `.ssa`, `.vtt`) and loads the chosen file. |
| `player_toggle_fullscreen` | | `boolean` | The new fullscreen state. |
| `player_exit_fullscreen` | | `boolean` | `true` if the window was fullscreen. |
| `player_set_subtitle_lift` | `fraction: number` | `null` | Raises subtitles above controls that cover this share of the window height (0 to 0.5); 0 puts them back. |

The UI decides where to resume (usually `TitleDetails.play.resume_position`) and passes it as
`startAt`.

## Data types

### Settings

Defined in [`commands.rs`](../src-tauri/src/commands.rs).

| Field | Type | Description |
|---|---|---|
| `media_folder` | `string \| null` | The chosen media folder. |
| `has_tmdb_key` | `boolean` | Whether a TMDB key is saved. |
| `onboarded` | `boolean` | Whether first-run setup is complete. |

### TitleSummary

A movie or show, as shown in grids and rows.

| Field | Type | Description |
|---|---|---|
| `id` | `number` | |
| `kind` | `"movie" \| "show"` | |
| `title` | `string` | TMDB title, or the title parsed from the file name if unmatched. |
| `year` | `number \| null` | |
| `rating` | `number \| null` | TMDB rating out of 10. |
| `runtime` | `number \| null` | In minutes. |
| `genres` | `string[]` | |
| `overview` | `string \| null` | |
| `poster_path` | `string \| null` | Cached image path. |
| `backdrop_path` | `string \| null` | Cached image path. |
| `added_at` | `number` | Unix time (seconds) when its newest file was added. |
| `file_count` | `number` | Files on disk (episodes, for a show). |
| `watched` | `boolean` | `true` when every file has been watched. |
| `matched` | `boolean` | `true` when linked to a TMDB entry. |

### TitleDetails

Every [`TitleSummary`](#titlesummary) field, plus:

| Field | Type | Description |
|---|---|---|
| `tmdb_id` | `number \| null` | |
| `tagline` | `string \| null` | |
| `certification` | `string \| null` | Age rating, such as `PG-13` or `TV-MA`. |
| `directors` | `string[]` | Directors of a movie, or creators of a show. |
| `cast` | [`CastMember[]`](#castmember) | |
| `files` | [`MovieFile[]`](#moviefile) | Files of a movie. Empty for shows. |
| `seasons` | [`Season[]`](#season) | Seasons of a show. Empty for movies. |
| `play` | [`PlayTarget`](#playtarget) `\| null` | What the main **Play** button should do. |

### CastMember

Defined in [`tmdb/metadata.rs`](../src-tauri/src/tmdb/metadata.rs).

| Field | Type | Description |
|---|---|---|
| `name` | `string` | |
| `character` | `string \| null` | |
| `profile_path` | `string \| null` | Cached image path. |

### FileEntry

Shared by [`MovieFile`](#moviefile) and [`EpisodeEntry`](#episodeentry), whose fields include
all of these.

| Field | Type | Description |
|---|---|---|
| `filename` | `string` | File name only, without the folder. |
| `size` | `number` | In bytes. |
| `width` | `number \| null` | Video width in pixels, once known. |
| `height` | `number \| null` | Video height in pixels, once known. |
| `position` | `number` | Last playback position in seconds. `0` if not started. |
| `duration` | `number \| null` | In seconds, once known. |
| `watched` | `boolean` | |

### MovieFile

| Field | Type |
|---|---|
| `file_id` | `number` |
| *…all [`FileEntry`](#fileentry) fields* | |

### Season

| Field | Type | Description |
|---|---|---|
| `number` | `number` | `0` for specials. |
| `name` | `string \| null` | |
| `episodes` | [`EpisodeEntry[]`](#episodeentry) | Only episodes that exist on disk. |

### EpisodeEntry

| Field | Type | Description |
|---|---|---|
| `file_id` | `number` | |
| `season` | `number` | |
| `episode` | `number` | |
| `name` | `string \| null` | |
| `overview` | `string \| null` | |
| `still_path` | `string \| null` | Cached image path. |
| *…all [`FileEntry`](#fileentry) fields* | | |

### PlayTarget

| Field | Type | Description |
|---|---|---|
| `file_id` | `number` | The file to play. |
| `season` | `number \| null` | Set for shows. |
| `episode` | `number \| null` | Set for shows. |
| `resume_position` | `number \| null` | Where to resume, or `null` to start from the beginning. |

### ContinueItem

| Field | Type | Description |
|---|---|---|
| `title_id` | `number` | |
| `file_id` | `number` | |
| `kind` | `"movie" \| "show"` | |
| `title` | `string` | |
| `season` | `number \| null` | |
| `episode` | `number \| null` | |
| `episode_name` | `string \| null` | |
| `position` | `number` | Seconds. |
| `duration` | `number \| null` | Seconds. |
| `poster_path` | `string \| null` | Cached image path. |
| `backdrop_path` | `string \| null` | Cached image path. |
| `still_path` | `string \| null` | Cached image path. |
| `updated_at` | `number` | Unix time (seconds) of the last progress update. |

### TmdbSearchHit

Defined in [`commands.rs`](../src-tauri/src/commands.rs).

| Field | Type | Description |
|---|---|---|
| `tmdb_id` | `number` | |
| `title` | `string` | |
| `year` | `number \| null` | |
| `poster_url` | `string \| null` | A remote `https://image.tmdb.org` URL, not a cached path. |

### Opened

Returned by `player_open`. Defined in [`player/mod.rs`](../src-tauri/src/player/mod.rs).

| Field | Type | Description |
|---|---|---|
| `embedded` | `boolean` | `true` when video is drawn inside the main window; `false` when it plays in the separate fallback window. |
| `item` | [`PlaybackItem`](#playbackitem) | |

### PlaybackItem

| Field | Type | Description |
|---|---|---|
| `file_id` | `number` | |
| `title_id` | `number` | |
| `title` | `string` | |
| `season` | `number \| null` | |
| `episode` | `number \| null` | |
| `episode_name` | `string \| null` | |
| `next` | [`NextEpisode`](#nextepisode) `\| null` | The following episode in season order, if any. |

### NextEpisode

| Field | Type |
|---|---|
| `file_id` | `number` |
| `season` | `number` |
| `episode` | `number` |
| `episode_name` | `string \| null` |

### Track

Defined in [`player/model.rs`](../src-tauri/src/player/model.rs).

| Field | Type | Description |
|---|---|---|
| `id` | `number` | Pass to `player_set_audio` or `player_set_subtitle`. |
| `title` | `string \| null` | |
| `lang` | `string \| null` | Language code, such as `eng`. |
| `selected` | `boolean` | |

## Events

| Event | Payload | When |
|---|---|---|
| `scan://progress` | `{ found: number, done: boolean, error: string \| null }` | During a scan, and once with `done: true` at the end. |
| `library://updated` | none | The library changed. Reload whatever is on screen. |
| `player://state` | `{ state: "playing" \| "paused" \| "buffering", volume: number, muted: boolean }` | When any of these values changes. |
| `player://time` | `{ position: number, duration: number \| null }` | While playing, at most every 250 ms. |
| `player://tracks` | `{ audio: Track[], subtitles: Track[] }` | When the track list or selection changes. See [`Track`](#track). |
| `player://ended` | `{ file_id: number }` | At the end of a file. The last frame stays on screen. |
| `player://error` | `{ message: string }` | A playback error, as a user-facing message. |
| `player://closed` | `{ file_id: number }` | Playback was closed from the native side (the fallback window). |
| `player://remote` | `{ action: "next" }` | A media key or Control Center asked for the next episode. Play, pause and seeking are handled in Rust. |
| `onboarding://drop` | `{ over: boolean, path: string \| null, error: string \| null }` | Only before onboarding is finished: something is dragged over the window (`over`), or a dropped folder became the media folder (`path`) and is being scanned. |
| `menu://command` | `string` | A menu bar item was chosen; the payload is its id (see `src-tauri/src/menu.rs`). |

## Embedded playback in the UI

Popcorn draws video in a native view **underneath** the web view, so the UI must make room for it:

- While the embedded player is open, the UI adds `class="playing"` to `<html>`. `html`, `body`
  and everything except the player controls must then be transparent or hidden.
- When `Opened.embedded` is `false`, video plays in a separate window. The UI shows a simple
  "Playing in a separate window" screen with a **Stop** button.

## Adding a command

1. Write the function with `#[tauri::command(async)]` in
   [`commands.rs`](../src-tauri/src/commands.rs) or
   [`player/commands.rs`](../src-tauri/src/player/commands.rs). Return `AppResult<T>`.
2. Register it in `generate_handler!` in [`lib.rs`](../src-tauri/src/lib.rs).
3. Add its name to `COMMANDS` in [`build.rs`](../src-tauri/build.rs).
4. Grant `allow-<command-name>` in
   [`capabilities/default.json`](../src-tauri/capabilities/default.json). Without this, calls from
   the UI are rejected.
5. Add a wrapper in [`ui/js/api.js`](../ui/js/api.js) and document it here.
