//! The macOS menu bar.
//!
//! Standard items (Edit, Window, Hide, Quit…) are AppKit's own. Popcorn's items
//! are sent to the UI as `menu://command` with the item's id, so a menu click
//! does exactly what the same action does in the page.
//!
//! The Playback menu is only enabled while a video is open. Its key
//! equivalents all use Command: the web view gets the first chance at those,
//! so they never take a key from a text field.

use std::sync::Mutex;

use tauri::menu::{
    AboutMetadata, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use tauri::{AppHandle, Emitter, Manager, Runtime, Wry};

use crate::state::lock;

/// The items that change with playback.
pub struct AppMenu {
    playback: Vec<MenuItem<Wry>>,
    next_episode: MenuItem<Wry>,
    state: Mutex<(bool, bool)>,
}

/// Popcorn's own items: id, title, key equivalent.
const NAVIGATION: &[(&str, &str, Option<&str>)] = &[
    ("home", "Home", Some("CmdOrCtrl+1")),
    ("movies", "Movies", Some("CmdOrCtrl+2")),
    ("shows", "TV Shows", Some("CmdOrCtrl+3")),
];

const PLAYBACK: &[&[(&str, &str, Option<&str>)]] = &[
    &[("play_pause", "Play/Pause", None)],
    &[
        ("skip_back", "Skip Back 10 Seconds", None),
        ("skip_forward", "Skip Forward 10 Seconds", None),
    ],
    &[
        ("volume_up", "Volume Up", Some("CmdOrCtrl+Up")),
        ("volume_down", "Volume Down", Some("CmdOrCtrl+Down")),
        ("mute", "Mute", Some("Alt+CmdOrCtrl+Down")),
    ],
    &[
        ("next_subtitle", "Next Subtitle Track", None),
        ("next_audio", "Next Audio Track", None),
        ("load_subtitle", "Load Subtitle File…", None),
    ],
    &[("close_player", "Close Player", None)],
];

/// Opened in the browser rather than sent to the UI.
const HELP_LINKS: &[(&str, &str, &str)] = &[
    (
        "help_website",
        "Popcorn on GitHub",
        "https://github.com/dizzpy/popcorn",
    ),
    (
        "help_issue",
        "Report an Issue…",
        "https://github.com/dizzpy/popcorn/issues/new/choose",
    ),
];

fn item<R: Runtime>(
    app: &AppHandle<R>,
    (id, text, keys): (&str, &str, Option<&str>),
    enabled: bool,
) -> tauri::Result<MenuItem<R>> {
    MenuItem::with_id(app, id, text, enabled, keys)
}

fn items<'a, R: Runtime>(list: &'a [&'a dyn IsMenuItem<R>]) -> &'a [&'a dyn IsMenuItem<R>] {
    list
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let separator = || PredefinedMenuItem::separator(app);
    let info = app.package_info();

    let about = PredefinedMenuItem::about(
        app,
        Some("About Popcorn"),
        Some(AboutMetadata {
            name: Some(info.name.clone()),
            version: Some(info.version.to_string()),
            copyright: Some("Free software under the GPL-3.0 licence.".into()),
            ..Default::default()
        }),
    )?;
    let settings = item(app, ("settings", "Settings…", Some("CmdOrCtrl+,")), true)?;
    let app_menu = Submenu::with_items(
        app,
        &info.name,
        true,
        items(&[
            &about,
            &separator()?,
            &settings,
            &separator()?,
            &PredefinedMenuItem::services(app, None)?,
            &separator()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &separator()?,
            &PredefinedMenuItem::quit(app, None)?,
        ]),
    )?;

    let rescan = item(app, ("rescan", "Rescan Library", Some("CmdOrCtrl+R")), true)?;
    let file_menu = Submenu::with_items(
        app,
        "File",
        true,
        items(&[
            &rescan,
            &separator()?,
            &PredefinedMenuItem::close_window(app, None)?,
        ]),
    )?;

    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        items(&[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &separator()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ]),
    )?;

    let navigation = NAVIGATION
        .iter()
        .map(|&entry| item(app, entry, true))
        .collect::<tauri::Result<Vec<_>>>()?;
    let search = item(app, ("search", "Search…", Some("CmdOrCtrl+F")), true)?;
    let back = item(app, ("back", "Back", Some("CmdOrCtrl+[")), true)?;
    let theme = item(
        app,
        ("theme", "Switch Theme", Some("Shift+CmdOrCtrl+L")),
        true,
    )?;
    let mut view_items: Vec<&dyn IsMenuItem<Wry>> = navigation
        .iter()
        .map(|i| i as &dyn IsMenuItem<Wry>)
        .collect();
    let view_separators = [separator()?, separator()?, separator()?];
    view_items.extend([
        &view_separators[0] as &dyn IsMenuItem<Wry>,
        &search,
        &back,
        &view_separators[1],
        &theme,
        &view_separators[2],
    ]);
    let fullscreen = PredefinedMenuItem::fullscreen(app, None)?;
    view_items.push(&fullscreen);
    let view_menu = Submenu::with_items(app, "View", true, &view_items)?;

    let mut playback = Vec::new();
    let mut playback_entries: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
    let next_episode = item(app, ("next_episode", "Next Episode", None), false)?;
    for (index, group) in PLAYBACK.iter().enumerate() {
        if index > 0 {
            playback_entries.push(Box::new(separator()?));
        }
        for &entry in *group {
            let menu_item = item(app, entry, false)?;
            playback.push(menu_item.clone());
            playback_entries.push(Box::new(menu_item));
        }
        if index == 1 {
            playback_entries.push(Box::new(next_episode.clone()));
        }
    }
    let playback_refs: Vec<&dyn IsMenuItem<Wry>> = playback_entries
        .iter()
        .map(|entry| entry.as_ref())
        .collect();
    let playback_menu = Submenu::with_items(app, "Playback", true, &playback_refs)?;

    let window_menu = Submenu::with_items(
        app,
        "Window",
        true,
        items(&[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, Some("Zoom"))?,
            &separator()?,
            &PredefinedMenuItem::bring_all_to_front(app, None)?,
        ]),
    )?;

    let help_items = HELP_LINKS
        .iter()
        .map(|&(id, text, _)| item(app, (id, text, None), true))
        .collect::<tauri::Result<Vec<_>>>()?;
    let help_refs: Vec<&dyn IsMenuItem<Wry>> = help_items
        .iter()
        .map(|i| i as &dyn IsMenuItem<Wry>)
        .collect();
    let help_menu = Submenu::with_items(app, "Help", true, &help_refs)?;

    let menu = Menu::with_items(
        app,
        items(&[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &playback_menu,
            &window_menu,
            &help_menu,
        ]),
    )?;
    app.set_menu(menu)?;
    // AppKit then adds its window list and the Help search field.
    window_menu.set_as_windows_menu_for_nsapp()?;
    help_menu.set_as_help_menu_for_nsapp()?;

    app.manage(AppMenu {
        playback,
        next_episode,
        state: Mutex::new((false, false)),
    });
    app.on_menu_event(on_event);
    Ok(())
}

fn on_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    if let Some((_, _, url)) = HELP_LINKS.iter().find(|(link, _, _)| *link == id) {
        #[cfg(target_os = "macos")]
        if let Err(error) = std::process::Command::new("/usr/bin/open")
            .arg(url)
            .status()
        {
            log::warn!("could not open {url}: {error}");
        }
        return;
    }
    if let Err(error) = app.emit("menu://command", id) {
        log::warn!("could not send the menu command {id}: {error}");
    }
}

/// Turns the Playback menu on or off. Cheap to call when nothing changes.
pub fn set_playing(app: &AppHandle, playing: bool, has_next: bool) {
    let Some(menu) = app.try_state::<AppMenu>() else {
        return;
    };
    let wanted = (playing, playing && has_next);
    {
        let mut state = lock(&menu.state);
        if *state == wanted {
            return;
        }
        *state = wanted;
    }
    let results = menu
        .playback
        .iter()
        .map(|item| item.set_enabled(wanted.0))
        .chain([menu.next_episode.set_enabled(wanted.1)]);
    for result in results {
        if let Err(error) = result {
            log::debug!("could not update the Playback menu: {error}");
        }
    }
}
