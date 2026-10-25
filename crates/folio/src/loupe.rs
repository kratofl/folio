//! The loupe: one photo filling the window, for judging focus and
//! expression. Space opens and closes it, arrows and a two-finger swipe step
//! through the view, and every culling key works as in the grid.
//!
//! Pinch, ⌘-scroll, Z, + and - zoom. Dragging or two-finger scrolling pans a
//! zoomed photo. The zoom carries over to the next photo, so a burst can be
//! compared at the same spot.

use std::sync::Arc;

use folio_core::culling::Intent;
use folio_core::layout::Direction;
use folio_core::thumbnail::load_thumbnail;
use folio_core::zoom::Zoom;
use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, FontWeight, ImageSource, IntoElement, Keystroke, MouseButton,
    MouseDownEvent, MouseMoveEvent, NavigationDirection, ObjectFit, PinchEvent, Pixels, Point, RenderImage, ScrollEvent,
    TouchPhase, Window, div, img, prelude::*, px,
};

use crate::app::{Folio, Mode, Thumb, cull_intent, to_render_image};
use crate::chrome::pill;
use crate::inspector::INSPECTOR_WIDTH;
use crate::theme;

/// The preview size for the screen: sharp on Retina, decoded in about a
/// tenth of a second. Zooming in past its pixels loads the full-size one.
const SCREEN_PX: u32 = 2560;
/// Room around the photo: the window's drag strip above, the dock below.
const TOP: f32 = 38.;
const BOTTOM: f32 = 72.;
const SIDE: f32 = 24.;
/// How far two fingers travel sideways over a fitting photo to step to the
/// next one.
const SWIPE: f32 = 90.;
/// One press of + or -.
const KEY_STEP: f32 = 2.;

pub struct Loupe {
    /// The photo `large` belongs to.
    pub photo: usize,
    pub large: Large,
    pub zoom: Zoom,
    /// Set once the full-size preview is on its way, so it's asked for once.
    wants_full: bool,
    swipe: Swipe,
    /// Where the pointer was at the last step of a drag.
    drag: Option<Point<Pixels>>,
}

/// The loupe's preview. It's freed as soon as the loupe moves on.
pub enum Large {
    Loading,
    /// `full` when it's the full-size preview, not the screen-sized one.
    Ready { image: Arc<RenderImage>, full: bool },
    Failed,
}

/// A two-finger sideways scroll over a fitting photo. It steps once per
/// gesture, however far the fingers go.
enum Swipe {
    Idle,
    Tracking(f32),
    Stepped,
}

impl Loupe {
    /// True while a preview it wants is still decoding.
    #[cfg(feature = "screenshot")]
    pub fn is_loading(&self) -> bool {
        matches!(self.large, Large::Loading) || (self.wants_full && !matches!(self.large, Large::Ready { full: true, .. }))
    }
}

impl Folio {
    /// Shows the selected photo filling the window.
    pub fn open_loupe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selection_shown() {
            return;
        }
        let photo = self.culling.selected();
        self.mode = Mode::Loupe(Loupe { photo, large: Large::Loading, zoom: Zoom::FIT, wants_full: false, swipe: Swipe::Idle, drag: None });
        self.load_preview(photo, false, window, cx);
        cx.notify();
    }

    pub fn close_loupe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Mode::Loupe(Loupe { large: Large::Ready { image, .. }, .. }) = std::mem::replace(&mut self.mode, Mode::Browse) {
            cx.drop_image(image, Some(window));
        }
        self.loupe_load = None;
        cx.notify();
    }

    pub fn loupe_key(&mut self, keystroke: &Keystroke, secondary: bool, window: &mut Window, cx: &mut Context<Self>) {
        let center = {
            let (width, height) = viewport(window, self.inspector.open);
            (width / 2., height / 2.)
        };
        match keystroke.key.as_str() {
            "e" if secondary => self.open_in_affinity(cx),
            "o" if secondary => self.open_other(window, cx),
            _ if secondary => {}
            "space" | "escape" | "enter" => self.close_loupe(window, cx),
            "left" => self.step(Direction::Left, window, cx),
            "right" => self.step(Direction::Right, window, cx),
            // The loupe shows one photo, so there's no row to go up or down.
            "up" | "down" => {}
            "z" => self.toggle_zoom(center, window, cx),
            "i" => self.toggle_inspector(window, cx),
            "=" | "+" => self.zoom(|zoom, view, aspect| zoom.zoom_at(KEY_STEP, center, view, aspect), window, cx),
            "-" => self.zoom(|zoom, view, aspect| zoom.zoom_at(1. / KEY_STEP, center, view, aspect), window, cx),
            key => {
                if let Some(intent) = cull_intent(key) {
                    self.cull(intent, cx);
                }
            }
        }
    }

    /// Moves to the neighbouring photo, keeping the zoom.
    fn step(&mut self, direction: Direction, window: &mut Window, cx: &mut Context<Self>) {
        self.cull(Intent::Move(direction), cx);
        let selected = self.culling.selected();
        let Mode::Loupe(loupe) = &mut self.mode else { return };
        if loupe.photo == selected {
            return;
        }
        if let Large::Ready { image, .. } = std::mem::replace(&mut loupe.large, Large::Loading) {
            cx.drop_image(image, Some(window));
        }
        // Same pixel scale on the next photo, so 100% stays 100% from a
        // portrait to a landscape photo.
        let view = viewport(window, self.inspector.open);
        let (before, after) = (self.library.photo(loupe.photo), self.library.photo(selected));
        if !loupe.zoom.is_fit()
            && let (Some((before_width, _)), Some((after_width, _))) = (before.size, after.size)
        {
            let points_per_pixel = loupe.zoom.rect(view, before.aspect).width / before_width as f32;
            let fit_width = Zoom::FIT.rect(view, after.aspect).width;
            let center = (view.0 / 2., view.1 / 2.);
            loupe.zoom = loupe.zoom.zoom_to(points_per_pixel * after_width as f32 / fit_width, center, view, after.aspect);
        }
        loupe.photo = selected;
        loupe.wants_full = false;
        self.load_preview(selected, false, window, cx);
    }

    /// Decodes a preview of `photo` off the UI thread: screen-sized, or
    /// `full` size for zooming in close.
    fn load_preview(&mut self, photo: usize, full: bool, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.library.photo(photo).path.clone();
        let max_edge = if full { u32::MAX } else { SCREEN_PX };
        self.loupe_load = Some(cx.spawn_in(window, async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    load_thumbnail(&path, max_edge)
                        .map(|image| Arc::new(to_render_image(image)))
                        .map_err(|error| eprintln!("no preview for {}: {error}", path.display()))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Mode::Loupe(loupe) = &mut this.mode else { return };
                if loupe.photo != photo {
                    return;
                }
                match loaded {
                    Ok(image) => {
                        if let Large::Ready { image, .. } = std::mem::replace(&mut loupe.large, Large::Ready { image, full }) {
                            cx.drop_image(image, Some(window));
                        }
                    }
                    // A failed full-size load keeps the screen-sized preview.
                    Err(()) if full => loupe.wants_full = false,
                    Err(()) => loupe.large = Large::Failed,
                }
                this.load_full_if_needed(window, cx);
                cx.notify();
            });
        }));
    }

    /// Asks for the full-size preview once the zoom shows the photo bigger
    /// than the screen-sized preview's pixels.
    fn load_full_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (view, scale) = (viewport(window, self.inspector.open), window.scale_factor());
        let Mode::Loupe(loupe) = &mut self.mode else { return };
        let Large::Ready { image, full: false } = &loupe.large else { return };
        if loupe.wants_full {
            return;
        }
        let shown = loupe.zoom.rect(view, self.library.photo(loupe.photo).aspect).width * scale;
        if shown > image.size(0).width.0 as f32 * 1.05 {
            loupe.wants_full = true;
            let photo = loupe.photo;
            self.load_preview(photo, true, window, cx);
        }
    }

    fn zoom(&mut self, change: impl FnOnce(Zoom, (f32, f32), f32) -> Zoom, window: &mut Window, cx: &mut Context<Self>) {
        let view = viewport(window, self.inspector.open);
        let Mode::Loupe(loupe) = &mut self.mode else { return };
        loupe.zoom = change(loupe.zoom, view, self.library.photo(loupe.photo).aspect);
        self.load_full_if_needed(window, cx);
        cx.notify();
    }

    /// Fit to 100% (one photo pixel per screen pixel) and back, at `anchor`.
    fn toggle_zoom(&mut self, anchor: (f32, f32), window: &mut Window, cx: &mut Context<Self>) {
        let (view, scale) = (viewport(window, self.inspector.open), window.scale_factor());
        let Mode::Loupe(loupe) = &self.mode else { return };
        let photo = self.library.photo(loupe.photo);
        let loaded_width = match &loupe.large {
            Large::Ready { image, .. } => Some(image.size(0).width.0 as f32),
            _ => None,
        };
        let native = photo.size.map(|(width, _)| width as f32).or(loaded_width);
        let fit_width = Zoom::FIT.rect(view, photo.aspect).width;
        // Small photos that already fit at 100% zoom to twice their size.
        let actual = native.map_or(2., |native| native / (fit_width * scale)).max(2.);
        self.zoom(
            |zoom, view, aspect| if zoom.is_fit() { zoom.zoom_to(actual, anchor, view, aspect) } else { Zoom::FIT },
            window,
            cx,
        );
    }

    fn on_loupe_pinch(&mut self, event: &PinchEvent, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = anchor(event.position);
        self.zoom(|zoom, view, aspect| zoom.zoom_at(1. + event.delta, anchor, view, aspect), window, cx);
    }

    /// ⌘ or Ctrl with the wheel zooms, like a pinch. Otherwise two fingers
    /// pan a zoomed photo, and swipe sideways over a fitting one.
    fn on_loupe_scroll(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(20.));
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        if event.modifiers.platform || event.modifiers.control {
            let factor = if dy > 0. { 1. + dy.abs() * 0.01 } else { 1. / (1. + dy.abs() * 0.01) };
            let anchor = anchor(event.position);
            return self.zoom(|zoom, view, aspect| zoom.zoom_at(factor, anchor, view, aspect), window, cx);
        }
        let Mode::Loupe(loupe) = &mut self.mode else { return };
        if !loupe.zoom.is_fit() {
            return self.zoom(|zoom, view, aspect| zoom.pan((dx, dy), view, aspect), window, cx);
        }
        // With natural scrolling, fingers moving left scroll left: the next photo.
        let direction = match (&mut loupe.swipe, event.touch_phase) {
            (swipe, TouchPhase::Started) => {
                *swipe = Swipe::Tracking(dx);
                None
            }
            (swipe, TouchPhase::Ended | TouchPhase::Cancelled) => {
                *swipe = Swipe::Idle;
                None
            }
            (Swipe::Tracking(travel), TouchPhase::Moved) => {
                *travel += dx;
                match *travel {
                    t if t <= -SWIPE => Some(Direction::Right),
                    t if t >= SWIPE => Some(Direction::Left),
                    _ => None,
                }
            }
            (Swipe::Idle | Swipe::Stepped, TouchPhase::Moved) => None,
        };
        if let Some(direction) = direction {
            loupe.swipe = Swipe::Stepped;
            self.step(direction, window, cx);
        }
    }

    fn on_loupe_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.button {
            MouseButton::Navigate(NavigationDirection::Back) => self.step(Direction::Left, window, cx),
            MouseButton::Navigate(NavigationDirection::Forward) => self.step(Direction::Right, window, cx),
            MouseButton::Left => {
                if let Mode::Loupe(loupe) = &mut self.mode
                    && !loupe.zoom.is_fit()
                {
                    loupe.drag = Some(event.position);
                }
            }
            _ => {}
        }
    }

    fn on_loupe_mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Mode::Loupe(loupe) = &mut self.mode else { return };
        let Some(last) = loupe.drag else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            loupe.drag = None;
            return;
        }
        loupe.drag = Some(event.position);
        let moved = event.position - last;
        self.zoom(|zoom, view, aspect| zoom.pan((f32::from(moved.x), f32::from(moved.y)), view, aspect), window, cx);
    }

    pub fn render_loupe(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Mode::Loupe(loupe) = &self.mode else { return div().into_any_element() };
        let photo = self.library.photo(loupe.photo);
        let rect = loupe.zoom.rect(viewport(window, self.inspector.open), photo.aspect);
        // The grid's thumbnail stands in until the preview is decoded.
        let image = match (&loupe.large, &self.thumbs[loupe.photo]) {
            (Large::Ready { image, .. }, _) | (_, Thumb::Ready(image)) => Some(image.clone()),
            _ => None,
        };
        let picture = match image {
            Some(image) => img(ImageSource::Render(image))
                .absolute()
                .left(px(rect.x))
                .top(px(rect.y))
                .w(px(rect.width))
                .h(px(rect.height))
                .object_fit(ObjectFit::Fill)
                .into_any_element(),
            None if matches!(loupe.large, Large::Failed) => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme::TEXT_DIM)
                .child("No preview")
                .into_any_element(),
            None => div().into_any_element(),
        };
        // 100% is one photo pixel per screen pixel.
        let percent = photo.size.map(|(width, _)| rect.width * window.scale_factor() / width as f32 * 100.);
        let zoomed = !loupe.zoom.is_fit();

        div()
            .id("loupe")
            .absolute()
            .top_0()
            .left_0()
            .bottom_0()
            // The inspector stays beside the photo.
            .right(px(if self.inspector.open { INSPECTOR_WIDTH } else { 0. }))
            .bg(theme::CANVAS)
            .when(zoomed, |loupe| loupe.cursor(if self.loupe_dragging() { CursorStyle::ClosedHand } else { CursorStyle::OpenHand }))
            .on_pinch(cx.listener(Self::on_loupe_pinch))
            .on_scroll(cx.listener(Self::on_loupe_scroll))
            .on_any_mouse_down(cx.listener(Self::on_loupe_mouse_down))
            .on_mouse_move(cx.listener(Self::on_loupe_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    if let Mode::Loupe(loupe) = &mut this.mode {
                        loupe.drag = None;
                    }
                }),
            )
            // A double click zooms to 100% where it lands, and back to fit.
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                if event.click_count() == 2 {
                    this.toggle_zoom(anchor(event.position()), window, cx);
                }
            }))
            .child(div().absolute().top(px(TOP)).left(px(SIDE)).right(px(SIDE)).bottom(px(BOTTOM)).overflow_hidden().child(picture))
            .child(
                div().absolute().top(px(6.)).right(px(12.)).child(
                    pill()
                        .text_size(px(12.))
                        .when_some(percent.filter(|_| zoomed), |pill, percent| {
                            pill.child(div().text_color(theme::TEXT).child(format!("{percent:.0}%")))
                        })
                        .child(
                            div()
                                .id("loupe-close")
                                .cursor_pointer()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::TEXT_MUTED)
                                .hover(|close| close.text_color(theme::TEXT))
                                .child("Grid  Esc")
                                .on_click(cx.listener(|this, _, window, cx| this.close_loupe(window, cx))),
                        ),
                ),
            )
            .child(self.render_bottom(cx))
            .into_any_element()
    }

    fn loupe_dragging(&self) -> bool {
        matches!(&self.mode, Mode::Loupe(loupe) if loupe.drag.is_some())
    }
}

/// The area the photo is drawn in, in window points.
/// `inspector` when the inspector takes the right of the window.
fn viewport(window: &Window, inspector: bool) -> (f32, f32) {
    let size = window.viewport_size();
    let inspector = if inspector { INSPECTOR_WIDTH } else { 0. };
    ((f32::from(size.width) - inspector - 2. * SIDE).max(1.), (f32::from(size.height) - TOP - BOTTOM).max(1.))
}

/// A window position in the viewport's coordinates.
fn anchor(position: Point<Pixels>) -> (f32, f32) {
    (f32::from(position.x) - SIDE, f32::from(position.y) - TOP)
}
