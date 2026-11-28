# Changelog

All notable changes to Popcorn are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-11-28

### Added

- README with screenshots, installation steps, a usage guide and keyboard shortcuts.
- Contributing guide, security policy, code of conduct, and issue and pull request templates.
- Full reference of commands, events and data types in `docs/API.md`.
- Rust crate licenses in `docs/THIRD_PARTY.md`.
- **Media keys and Now Playing:** the play/pause key, headphone controls and the Control Center
  widget control playback, which shows the title, episode, poster and progress.
- **Menu bar** with Settings, Rescan Library, navigation, a Playback menu, theme switching, and the
  standard Edit and Window menus.

### Changed

- The player's controls and title now hide when the mouse rests, also while paused, and at once
  when the mouse leaves the window. Subtitles move up while the controls are shown.
- The DMG opens a styled install window: drag Popcorn onto Applications, with first-launch help.
- A simpler welcome screen: choose or drop the media folder, watch the scan, and add a TMDB key
  only if you want to.
- On Home, the featured text no longer runs into the film strip in smaller windows.
- On Home, a long title with a colon shows its subtitle on a smaller line underneath, and long
  titles get a smaller size instead of being cut off. Turn it off in Settings > Appearance.
- Settings are grouped into cards, with a switch for the split titles.

## [0.1.0] - 2026-11-26

First public release.

### Added

- **Library:** pick a media folder; Popcorn finds movies and episodes, groups episodes into shows
  and seasons, and watches the folder for changes. Files are never copied, moved or renamed.
- **File name parsing** for common naming styles, including release names, `S01E02`, `1x02`
  and `Season N` folders.
- **TMDB metadata:** posters, backdrops, ratings, genres, overviews, cast, directors and episode
  details, cached on disk so the library works offline. Accepts an API key or a read access
  token.
- **Fix match** to link a title to the correct TMDB entry, and refreshing of metadata.
- **Home screen** with a featured title and a Continue watching row; Movies and TV Shows
  screens with sorting and genre and watched filters.
- **Search** across titles, cast and episode names (<kbd>⌘</kbd> <kbd>K</kbd>).
- **Built-in player** based on libmpv, embedded in the main window, with a fallback window. Audio
  track and subtitle selection, external subtitle files, fullscreen, and keyboard shortcuts.
- **Watch progress:** resume where you left off, play the next episode, and automatic or manual
  watched state.
- **Dark and light themes.**
- Slim libmpv build from pinned, checksummed sources; a DMG under 50 MB for Apple Silicon and
  macOS 13 or newer; and a GitHub Actions release workflow.

[Unreleased]: https://github.com/dizzpy/Popcorn/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/dizzpy/Popcorn/compare/v0.1.0...v0.2.1
[0.1.0]: https://github.com/dizzpy/Popcorn/releases/tag/v0.1.0
