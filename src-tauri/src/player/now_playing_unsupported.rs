//! Now Playing is macOS only; elsewhere these do nothing.

use tauri::AppHandle;

pub struct Info {
    pub file_id: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub poster: Option<String>,
    pub has_next: bool,
}

pub fn install(_: &AppHandle) {}
pub fn begin(_: Info) {}
pub fn update(_: i64, _: f64, _: Option<f64>, _: bool) {}
pub fn end(_: i64) {}
