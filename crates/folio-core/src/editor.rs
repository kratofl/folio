//! Hands a photo to Affinity for editing. Edge code: it starts another app
//! and nothing more. Folio never waits for the editor or reads what it does.

use std::io;
use std::path::Path;
use std::process::Command;

/// Opens `photo` in Affinity: the 2025 all-in-one app if it's installed,
/// otherwise Affinity Photo 2 or 1. Blocks for a moment, so call it off the
/// UI thread.
#[cfg(target_os = "macos")]
pub fn open_in_affinity(photo: &Path) -> io::Result<()> {
    for app in ["Affinity", "Affinity Photo 2", "Affinity Photo"] {
        // `open -a` fails when there's no app by that name, so try the next.
        if Command::new("open").arg("-a").arg(app).arg(photo).output()?.status.success() {
            return Ok(());
        }
    }
    Err(not_installed())
}

/// Windows: the same apps, found in their default install folders. Untested.
#[cfg(target_os = "windows")]
pub fn open_in_affinity(photo: &Path) -> io::Result<()> {
    let program_files = std::env::var_os("ProgramFiles").ok_or_else(not_installed)?;
    let candidates = [r"Affinity\Affinity\Affinity.exe", r"Affinity\Photo 2\Photo.exe", r"Affinity\Photo\Photo.exe"];
    let app = candidates.iter().map(|c| Path::new(&program_files).join(c)).find(|p| p.is_file()).ok_or_else(not_installed)?;
    Command::new(app).arg(photo).spawn().map(drop)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn open_in_affinity(_photo: &Path) -> io::Result<()> {
    Err(not_installed())
}

fn not_installed() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "Affinity isn't installed")
}
