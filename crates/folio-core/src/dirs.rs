//! Where per-machine data lives: the OS cache folder for things Folio can
//! rebuild (thumbnails), and the app data folder for settings (recent
//! libraries). Never inside a library, so file sync never sees them.

use std::env;
use std::path::PathBuf;

/// `~/Library/Caches/Folio` on macOS, `%LOCALAPPDATA%\Folio\Cache` on Windows,
/// `~/.cache/folio` elsewhere.
pub fn cache_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Caches/Folio"))
    } else if cfg!(windows) {
        env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Folio").join("Cache"))
    } else {
        env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".cache"))).map(|d| d.join("folio"))
    }
}

/// `~/Library/Application Support/Folio` on macOS, `%APPDATA%\Folio` on
/// Windows, `~/.config/folio` elsewhere.
pub fn data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Application Support/Folio"))
    } else if cfg!(windows) {
        env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Folio"))
    } else {
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".config"))).map(|d| d.join("folio"))
    }
}

fn home() -> Option<PathBuf> {
    env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from)
}
