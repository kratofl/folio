//! Times the header pass (photo shapes) and thumbnail extraction for every
//! photo in a folder, on one thread.
//! Usage: `cargo run --release -p folio-core --example thumbnail_speed -- <folder>`

use std::time::Instant;

use folio_core::info::read_info;
use folio_core::scan::scan;
use folio_core::thumbnail::load_thumbnail;

fn main() {
    let Some(root) = std::env::args_os().nth(1) else {
        eprintln!("usage: thumbnail_speed <folder>");
        return;
    };
    let photos = match scan(root.as_ref()) {
        Ok(found) => found.photos,
        Err(error) => return eprintln!("scan failed: {error}"),
    };

    let start = Instant::now();
    let infos: Vec<_> = photos.iter().map(|p| read_info(&p.path)).collect();
    let no_size = infos.iter().filter(|i| i.size.is_none()).count();
    let no_time = infos.iter().filter(|i| i.exif.captured.is_none()).count();
    println!("headers: {:.2?}, {no_size} without a size, {no_time} without a capture time", start.elapsed());
    if let Some(info) = infos.first() {
        println!("first photo: {:?}", info.exif);
    }

    let start = Instant::now();
    let mut failed = 0;
    for photo in &photos {
        if let Err(error) = load_thumbnail(&photo.path, 336) {
            failed += 1;
            eprintln!("{}: {error}", photo.path.display());
        }
    }
    let elapsed = start.elapsed();
    let count = photos.len().max(1) as u32;
    println!("{} photos, {failed} failed, {elapsed:.2?} total, {:.1?} each", photos.len(), elapsed / count);
}
