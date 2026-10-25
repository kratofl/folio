//! Folio's core: everything that isn't UI. Orchestration (`culling`,
//! `filter`, `histogram`, `layout`, `library`, `load_queue`, `outings`,
//! `text`, `zoom`) is pure.
//! File access (`open`, `conflicts`, `scan`, `preview`, `info`, `exif`, `video`,
//! `thumbnail`, `thumbnail_cache`, `dirs`, `drafts`, `moves`, `recent`,
//! `settings`, `views`, `xmp`) and the `editor` launcher sit at the edges.

pub mod conflicts;
pub mod culling;
pub mod dirs;
pub mod drafts;
pub mod editor;
pub mod exif;
pub mod filter;
pub mod histogram;
pub mod info;
pub mod layout;
pub mod library;
pub mod load_queue;
pub mod moves;
pub mod open;
pub mod outings;
pub mod preview;
pub mod recent;
pub mod scan;
pub mod settings;
pub mod text;
pub mod thumbnail;
pub mod thumbnail_cache;
pub mod video;
pub mod views;
pub mod xmp;
pub mod zoom;
mod tiff;
