//! Now Playing and the media keys, through the MediaPlayer framework.
//!
//! The play/pause key, AirPods and headphone controls, and the Now Playing
//! widget in Control Center all reach the app through
//! `MPRemoteCommandCenter`. The widget shows what `MPNowPlayingInfoCenter`
//! was last told: it extrapolates the position from the rate, so it only
//! needs an update when playback starts, stops or jumps.
//!
//! Every AppKit/MediaPlayer call happens on the main thread. Updates are
//! queued there and never waited for, so callers on any thread are safe.

use std::ffi::{c_void, CStr, CString};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::objc::{self, msg, Id, Method, Sel, NIL};
use super::PlayerHost;
use crate::state::lock;

#[link(name = "MediaPlayer", kind = "framework")]
extern "C" {
    static MPMediaItemPropertyTitle: Id;
    static MPMediaItemPropertyArtist: Id;
    static MPMediaItemPropertyPlaybackDuration: Id;
    static MPMediaItemPropertyArtwork: Id;
    static MPNowPlayingInfoPropertyElapsedPlaybackTime: Id;
    static MPNowPlayingInfoPropertyPlaybackRate: Id;
    static MPNowPlayingInfoPropertyDefaultPlaybackRate: Id;
    static MPNowPlayingInfoPropertyMediaType: Id;
}

extern "C" {
    static _NSConcreteGlobalBlock: c_void;
}

/// `MPRemoteCommandHandlerStatus`
const HANDLED: isize = 0;
const NOTHING_PLAYING: isize = 110;
/// `MPNowPlayingPlaybackState`
const STATE_PLAYING: usize = 1;
const STATE_PAUSED: usize = 2;
const STATE_STOPPED: usize = 3;
/// `MPNowPlayingInfoMediaTypeVideo`
const MEDIA_TYPE_VIDEO: usize = 2;
const SKIP_SECONDS: f64 = 10.0;

/// What the widget shows for one file.
pub struct Info {
    pub file_id: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub poster: Option<String>,
    pub has_next: bool,
}

#[derive(Clone, Copy)]
struct Progress {
    position: f64,
    duration: Option<f64>,
    playing: bool,
}

struct Current {
    info: Info,
    progress: Option<Progress>,
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static CURRENT: Mutex<Option<Current>> = Mutex::new(None);
/// The poster as a retained `NSImage`, which the artwork block hands out.
static ARTWORK: Mutex<usize> = Mutex::new(0);

#[derive(Clone, Serialize)]
struct RemotePayload {
    action: &'static str,
}

// ---- Remote commands ------------------------------------------------------

fn with_session(run: impl FnOnce(&super::session::Session)) -> isize {
    let Some(app) = APP.get() else {
        return NOTHING_PLAYING;
    };
    match app.state::<PlayerHost>().session() {
        Some(session) if !session.is_stopped() => {
            run(&session);
            HANDLED
        }
        _ => NOTHING_PLAYING,
    }
}

/// Asynchronous, so the main thread never waits for the video engine.
fn send(session: &super::session::Session, args: &[&str]) {
    if let Err(error) = session.mpv.command_async(args) {
        log::debug!("remote command not delivered: {error}");
    }
}

extern "C" fn on_play(_: Id, _: Sel, _: Id) -> isize {
    with_session(|s| send(s, &["set", "pause", "no"]))
}

extern "C" fn on_pause(_: Id, _: Sel, _: Id) -> isize {
    with_session(|s| send(s, &["set", "pause", "yes"]))
}

extern "C" fn on_toggle(_: Id, _: Sel, _: Id) -> isize {
    with_session(|s| send(s, &["cycle", "pause"]))
}

extern "C" fn on_forward(_: Id, _: Sel, _: Id) -> isize {
    with_session(|s| send(s, &["seek", &SKIP_SECONDS.to_string(), "relative"]))
}

extern "C" fn on_backward(_: Id, _: Sel, _: Id) -> isize {
    with_session(|s| send(s, &["seek", &(-SKIP_SECONDS).to_string(), "relative"]))
}

extern "C" fn on_position(_: Id, _: Sel, event: Id) -> isize {
    // SAFETY: this target is only registered for the change-playback-position
    // command, whose events are `MPChangePlaybackPositionCommandEvent`s.
    let seconds = unsafe { msg!(event, c"positionTime", f64) };
    if !seconds.is_finite() {
        return HANDLED;
    }
    with_session(|s| send(s, &["seek", &format!("{seconds:.3}"), "absolute"]))
}

/// The UI knows the episode order and plays the next one itself.
extern "C" fn on_next(_: Id, _: Sel, _: Id) -> isize {
    let Some(app) = APP.get() else {
        return NOTHING_PLAYING;
    };
    if let Err(error) = app.emit("player://remote", RemotePayload { action: "next" }) {
        log::warn!("could not send the next-episode command: {error}");
    }
    HANDLED
}

type Handler = extern "C" fn(Id, Sel, Id) -> isize;

/// The command getters on `MPRemoteCommandCenter`, with our handlers.
const COMMANDS: [(&CStr, &CStr, Handler); 7] = [
    (c"playCommand", c"popcornPlay:", on_play),
    (c"pauseCommand", c"popcornPause:", on_pause),
    (c"togglePlayPauseCommand", c"popcornToggle:", on_toggle),
    (c"skipForwardCommand", c"popcornForward:", on_forward),
    (c"skipBackwardCommand", c"popcornBackward:", on_backward),
    (
        c"changePlaybackPositionCommand",
        c"popcornPosition:",
        on_position,
    ),
    (c"nextTrackCommand", c"popcornNext:", on_next),
];

fn command_center() -> Id {
    // SAFETY: a class method without arguments that returns the shared object.
    unsafe {
        msg!(
            objc::class(c"MPRemoteCommandCenter"),
            c"sharedCommandCenter",
            Id
        )
    }
}

fn info_center() -> Id {
    // SAFETY: as above.
    unsafe { msg!(objc::class(c"MPNowPlayingInfoCenter"), c"defaultCenter", Id) }
}

/// Registers the handlers, all disabled until something plays. Call once, on
/// the main thread.
pub fn install(app: &AppHandle) {
    if APP.set(app.clone()).is_err() {
        return;
    }
    let methods: Vec<Method> = COMMANDS
        .iter()
        .map(|&(_, name, imp)| Method {
            name,
            // Returns NSInteger, takes the command event.
            types: c"q@:@",
            imp: imp as *const c_void,
        })
        .collect();
    objc::autorelease_pool(|| {
        // SAFETY: every handler is `extern "C" fn(Id, Sel, Id) -> isize`,
        // matching "q@:@". The target is never released: the command center
        // keeps targets weakly, and this one lives as long as the app.
        unsafe {
            let class = objc::subclass(c"NSObject", c"PopcornRemoteTarget", &methods);
            if class.is_null() {
                log::warn!("media keys are not available");
                return;
            }
            let target = msg!(msg!(class, c"alloc", Id), c"init", Id);
            let center = command_center();
            for (getter, action, _) in COMMANDS {
                let command = msg!(center, getter, Id);
                msg!(command, c"addTarget:action:", (); target => Id, objc::sel(action) => Sel);
                msg!(command, c"setEnabled:", (); false => bool);
            }
            let interval =
                msg!(objc::class(c"NSNumber"), c"numberWithDouble:", Id; SKIP_SECONDS => f64);
            let intervals = msg!(objc::class(c"NSArray"), c"arrayWithObject:", Id; interval => Id);
            for getter in [c"skipForwardCommand", c"skipBackwardCommand"] {
                let command = msg!(center, getter, Id);
                msg!(command, c"setPreferredIntervals:", (); intervals => Id);
            }
        }
    });
}

fn on_main(run: impl FnOnce() + Send + 'static) {
    let Some(app) = APP.get() else {
        return;
    };
    if let Err(error) = app.run_on_main_thread(move || objc::autorelease_pool(run)) {
        log::debug!("now playing not updated: {error}");
    }
}

/// A new file is open.
pub fn begin(info: Info) {
    let has_next = info.has_next;
    let poster = info.poster.clone();
    *lock(&CURRENT) = Some(Current {
        info,
        progress: None,
    });
    on_main(move || {
        set_artwork(poster.as_deref());
        // SAFETY: main thread; the getters return the shared command objects.
        unsafe {
            let center = command_center();
            for (getter, _, _) in COMMANDS {
                let enabled = getter != c"nextTrackCommand" || has_next;
                msg!(msg!(center, getter, Id), c"setEnabled:", (); enabled => bool);
            }
        }
        publish();
    });
}

/// Playback started, stopped or jumped.
pub fn update(file_id: i64, position: f64, duration: Option<f64>, playing: bool) {
    {
        let mut current = lock(&CURRENT);
        let Some(current) = current.as_mut().filter(|c| c.info.file_id == file_id) else {
            return;
        };
        current.progress = Some(Progress {
            position,
            duration,
            playing,
        });
    }
    on_main(publish);
}

/// The file closed. Ignored if another file is already playing.
pub fn end(file_id: i64) {
    {
        let mut current = lock(&CURRENT);
        if current.as_ref().is_none_or(|c| c.info.file_id != file_id) {
            return;
        }
        *current = None;
    }
    on_main(|| {
        // Nothing else started in the meantime.
        if lock(&CURRENT).is_some() {
            return;
        }
        set_artwork(None);
        // SAFETY: main thread; nil clears the widget.
        unsafe {
            let center = command_center();
            for (getter, _, _) in COMMANDS {
                msg!(msg!(center, getter, Id), c"setEnabled:", (); false => bool);
            }
            let info = info_center();
            msg!(info, c"setNowPlayingInfo:", (); NIL => Id);
            msg!(info, c"setPlaybackState:", (); STATE_STOPPED => usize);
        }
    });
}

// ---- Publishing -----------------------------------------------------------

fn ns_string(text: &str) -> Option<Id> {
    CString::new(text).ok().map(|text| objc::ns_string(&text))
}

fn number(value: f64) -> Id {
    // SAFETY: returns an autoreleased NSNumber.
    unsafe { msg!(objc::class(c"NSNumber"), c"numberWithDouble:", Id; value => f64) }
}

/// Sends the current state to the widget. Main thread only.
fn publish() {
    let current = lock(&CURRENT);
    let Some(current) = current.as_ref() else {
        return;
    };
    let progress = current.progress;
    // SAFETY: main thread, inside an autorelease pool. Keys are MediaPlayer's
    // constants; values are the types the documentation lists for each key.
    unsafe {
        let dictionary = msg!(objc::class(c"NSMutableDictionary"), c"dictionary", Id);
        let put = |key: Id, value: Id| {
            if !value.is_null() {
                msg!(dictionary, c"setObject:forKey:", (); value => Id, key => Id);
            }
        };
        let (title, artist) = match &current.info.subtitle {
            // An episode: its name first, the show underneath.
            Some(episode) => (episode.as_str(), Some(current.info.title.as_str())),
            None => (current.info.title.as_str(), None),
        };
        put(MPMediaItemPropertyTitle, ns_string(title).unwrap_or(NIL));
        if let Some(artist) = artist.and_then(ns_string) {
            put(MPMediaItemPropertyArtist, artist);
        }
        let media_type = msg!(objc::class(c"NSNumber"), c"numberWithUnsignedInteger:", Id; MEDIA_TYPE_VIDEO => usize);
        put(MPNowPlayingInfoPropertyMediaType, media_type);
        put(MPNowPlayingInfoPropertyDefaultPlaybackRate, number(1.0));
        if let Some(progress) = progress {
            put(
                MPNowPlayingInfoPropertyElapsedPlaybackTime,
                number(progress.position.max(0.0)),
            );
            put(
                MPNowPlayingInfoPropertyPlaybackRate,
                number(if progress.playing { 1.0 } else { 0.0 }),
            );
            if let Some(duration) = progress.duration.filter(|d| d.is_finite() && *d > 0.0) {
                put(MPMediaItemPropertyPlaybackDuration, number(duration));
            }
        }
        let artwork = artwork_object();
        put(MPMediaItemPropertyArtwork, artwork);

        let info = info_center();
        msg!(info, c"setNowPlayingInfo:", (); dictionary => Id);
        let state = match progress {
            Some(p) if p.playing => STATE_PLAYING,
            _ => STATE_PAUSED,
        };
        msg!(info, c"setPlaybackState:", (); state => usize);
    }
}

// ---- Artwork ----------------------------------------------------------------
//
// `MPMediaItemArtwork` asks for its image through a block. A global block
// (no captures) is a plain static struct, so no blocks runtime is needed: it
// hands out whatever image `ARTWORK` holds.

#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    width: f64,
    height: f64,
}

#[repr(C)]
struct BlockDescriptor {
    reserved: usize,
    size: usize,
    signature: *const std::ffi::c_char,
}

#[repr(C)]
struct Block {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: extern "C" fn(*const Block, Size) -> Id,
    descriptor: *const BlockDescriptor,
}

// SAFETY: both are immutable after construction and only hold addresses of
// statics and functions.
unsafe impl Sync for Block {}
unsafe impl Sync for BlockDescriptor {}

const BLOCK_IS_GLOBAL: i32 = 1 << 28;
const BLOCK_HAS_SIGNATURE: i32 = 1 << 30;

static DESCRIPTOR: BlockDescriptor = BlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<Block>(),
    // Returns an object; takes the block and a CGSize.
    signature: c"@24@?0{CGSize=dd}8".as_ptr(),
};

static ARTWORK_BLOCK: Block = Block {
    // Only the address of the runtime's class object is taken.
    isa: &raw const _NSConcreteGlobalBlock,
    flags: BLOCK_IS_GLOBAL | BLOCK_HAS_SIGNATURE,
    reserved: 0,
    invoke: artwork_for_size,
    descriptor: &DESCRIPTOR,
};

extern "C" fn artwork_for_size(_: *const Block, _: Size) -> Id {
    // The image stays retained in ARTWORK until it is replaced on the main
    // thread; MediaPlayer retains what it is given.
    *lock(&ARTWORK) as Id
}

/// Loads the poster into `ARTWORK`, releasing the previous one. Main thread.
fn set_artwork(path: Option<&str>) {
    let image = path
        .and_then(ns_string)
        .map(|path| {
            // SAFETY: alloc/init returns +1, which ARTWORK now owns. A file
            // that cannot be read gives nil.
            unsafe {
                let image = msg!(objc::class(c"NSImage"), c"alloc", Id);
                msg!(image, c"initWithContentsOfFile:", Id; path => Id)
            }
        })
        .unwrap_or(NIL);
    let old = std::mem::replace(&mut *lock(&ARTWORK), image as usize) as Id;
    if !old.is_null() {
        // SAFETY: the +1 reference taken above for the previous image.
        unsafe { msg!(old, c"release", ()) };
    }
}

/// An autoreleased `MPMediaItemArtwork` for the current image, or nil.
fn artwork_object() -> Id {
    let image = *lock(&ARTWORK) as Id;
    if image.is_null() {
        return NIL;
    }
    // SAFETY: main thread; `size` is an NSSize (two doubles), and the block
    // is a valid global block with the signature MediaPlayer expects.
    unsafe {
        let size = msg!(image, c"size", Size);
        let artwork = msg!(objc::class(c"MPMediaItemArtwork"), c"alloc", Id);
        let artwork = msg!(
            artwork,
            c"initWithBoundsSize:requestHandler:",
            Id;
            size => Size,
            (&raw const ARTWORK_BLOCK).cast::<c_void>() => *const c_void
        );
        msg!(artwork, c"autorelease", Id)
    }
}

/// What macOS currently shows, for the smoke run. Main thread.
#[cfg(debug_assertions)]
pub fn describe() -> String {
    objc::autorelease_pool(|| {
        // SAFETY: main thread; reads the shared info center.
        unsafe {
            let info = info_center();
            let state = msg!(info, c"playbackState", usize);
            let dictionary = msg!(info, c"nowPlayingInfo", Id);
            if dictionary.is_null() {
                return format!("state={state} info=none");
            }
            let read = |key: Id| {
                let value = msg!(dictionary, c"objectForKey:", Id; key => Id);
                if value.is_null() {
                    return "-".to_owned();
                }
                let text = msg!(value, c"description", Id);
                let utf8 = msg!(text, c"UTF8String", *const std::ffi::c_char);
                std::ffi::CStr::from_ptr(utf8)
                    .to_string_lossy()
                    .into_owned()
            };
            format!(
                "state={state} title={} artist={} elapsed={} rate={} duration={} artwork={}",
                read(MPMediaItemPropertyTitle),
                read(MPMediaItemPropertyArtist),
                read(MPNowPlayingInfoPropertyElapsedPlaybackTime),
                read(MPNowPlayingInfoPropertyPlaybackRate),
                read(MPMediaItemPropertyPlaybackDuration),
                read(MPMediaItemPropertyArtwork) != "-",
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hand-made block must be callable the way MediaPlayer calls it.
    #[test]
    fn artwork_block_hands_out_the_poster() {
        let icon = concat!(env!("CARGO_MANIFEST_DIR"), "/icons/128x128.png");
        objc::autorelease_pool(|| {
            set_artwork(Some(icon));
            let image = *lock(&ARTWORK) as Id;
            assert!(!image.is_null(), "the poster did not load");
            let artwork = artwork_object();
            assert!(!artwork.is_null());
            // SAFETY: `imageWithSize:` calls our block and returns its image.
            let given = unsafe {
                msg!(artwork, c"imageWithSize:", Id; Size { width: 64.0, height: 64.0 } => Size)
            };
            assert_eq!(given, image);
            set_artwork(None);
            assert_eq!(*lock(&ARTWORK), 0);
        });
    }
}
