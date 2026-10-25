//! Recently opened libraries, for the welcome screen. They're per machine,
//! so the list lives in the app-data folder (see `dirs`), never in a
//! library. One absolute path per line, newest first.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// How many libraries the list remembers.
const KEPT: usize = 8;
const FILE: &str = "recent.txt";

/// The remembered libraries that still exist, newest first.
pub fn load_recent(data_dir: &Path) -> Vec<PathBuf> {
    let text = fs::read_to_string(data_dir.join(FILE)).unwrap_or_default();
    text.lines().filter(|l| !l.is_empty()).map(PathBuf::from).filter(|p| p.is_dir()).collect()
}

/// Puts `library` at the top of the list.
pub fn remember(data_dir: &Path, library: &Path) -> io::Result<()> {
    let text = fs::read_to_string(data_dir.join(FILE)).unwrap_or_default();
    let list = with_first(text.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect(), library);
    let lines: Vec<&str> = list.iter().filter_map(|p| p.to_str()).collect();
    fs::create_dir_all(data_dir)?;
    let temporary = data_dir.join(format!(".{FILE}.folio-tmp"));
    fs::write(&temporary, lines.join("\n") + "\n")?;
    fs::rename(&temporary, data_dir.join(FILE))
}

/// `list` with `library` moved or added to the front, at most `KEPT` long.
fn with_first(mut list: Vec<PathBuf>, library: &Path) -> Vec<PathBuf> {
    list.retain(|p| p != library);
    list.insert(0, library.to_path_buf());
    list.truncate(KEPT);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_moves_a_library_to_the_top_and_the_list_stays_short() {
        let paths: Vec<PathBuf> = (0..KEPT).map(|i| PathBuf::from(format!("/photos/{i}"))).collect();
        let list = with_first(paths.clone(), Path::new("/photos/3"));
        assert_eq!(list[0], Path::new("/photos/3"));
        assert_eq!(list.len(), KEPT, "no duplicate");

        let list = with_first(paths, Path::new("/photos/new"));
        assert_eq!(list.len(), KEPT);
        assert_eq!(list.last().map(PathBuf::as_path), Some(Path::new("/photos/6")), "the oldest falls off");
    }
}
