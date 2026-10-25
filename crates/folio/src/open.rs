//! Opening a library into a window, from the command line, the welcome
//! screen, a recent library or ⌘O. Reading the library happens off the UI
//! thread. Each library gets its own window.

use std::path::{Path, PathBuf};

use folio_core::culling::Culling;
use folio_core::drafts::load_drafts;
use folio_core::library::Library;
use folio_core::open::open_library;
use folio_core::outings::Draft;
use folio_core::settings::{Settings, load_settings};
use folio_core::views::{SavedView, load_views};
use gpui::{
    AnyWindowHandle, App, AsyncApp, Bounds, PathPromptOptions, SharedString, TitlebarOptions, WindowBounds, WindowOptions,
    point, prelude::*, px, size,
};

use crate::app::Folio;

/// Everything a library window starts from.
pub struct Loaded {
    pub library: Library,
    /// Proves the library has at least one photo.
    pub culling: Culling,
    pub views: Vec<SavedView>,
    pub drafts: Vec<Draft>,
    pub settings: Settings,
    /// Messages from opening the library that the user should see.
    pub problems: Vec<String>,
}

/// Reads a library and remembers it as recently opened. Blocking: run it
/// off the UI thread. The error is a sentence for the user.
pub fn load(root: &Path) -> Result<Loaded, String> {
    let opened = open_library(root).map_err(|error| format!("Couldn't open {}: {error}", root.display()))?;
    let library = opened.library;
    let Some(culling) = Culling::new(library.photos().iter().map(|p| p.marks).collect()) else {
        return Err(format!("There are no photos in {}", library.name()));
    };
    let mut problems = opened.problems;
    if opened.recovered > 0 {
        problems.insert(0, format!("Finished {} that a crash interrupted", crate::app::count(opened.recovered, "move batch")));
    }
    if opened.skipped > 0 {
        problems.push(format!("Skipped {} whose names aren't valid Unicode", crate::app::count(opened.skipped, "photo")));
    }
    let (views, view_problems) = load_views(library.root());
    let (drafts, draft_problems) = load_drafts(library.root());
    let (settings, settings_problems) = load_settings(library.root());
    problems.extend(view_problems.into_iter().chain(draft_problems).chain(settings_problems));
    if let Some(data) = folio_core::dirs::data_dir()
        && let Err(error) = folio_core::recent::remember(&data, library.root())
    {
        eprintln!("can't remember {} as recent: {error}", library.root().display());
    }
    Ok(Loaded { library, culling, views, drafts, settings, problems })
}

/// Opens a window for a loaded library.
pub fn open_window(loaded: Loaded, cx: &mut App) -> Result<AnyWindowHandle, String> {
    let title: SharedString = format!("Folio: {}", loaded.library.name()).into();
    let opened = cx.open_window(window_options(title, size(px(1440.), px(900.)), cx), |window, cx| {
        let folio = cx.new(|cx| Folio::new(loaded, window, cx));
        #[cfg(feature = "screenshot")]
        crate::screenshot::capture_when_ready(folio.clone(), window, cx);
        folio
    });
    opened.map(Into::into).map_err(|error| format!("Couldn't open a window: {error}"))
}

/// Loads `root` in the background and opens its window.
pub async fn open(root: PathBuf, cx: &mut AsyncApp) -> Result<AnyWindowHandle, String> {
    let loaded = cx.background_spawn(async move { load(&root) }).await?;
    cx.update(|cx| open_window(loaded, cx))
}

/// Asks for a library folder. `None` when the user cancels.
pub async fn choose_folder(cx: &mut AsyncApp) -> Option<PathBuf> {
    let options = PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Open".into()) };
    let chosen = cx.update(|cx| cx.prompt_for_paths(options)).await.ok()?.ok()??;
    chosen.into_iter().next()
}

/// Window chrome shared by library windows and the welcome screen: the
/// titlebar is drawn by Folio, with the traffic lights inset to match.
pub fn window_options(title: SharedString, window_size: gpui::Size<gpui::Pixels>, cx: &App) -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(title),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(14.))),
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, window_size, cx))),
        ..Default::default()
    }
}
