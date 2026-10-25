//! The window's state and everything the user can do to it. Keys and clicks
//! land here and turn into calls on `folio_core`. Rendering lives in `grid`
//! (photos and headers), `chrome` (sidebar, dock, notices), `filter_bar`
//! (the ⌘K bar and the filter strip) and `sheet` (the move review).

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use folio_core::culling::{Culling, Flag, Intent, Label, Stars};
use folio_core::drafts;
use folio_core::filter::{self, Filter, Suggestion};
use folio_core::layout::{Direction, Layout, Metrics, Section};
use folio_core::library::{Library, new_sidecar};
use folio_core::load_queue::LoadQueue;
use folio_core::moves::{self, MovePlan};
use folio_core::outings::{Draft, Outing, SortIntent, SortPlan, gap_text, step_gap};
use folio_core::settings::{Settings, save_settings};
use folio_core::text::TextField;
use folio_core::thumbnail::load_thumbnail;
use folio_core::thumbnail_cache::ThumbnailCache;
use folio_core::views::{self, SavedView};
use folio_core::xmp;
use gpui::{
    Context, FocusHandle, KeyDownEvent, Keystroke, ListAlignment, ListOffset, ListState, RenderImage, Subscription, Task, Window,
    prelude::*, px,
};

use crate::inspector::{INSPECTOR_WIDTH, Inspector};
use crate::loupe::Loupe;
use crate::open::{self, Loaded};

/// Rows are filled until they'd get shorter than this, at first. Pinching
/// or ⌘= and ⌘- change it within `ROW_HEIGHTS`.
const ROW_HEIGHT: f32 = 220.;
/// Smallest and biggest rows. Thumbnails are decoded at `THUMB_PX`, which
/// stays sharp up to the biggest on a Retina screen.
const ROW_HEIGHTS: (f32, f32) = (90., 300.);
pub const GAP: f32 = 2.;
pub const SIDEBAR_WIDTH: f32 = 216.;
/// Long edge of a decoded thumbnail: tall enough for a row on Retina.
pub(crate) const THUMB_PX: u32 = 480;
const NOTICE_SECONDS: u64 = 5;

pub enum Thumb {
    Loading,
    Ready(Arc<RenderImage>),
    Failed,
}

/// What the grid shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    /// Every photo, by folder.
    Library,
    /// Photos still in date folders, grouped into outings to sort.
    Unsorted,
    /// One category's photos, by folder.
    Category(String),
}

/// What the keyboard is doing right now.
pub enum Mode {
    Browse,
    /// Typing a name or category for an outing. It starts as the old
    /// value, selected, so the first key replaces it.
    Typing { outing: usize, field: Field, text: TextField },
    /// The review sheet for moves about to run.
    Review(MovePlan),
    /// The ⌘K filter bar. `suggestions` are worked out as the text changes,
    /// never while drawing.
    Filtering { text: TextField, highlighted: usize, suggestions: Vec<BarItem> },
    /// Typing a name to save the current filter as a view.
    NamingView { text: TextField },
    /// One photo filling the window (see `loupe`).
    Loupe(Loupe),
}

/// A row in the ⌘K bar: a saved view to open, or a condition to add.
pub enum BarItem {
    /// Index into the saved views.
    View(usize),
    Condition(Suggestion),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    /// `then_accept` when Enter asked for a category before accepting.
    Category { then_accept: bool },
}

/// A short message that replaces the dock for a few seconds.
pub struct Notice {
    pub text: String,
    pub action: Option<NoticeAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeAction {
    Undo,
    Redo,
    /// Brings back a view that was just deleted.
    Restore(SavedView),
}

pub struct Folio {
    pub library: Library,
    pub plan: SortPlan,
    pub culling: Culling,
    pub view: View,
    /// Narrows whatever the view shows. Empty shows everything.
    pub filter: Filter,
    pub views: Vec<SavedView>,
    pub settings: Settings,
    /// How many photos each saved view shows, in `views` order.
    pub view_counts: Vec<usize>,
    pub sections: Vec<Section>,
    /// In the Unsorted view, the outing each section shows. Filtering can
    /// hide whole outings, so section and outing indices differ.
    pub section_outings: Vec<usize>,
    /// How many videos go with each outing, indexed like the plan's outings.
    pub outing_videos: Vec<usize>,
    /// The view's photos before filtering, which suggestions count against.
    view_photos: Vec<usize>,
    /// Photos in accepted outings, drawn faded in the Unsorted view.
    pub faded: Vec<bool>,
    pub layout: Layout,
    aspects: Vec<f32>,
    layout_width: f32,
    /// The rows' target height, which pinching changes.
    row_height: f32,
    pub thumbs: Vec<Thumb>,
    pub queue: LoadQueue,
    pub list: ListState,
    pub focus: FocusHandle,
    pub mode: Mode,
    pub inspector: Inspector,
    /// Thumbnails on disk, shared by the loading workers and the inspector.
    pub(crate) thumb_cache: Option<Arc<ThumbnailCache>>,
    /// Reads the inspector's details. Replacing it cancels the one before.
    pub(crate) details_load: Option<Task<()>>,
    pub notice: Option<Notice>,
    _notice_timer: Option<Task<()>>,
    /// Decodes the loupe's preview. Replacing it cancels the one before.
    pub(crate) loupe_load: Option<Task<()>>,
    /// The outing drafts as last read or written, so saving only touches
    /// files that changed.
    saved_drafts: Vec<Draft>,
    /// A titlebar press that may become a drag (see `chrome::titlebar`).
    drag_pending: bool,
    _workers: Vec<Task<()>>,
    _bounds: Subscription,
    _details: Subscription,
}

impl Folio {
    pub fn new(loaded: Loaded, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let Loaded { library, culling, views, drafts, settings, problems } = loaded;
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let mut plan = SortPlan::group(&library, settings.outing_gap_minutes * 60);
        plan.apply_drafts(&library, &drafts);
        // Open on the pile that needs work, if there is one.
        let view = if plan.outings().is_empty() { View::Library } else { View::Unsorted };
        let aspects: Vec<f32> = library.photos().iter().map(|p| p.aspect).collect();
        let count = aspects.len();
        let bounds = cx.observe_window_bounds(window, |this, window, cx| this.resize(window, cx));
        // Whatever changed, the inspector follows the selection.
        let details = cx.observe_self(|this, cx| this.load_details(cx));
        let thumb_cache = folio_core::dirs::cache_dir().map(|dir| Arc::new(ThumbnailCache::new(&dir)));

        let mut folio = Self {
            library,
            plan,
            culling,
            view,
            filter: Filter::default(),
            views,
            settings,
            view_counts: Vec::new(),
            sections: Vec::new(),
            section_outings: Vec::new(),
            outing_videos: Vec::new(),
            view_photos: Vec::new(),
            faded: vec![false; count],
            layout: Layout::new(&[], &aspects, metrics(1., ROW_HEIGHT)),
            row_height: ROW_HEIGHT,
            aspects,
            layout_width: grid_width(window, false),
            thumbs: (0..count).map(|_| Thumb::Loading).collect(),
            queue: LoadQueue::new(count),
            list: ListState::new(0, ListAlignment::Top, px(1200.)),
            focus,
            mode: Mode::Browse,
            inspector: Inspector::new(),
            details_load: None,
            notice: None,
            _notice_timer: None,
            loupe_load: None,
            saved_drafts: drafts,
            drag_pending: false,
            _workers: spawn_workers(thumb_cache.clone(), window, cx),
            thumb_cache,
            _bounds: bounds,
            _details: details,
        };
        folio.refresh();
        folio.count_views();
        // Drops drafts whose photos moved or vanished while Folio was closed.
        folio.save_drafts(cx);
        if let Some(first) = problems.first() {
            let more = if problems.len() > 1 { format!(" (and {} more)", problems.len() - 1) } else { String::new() };
            folio.tell(format!("{first}{more}"), None, cx);
        }
        folio
    }

    /// Where thumbnail loading centers: the position, in the view's reading
    /// order, of the first photo at the top of the scrolled list.
    fn load_focus(&self) -> usize {
        let top = self.list.logical_scroll_top().item_ix;
        self.layout.first_photo_from(top).and_then(|photo| self.layout.position(photo)).unwrap_or(0)
    }

    /// True when every thumbnail around the scroll position has loaded.
    pub fn thumbnails_settled(&self) -> bool {
        self.queue.is_settled(self.layout.order(), self.load_focus())
    }

    /// True when nothing on screen is still loading, the loupe included.
    #[cfg(feature = "screenshot")]
    pub fn is_settled(&self) -> bool {
        self.thumbnails_settled() && !matches!(&self.mode, Mode::Loupe(loupe) if loupe.is_loading()) && !self.inspector.is_loading()
    }

    /// Whether the selected photo is in the current view. The dock only
    /// shows then.
    pub fn selection_shown(&self) -> bool {
        self.layout.contains(self.culling.selected())
    }

    /// The outing holding the selected photo, in the Unsorted view.
    pub fn selected_outing(&self) -> Option<usize> {
        (self.view == View::Unsorted).then(|| self.plan.outing_of(self.culling.selected())).flatten()
    }

    /// Rebuilds sections and rows after the view, the filter or the library
    /// changed. Keeps the scroll position when only header text changed.
    fn refresh(&mut self) {
        let base: Vec<Section> = match &self.view {
            View::Library => self.library.folder_sections(|_| true),
            View::Unsorted => self.plan.sections(),
            View::Category(name) => self.library.folder_sections(|i| self.library.category_of(i) == Some(name.as_str())),
        };
        self.view_photos = base.iter().flat_map(|s| s.photos.iter().copied()).collect();
        self.sections.clear();
        self.section_outings.clear();
        for (index, mut section) in base.into_iter().enumerate() {
            if !self.filter.is_empty() {
                section.photos = self.filter.apply(&self.library, &section.photos, |p| self.culling.marks(p));
            }
            if !section.photos.is_empty() {
                self.sections.push(section);
                self.section_outings.push(index);
            }
        }
        self.outing_videos = match self.view {
            View::Unsorted => self.plan.videos(&self.library).iter().map(Vec::len).collect(),
            _ => Vec::new(),
        };
        self.faded = vec![false; self.aspects.len()];
        if self.view == View::Unsorted {
            for &photo in self.plan.accepted().flat_map(Outing::photos) {
                self.faded[photo] = true;
            }
        }

        let layout = Layout::new(&self.sections, &self.aspects, metrics(self.layout_width, self.row_height));
        let same_rows = layout == self.layout;
        self.layout = layout;
        if !self.layout.contains(self.culling.selected())
            && let Some(&first) = self.layout.order().first()
        {
            self.culling.apply(Intent::Select(first), &self.layout);
        }
        if same_rows {
            self.list.remeasure();
        } else {
            self.list.reset(self.layout.blocks().len());
            self.reveal_selection();
        }
    }

    pub(crate) fn resize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let width = grid_width(window, self.inspector.open);
        if width != self.layout_width {
            self.layout_width = width;
            self.refresh();
            cx.notify();
        }
    }

    /// Makes the thumbnails bigger or smaller by `factor`, keeping the
    /// photo at the top of the grid in place.
    pub fn zoom_grid(&mut self, factor: f32, cx: &mut Context<Self>) {
        let height = (self.row_height * factor).clamp(ROW_HEIGHTS.0, ROW_HEIGHTS.1);
        if (height - self.row_height).abs() < 0.5 {
            return;
        }
        // The top photo, and how many blocks above it the top is, so a
        // section header shown above it stays shown.
        let top = self.list.logical_scroll_top().item_ix;
        let anchor = self.layout.first_photo_from(top).and_then(|photo| Some((photo, self.layout.block_of(photo)?.saturating_sub(top))));
        self.row_height = height;
        self.refresh();
        if let Some((photo, above)) = anchor
            && let Some(block) = self.layout.block_of(photo)
        {
            self.list.scroll_to(ListOffset { item_ix: block.saturating_sub(above), offset_in_item: px(0.) });
        }
        cx.notify();
    }

    fn reveal_selection(&self) {
        if let Some(block) = self.layout.block_of(self.culling.selected()) {
            self.list.scroll_to_reveal_item(block);
        }
    }

    pub fn show(&mut self, view: View, cx: &mut Context<Self>) {
        if self.view != view {
            self.view = view;
            self.mode = Mode::Browse;
            self.refresh();
        }
        cx.notify();
    }

    /// The saved view the grid shows right now, if any: the Library with
    /// exactly that view's filter.
    pub fn active_view(&self) -> Option<&SavedView> {
        (self.view == View::Library && !self.filter.is_empty()).then(|| self.views.iter().find(|v| v.filter == self.filter)).flatten()
    }

    /// Updates the photo counts next to saved views in the sidebar.
    fn count_views(&mut self) {
        let all: Vec<usize> = (0..self.library.photos().len()).collect();
        self.view_counts = self.views.iter().map(|v| v.filter.apply(&self.library, &all, |p| self.culling.marks(p)).len()).collect();
    }

    pub fn open_view(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(view) = self.views.get(index) else { return };
        self.filter = view.filter.clone();
        self.view = View::Library;
        self.mode = Mode::Browse;
        self.refresh();
        cx.notify();
    }

    /// Opens the ⌘K bar. With nothing typed it lists a few of every kind of
    /// filter, so it doubles as an overview of what's possible.
    pub fn open_filter(&mut self, cx: &mut Context<Self>) {
        self.mode = Mode::Filtering { text: TextField::new(""), highlighted: 0, suggestions: Vec::new() };
        self.update_suggestions();
        cx.notify();
    }

    /// Saved views whose name has every typed word come first, then
    /// conditions. With nothing typed and no filter yet, every view shows,
    /// as part of the overview.
    fn update_suggestions(&mut self) {
        let Mode::Filtering { text, highlighted, suggestions } = &mut self.mode else { return };
        let words: Vec<String> = text.text().split_whitespace().map(str::to_lowercase).collect();
        let views = self.views.iter().enumerate().filter(|(_, view)| {
            let name = view.name.to_lowercase();
            if words.is_empty() { self.filter.is_empty() } else { words.iter().all(|w| name.contains(w.as_str())) }
        });
        let conditions = filter::suggest(&self.library, &self.view_photos, |p| self.culling.marks(p), &self.filter, text.text());
        *suggestions = views.map(|(i, _)| BarItem::View(i)).chain(conditions.into_iter().map(BarItem::Condition)).collect();
        *highlighted = (*highlighted).min(suggestions.len().saturating_sub(1));
    }

    /// Opens a picked view, or adds a picked condition to the filter and
    /// clears what was typed.
    pub fn add_suggestion(&mut self, index: usize, cx: &mut Context<Self>) {
        let Mode::Filtering { text, highlighted, suggestions } = &mut self.mode else { return };
        let condition = match suggestions.get(index) {
            Some(BarItem::Condition(picked)) => picked.condition.clone(),
            Some(&BarItem::View(view)) => return self.open_view(view, cx),
            None => return,
        };
        self.filter.add(condition);
        *text = TextField::new("");
        *highlighted = 0;
        self.refresh();
        self.update_suggestions();
        cx.notify();
    }

    pub fn remove_condition(&mut self, index: usize, cx: &mut Context<Self>) {
        self.filter.remove(index);
        self.refresh();
        self.update_suggestions();
        cx.notify();
    }

    pub fn clear_filter(&mut self, cx: &mut Context<Self>) {
        self.filter = Filter::default();
        self.refresh();
        self.update_suggestions();
        cx.notify();
    }

    fn filter_key(&mut self, keystroke: &Keystroke, secondary: bool, cx: &mut Context<Self>) {
        let Mode::Filtering { text, highlighted, suggestions } = &mut self.mode else { return };
        match keystroke.key.as_str() {
            "escape" => self.mode = Mode::Browse,
            "enter" => {
                let index = *highlighted;
                return self.add_suggestion(index, cx);
            }
            "down" => *highlighted = (*highlighted + 1).min(suggestions.len().saturating_sub(1)),
            "up" => *highlighted = highlighted.saturating_sub(1),
            "s" if secondary => return self.start_naming(cx),
            "backspace" if text.is_empty() => {
                self.filter.pop();
                self.refresh();
            }
            _ => {
                self.edit_key(keystroke, secondary, cx);
                return;
            }
        }
        self.update_suggestions();
        cx.notify();
    }

    /// Keeps what depends on the typed text in step with it.
    pub(crate) fn after_text_change(&mut self) {
        match self.mode {
            Mode::Filtering { .. } => self.update_suggestions(),
            Mode::Typing { .. } => self.list.remeasure(),
            _ => {}
        }
    }

    /// Asks for a name to save the current filter under. In a saved view
    /// the name starts as that view's, so Enter updates it.
    pub fn start_naming(&mut self, cx: &mut Context<Self>) {
        if self.filter.is_empty() {
            self.tell("Add a filter first, then save it as a view".into(), None, cx);
            return;
        }
        let text = self.active_view().map(|v| v.name.clone()).unwrap_or_default();
        self.mode = Mode::NamingView { text: TextField::selected(text) };
        cx.notify();
    }

    fn naming_key(&mut self, keystroke: &Keystroke, secondary: bool, cx: &mut Context<Self>) {
        let Mode::NamingView { text } = &mut self.mode else { return };
        match keystroke.key.as_str() {
            "escape" => self.mode = Mode::Browse,
            "enter" => {
                let name = text.text().trim().to_owned();
                if !name.is_empty() {
                    self.mode = Mode::Browse;
                    self.save_view(SavedView { name: name.clone(), filter: self.filter.clone() }, format!("Saved the view {name}"), cx);
                }
            }
            _ => {
                self.edit_key(keystroke, secondary, cx);
            }
        }
        cx.notify();
    }

    fn save_view(&mut self, view: SavedView, done: String, cx: &mut Context<Self>) {
        match views::save_view(self.library.root(), &view) {
            Ok(()) => {
                self.reload_views(cx);
                self.tell(done, None, cx);
            }
            Err(error) => self.tell(format!("Couldn't save the view: {error}"), None, cx),
        }
    }

    /// Deletes the saved view on screen. The notice offers to restore it.
    pub fn delete_active_view(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.active_view().cloned() else { return };
        match views::delete_view(self.library.root(), &view.name) {
            Ok(()) => {
                self.reload_views(cx);
                self.tell(format!("Deleted the view {}", view.name), Some(NoticeAction::Restore(view)), cx);
            }
            Err(error) => self.tell(format!("Couldn't delete the view: {error}"), None, cx),
        }
        cx.notify();
    }

    pub fn restore_view(&mut self, view: SavedView, cx: &mut Context<Self>) {
        let done = format!("Restored the view {}", view.name);
        self.save_view(view, done, cx);
        cx.notify();
    }

    fn reload_views(&mut self, cx: &mut Context<Self>) {
        let (views, problems) = views::load_views(self.library.root());
        self.views = views;
        self.count_views();
        if let Some(problem) = problems.into_iter().next() {
            self.tell(problem, None, cx);
        }
    }

    pub fn select(&mut self, photo: usize, cx: &mut Context<Self>) {
        self.cull(Intent::Select(photo), cx);
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.press(&event.keystroke, window, cx);
    }

    /// Handles one keystroke. Public so the screenshot harness can type.
    pub fn press(&mut self, keystroke: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        let key = keystroke.key.as_str();
        let secondary = keystroke.modifiers.secondary(cx);
        match &self.mode {
            Mode::Loupe(_) => self.loupe_key(keystroke, secondary, window, cx),
            Mode::Typing { .. } => self.type_key(keystroke, secondary, cx),
            Mode::Filtering { .. } => self.filter_key(keystroke, secondary, cx),
            Mode::NamingView { .. } => self.naming_key(keystroke, secondary, cx),
            Mode::Review(_) => match key {
                "escape" => self.close_review(cx),
                "enter" => self.run_moves(cx),
                _ => {}
            },
            Mode::Browse if secondary => match key {
                "z" if keystroke.modifiers.shift => self.redo(cx),
                "z" => self.undo(cx),
                "1" => self.show(View::Library, cx),
                "2" => self.show(View::Unsorted, cx),
                // The categories, in sidebar order.
                "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
                    let index = key.parse::<usize>().unwrap_or(3) - 3;
                    if let Some((name, _)) = self.library.categories().into_iter().nth(index) {
                        self.show(View::Category(name), cx);
                    }
                }
                "enter" => self.open_review(cx),
                "e" => self.open_in_affinity(cx),
                "o" => self.open_other(window, cx),
                "k" => self.open_filter(cx),
                "=" | "+" => self.zoom_grid(1.25, cx),
                "-" => self.zoom_grid(0.8, cx),
                "s" => self.start_naming(cx),
                "backspace" => self.delete_active_view(cx),
                _ => {}
            },
            Mode::Browse => {
                if key == "escape" && !self.filter.is_empty() {
                    self.clear_filter(cx);
                } else if key == "space" {
                    self.open_loupe(window, cx);
                } else if key == "i" {
                    self.toggle_inspector(window, cx);
                } else if let Some(intent) = cull_intent(key) {
                    self.cull(intent, cx);
                } else if self.view == View::Unsorted {
                    match key {
                        "enter" => self.accept_or_ask(cx),
                        "r" => self.start_typing(Field::Name, cx),
                        "c" => self.start_typing(Field::Category { then_accept: false }, cx),
                        "s" => self.split(cx),
                        "m" => self.merge_up(cx),
                        "[" => self.change_gap(false, cx),
                        "]" => self.change_gap(true, cx),
                        _ => {}
                    }
                }
            }
        }
    }

    /// Moves the selection or marks the selected photo, saving marks to its
    /// sidecar straight away.
    pub fn cull(&mut self, intent: Intent, cx: &mut Context<Self>) {
        if let Some(photo) = self.culling.apply(intent, &self.layout) {
            self.save_marks(photo, cx);
            // The grid keeps the photo until the filter changes, so a mark
            // that no longer matches doesn't yank it away mid-cull.
            self.count_views();
        }
        self.reveal_selection();
        cx.notify();
    }

    /// Hands the selected photo to Affinity. Only a failure gets a notice,
    /// since Affinity coming to the front says enough.
    pub fn open_in_affinity(&mut self, cx: &mut Context<Self>) {
        if !self.selection_shown() {
            return;
        }
        let path = self.library.photo(self.culling.selected()).path.clone();
        cx.spawn(async move |this, cx| {
            let opened = cx.background_spawn(async move { folio_core::editor::open_in_affinity(&path) }).await;
            if let Err(error) = opened {
                let _ = this.update(cx, |this, cx| this.tell(format!("Couldn't open in Affinity: {error}"), None, cx));
            }
        })
        .detach();
    }

    fn save_marks(&mut self, photo: usize, cx: &mut Context<Self>) {
        let existing = self.library.photo(photo).sidecar.clone();
        let sidecar = existing.clone().unwrap_or_else(|| new_sidecar(&self.library.photo(photo).relative));
        match xmp::write_marks(&self.library.absolute(&sidecar), self.culling.marks(photo)) {
            Ok(()) if existing.is_none() => self.library.set_sidecar(photo, sidecar),
            Ok(()) => {}
            Err(error) => self.tell(format!("Couldn't save to {sidecar}: {error}"), None, cx),
        }
    }

    fn sort(&mut self, intent: SortIntent, cx: &mut Context<Self>) -> bool {
        let changed = self.plan.apply(intent);
        if changed {
            self.refresh();
            self.save_drafts(cx);
        }
        changed
    }

    /// Saves the outings the user changed, so they survive a quit.
    fn save_drafts(&mut self, cx: &mut Context<Self>) {
        let drafts = self.plan.drafts(&self.library);
        if drafts == self.saved_drafts {
            return;
        }
        match drafts::save_drafts(self.library.root(), &self.saved_drafts, &drafts) {
            Ok(()) => self.saved_drafts = drafts,
            Err(error) => self.tell(format!("Couldn't save the outings: {error}"), None, cx),
        }
    }

    /// Asks for another library and opens it in place of this one.
    pub fn open_other(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let Some(root) = open::choose_folder(cx).await else { return };
            let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let _ = this.update(cx, |this, cx| this.tell(format!("Opening {name}…"), None, cx));
            match open::open(root, cx).await {
                Ok(_) => {
                    let _ = cx.update(|window, _| window.remove_window());
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| this.tell(error, None, cx));
                }
            }
        })
        .detach();
    }

    /// Enter on an outing: accept it, or reopen it if it's accepted. Without
    /// a category, ask for one first and accept once it's typed.
    pub fn accept_or_ask(&mut self, cx: &mut Context<Self>) {
        let Some(outing) = self.selected_outing() else { return };
        if self.plan.outings()[outing].category().is_none() {
            self.start_typing(Field::Category { then_accept: true }, cx);
            return;
        }
        self.sort(SortIntent::ToggleAccepted { outing }, cx);
        if self.plan.outings()[outing].is_accepted() {
            self.select_next_open_outing(outing);
        }
        cx.notify();
    }

    /// After accepting, jump to the next outing that still needs a decision.
    fn select_next_open_outing(&mut self, after: usize) {
        let next = self.plan.outings().iter().skip(after + 1).find(|o| !o.is_accepted());
        if let Some(&photo) = next.and_then(|o| o.photos().first()) {
            self.culling.apply(Intent::Select(photo), &self.layout);
            self.reveal_selection();
        }
    }

    pub fn start_typing(&mut self, field: Field, cx: &mut Context<Self>) {
        let Some(outing) = self.selected_outing() else { return };
        let current = &self.plan.outings()[outing];
        let text = match field {
            Field::Name => current.name().to_owned(),
            Field::Category { .. } => current.category().unwrap_or_default().to_owned(),
        };
        self.mode = Mode::Typing { outing, field, text: TextField::selected(text) };
        self.list.remeasure();
        cx.notify();
    }

    fn type_key(&mut self, keystroke: &Keystroke, secondary: bool, cx: &mut Context<Self>) {
        let Mode::Typing { field, text, .. } = &mut self.mode else { return };
        match keystroke.key.as_str() {
            "escape" => self.mode = Mode::Browse,
            "enter" => return self.commit_typing(None, cx),
            // Completes the category from what's typed before the selection.
            "tab" => {
                if matches!(field, Field::Category { .. })
                    && let Some(first) =
                        self.plan.category_suggestions(&self.library, typed_prefix(text)).into_iter().next()
                {
                    *text = TextField::new(first);
                }
            }
            _ => {
                self.edit_key(keystroke, secondary, cx);
                return;
            }
        }
        self.list.remeasure();
        cx.notify();
    }

    /// Ends typing. `picked` is a suggestion the user clicked; otherwise the
    /// typed text counts, spelled like an existing category if it matches one.
    pub fn commit_typing(&mut self, picked: Option<String>, cx: &mut Context<Self>) {
        let Mode::Typing { outing, field, text } = std::mem::replace(&mut self.mode, Mode::Browse) else { return };
        let text = text.text().to_owned();
        match field {
            Field::Name => {
                self.sort(SortIntent::Rename { outing, name: text }, cx);
            }
            Field::Category { then_accept } => {
                let category = picked.unwrap_or_else(|| {
                    let exact = self.plan.category_suggestions(&self.library, &text).into_iter().find(|s| s.eq_ignore_ascii_case(text.trim()));
                    exact.unwrap_or(text)
                });
                let set = self.sort(SortIntent::SetCategory { outing, category }, cx);
                if set && then_accept && !self.plan.outings()[outing].is_accepted() {
                    self.sort(SortIntent::ToggleAccepted { outing }, cx);
                    self.select_next_open_outing(outing);
                }
            }
        }
        self.list.remeasure();
        cx.notify();
    }

    /// Starts new outings after a shorter or longer gap, saved with the
    /// library. Outings the user edited stay as they are; the rest regroup.
    pub fn change_gap(&mut self, longer: bool, cx: &mut Context<Self>) {
        let Some(minutes) = step_gap(self.settings.outing_gap_minutes, longer) else {
            let end = if longer { "longest" } else { "shortest" };
            self.tell(format!("{} is the {end} gap", gap_text(self.settings.outing_gap_minutes)), None, cx);
            return cx.notify();
        };
        self.settings.outing_gap_minutes = minutes;
        if let Err(error) = save_settings(self.library.root(), &self.settings) {
            self.tell(format!("Couldn't save the gap: {error}"), None, cx);
        }
        let drafts = self.plan.drafts(&self.library);
        self.plan = SortPlan::group(&self.library, minutes * 60);
        self.plan.apply_drafts(&self.library, &drafts);
        self.refresh();
        cx.notify();
    }

    pub fn split(&mut self, cx: &mut Context<Self>) {
        if !self.sort(SortIntent::SplitAt { photo: self.culling.selected() }, cx) {
            self.tell("Select a photo after the first one to split there".into(), None, cx);
        }
        cx.notify();
    }

    pub fn merge_up(&mut self, cx: &mut Context<Self>) {
        let Some(outing) = self.selected_outing() else { return };
        if !self.sort(SortIntent::MergeUp { outing }, cx) {
            self.tell("There's no outing above this one to merge into".into(), None, cx);
        }
        cx.notify();
    }

    /// Plans the moves for every accepted outing and shows them for review.
    pub fn open_review(&mut self, cx: &mut Context<Self>) {
        let folders: Vec<String> = self.plan.accepted().filter_map(Outing::folder).collect();
        let taken = moves::existing_files(self.library.root(), folders);
        let plan = moves::plan(&self.library, &self.plan, &taken);
        if plan.is_empty() {
            self.tell("Nothing to move yet. Give an outing a category with C, then accept it with Enter".into(), None, cx);
        } else {
            self.mode = Mode::Review(plan);
        }
        cx.notify();
    }

    pub fn close_review(&mut self, cx: &mut Context<Self>) {
        self.mode = Mode::Browse;
        cx.notify();
    }

    /// Runs the reviewed moves.
    pub fn run_moves(&mut self, cx: &mut Context<Self>) {
        let Mode::Review(plan) = std::mem::replace(&mut self.mode, Mode::Browse) else { return };
        let id = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis()).to_string();
        match moves::execute(self.library.root(), &id, &plan.moves()) {
            Ok(outcome) => {
                self.library.apply_moves(&outcome.moved);
                self.plan.forget_sorted(&self.library);
                self.refresh();
                self.save_drafts(cx);
                let moved: HashSet<usize> =
                    plan.folders.iter().flat_map(|f| &f.photos).map(|p| p.photo).filter(|&p| self.library.is_sorted(p)).collect();
                let folders = plan.folders.len();
                let mut text = format!("Moved {} into {}", count(moved.len(), "photo"), count(folders, "folder"));
                if !outcome.failed.is_empty() {
                    text.push_str(&format!(". {} couldn't move", count(outcome.failed.len(), "file")));
                    for (step, reason) in &outcome.failed {
                        eprintln!("couldn't move {} to {}: {reason}", step.from, step.to);
                    }
                }
                self.tell(text, Some(NoticeAction::Undo), cx);
            }
            Err(error) => self.tell(format!("Nothing moved: {error}"), None, cx),
        }
        cx.notify();
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        self.mode = Mode::Browse;
        match moves::undo(self.library.root()) {
            Ok(Some((_, outcome))) => {
                self.library.apply_moves(&outcome.moved);
                self.plan.restore(&self.library, &outcome.moved);
                self.refresh();
                self.save_drafts(cx);
                let photos = outcome.moved.iter().filter(|m| self.library.photo_at(&m.to).is_some()).count();
                self.tell(format!("Moved {} back", count(photos, "photo")), Some(NoticeAction::Redo), cx);
            }
            Ok(None) => self.tell("Nothing to undo".into(), None, cx),
            Err(error) => self.tell(format!("Couldn't undo: {error}"), None, cx),
        }
        cx.notify();
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        self.mode = Mode::Browse;
        match moves::redo(self.library.root()) {
            Ok(Some((_, outcome))) => {
                self.library.apply_moves(&outcome.moved);
                self.plan.forget_sorted(&self.library);
                self.refresh();
                self.save_drafts(cx);
                let photos = outcome.moved.iter().filter(|m| self.library.photo_at(&m.to).is_some()).count();
                self.tell(format!("Moved {} again", count(photos, "photo")), Some(NoticeAction::Undo), cx);
            }
            Ok(None) => self.tell("Nothing to redo".into(), None, cx),
            Err(error) => self.tell(format!("Couldn't redo: {error}"), None, cx),
        }
        cx.notify();
    }

    /// Shows a notice in place of the dock, then hides it after a few seconds.
    pub fn tell(&mut self, text: String, action: Option<NoticeAction>, cx: &mut Context<Self>) {
        self.notice = Some(Notice { text, action });
        self._notice_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(NOTICE_SECONDS)).await;
            let _ = this.update(cx, |this, cx| {
                this.notice = None;
                cx.notify();
            });
        }));
    }
}

impl Render for Folio {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .relative()
            .size_full()
            .flex()
            .bg(crate::theme::CANVAS)
            .font_family(crate::theme::FONT_UI)
            .text_color(crate::theme::TEXT)
            .child(self.render_sidebar(cx))
            .child(self.render_main(cx))
            .when(self.inspector.open, |root| root.child(self.render_inspector(cx)))
            .when(matches!(self.mode, Mode::Review(_)), |root| root.child(self.render_review(cx)))
            .when(matches!(self.mode, Mode::Loupe(_)), |root| root.child(self.render_loupe(window, cx)))
            .when(matches!(self.mode, Mode::Filtering { .. } | Mode::NamingView { .. }), |root| {
                root.child(self.render_filter_bar(cx))
            })
            .child(crate::chrome::titlebar(cx, |this: &mut Self| &mut this.drag_pending))
    }
}

/// What the user typed in a field, leaving out a selected old value that
/// the next key would replace.
pub(crate) fn typed_prefix(field: &TextField) -> &str {
    let selection = field.selection();
    if selection.is_empty() { field.text() } else { &field.text()[..selection.start] }
}

/// `3 photos`, `1 folder`.
pub fn count(n: usize, noun: &str) -> String {
    if n == 1 { format!("1 {noun}") } else { format!("{n} {noun}s") }
}

/// The grid's width: the window less the sidebar and the inspector.
fn grid_width(window: &Window, inspector: bool) -> f32 {
    let inspector = if inspector { INSPECTOR_WIDTH } else { 0. };
    f32::from(window.viewport_size().width) - SIDEBAR_WIDTH - inspector
}

fn metrics(width: f32, target_height: f32) -> Metrics {
    Metrics { width, target_height, gap: GAP }
}

/// The culling keys: arrows move, 1 to 5 rate, 0 clears, 6 to 9 set a color
/// label (Lightroom's mapping), P picks, X rejects, U clears the flag.
pub(crate) fn cull_intent(key: &str) -> Option<Intent> {
    let intent = match key {
        "left" => Intent::Move(Direction::Left),
        "right" => Intent::Move(Direction::Right),
        "up" => Intent::Move(Direction::Up),
        "down" => Intent::Move(Direction::Down),
        "0" => Intent::Rate(None),
        "1" => Intent::Rate(Some(Stars::One)),
        "2" => Intent::Rate(Some(Stars::Two)),
        "3" => Intent::Rate(Some(Stars::Three)),
        "4" => Intent::Rate(Some(Stars::Four)),
        "5" => Intent::Rate(Some(Stars::Five)),
        "6" => Intent::ToggleLabel(Label::Red),
        "7" => Intent::ToggleLabel(Label::Yellow),
        "8" => Intent::ToggleLabel(Label::Green),
        "9" => Intent::ToggleLabel(Label::Blue),
        "p" => Intent::Flag(Some(Flag::Pick)),
        "x" => Intent::Flag(Some(Flag::Reject)),
        "u" => Intent::Flag(None),
        _ => return None,
    };
    Some(intent)
}

/// One worker per core. Each takes the most urgent thumbnail, loads it on
/// the background pool through the disk cache, hands it back and asks for
/// the next. After each load, thumbnails that left the window around the
/// scroll position are freed. An idle worker looks again a moment later, so
/// scrolling or switching views starts loading without anyone waking it.
fn spawn_workers(cache: Option<Arc<ThumbnailCache>>, window: &mut Window, cx: &mut Context<Folio>) -> Vec<Task<()>> {
    const IDLE: Duration = Duration::from_millis(250);
    let count = std::thread::available_parallelism().map_or(4, NonZeroUsize::get);
    (0..count)
        .map(|_| {
            let cache = cache.clone();
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    let job = this.update(cx, |this, _| {
                        let focus = this.load_focus();
                        this.queue.take_next(this.layout.order(), focus).map(|i| (i, this.library.photo(i).path.clone()))
                    });
                    let (index, path) = match job {
                        Ok(Some(job)) => job,
                        Ok(None) => {
                            cx.background_executor().timer(IDLE).await;
                            continue;
                        }
                        Err(_) => break,
                    };

                    let cache = cache.clone();
                    let thumb = cx
                        .background_spawn(async move {
                            let loaded = match &cache {
                                Some(cache) => cache.load(&path, THUMB_PX),
                                None => load_thumbnail(&path, THUMB_PX),
                            };
                            match loaded {
                                Ok(image) => Thumb::Ready(Arc::new(to_render_image(image))),
                                Err(error) => {
                                    eprintln!("no thumbnail for {}: {error}", path.display());
                                    Thumb::Failed
                                }
                            }
                        })
                        .await;

                    let stored = this.update_in(cx, |this, window, cx| {
                        this.queue.finish(index, matches!(thumb, Thumb::Ready(_)));
                        this.thumbs[index] = thumb;
                        let focus = this.load_focus();
                        for photo in this.queue.evict(this.layout.order(), focus) {
                            if let Thumb::Ready(image) = std::mem::replace(&mut this.thumbs[photo], Thumb::Loading) {
                                cx.drop_image(image, Some(window));
                            }
                        }
                        cx.notify();
                    });
                    if stored.is_err() {
                        break;
                    }
                }
            })
        })
        .collect()
}

/// gpui wants BGRA; the core hands out RGBA.
pub(crate) fn to_render_image(mut image: image::RgbaImage) -> RenderImage {
    for pixel in image.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    RenderImage::new(vec![image::Frame::new(image)])
}
