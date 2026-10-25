//! Zooming and panning the loupe. Pure: the viewport, the photo's shape and
//! a gesture in, the next zoom and where to draw the photo out.
//!
//! Zoom is kept relative to the photo, not to pixels on screen, so it means
//! the same thing whichever preview is loaded and however big the window is.

/// The furthest the loupe zooms in, relative to the whole photo fitting.
pub const MAX_MAGNIFICATION: f32 = 32.;

/// Where to draw the photo, in the viewport's coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// How the loupe shows a photo: `magnification` 1 fits the whole photo, 2
/// shows it twice as big. `center` is the point of the photo, from 0 to 1
/// on each axis, that sits in the middle of the viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zoom {
    magnification: f32,
    center: (f32, f32),
}

impl Default for Zoom {
    fn default() -> Self {
        Self::FIT
    }
}

impl Zoom {
    pub const FIT: Self = Self { magnification: 1., center: (0.5, 0.5) };

    pub fn is_fit(&self) -> bool {
        self.magnification <= 1.
    }

    pub fn magnification(&self) -> f32 {
        self.magnification
    }

    /// Where the photo goes in a `viewport` (width, height) for a photo of
    /// width over height `aspect`.
    pub fn rect(&self, viewport: (f32, f32), aspect: f32) -> Rect {
        let (fit_width, fit_height) = fit(viewport, aspect);
        let (width, height) = (fit_width * self.magnification, fit_height * self.magnification);
        Rect { x: viewport.0 / 2. - self.center.0 * width, y: viewport.1 / 2. - self.center.1 * height, width, height }
    }

    /// Zooms by `factor` keeping the photo's point under `anchor` in place,
    /// as a pinch does under the fingers.
    pub fn zoom_at(self, factor: f32, anchor: (f32, f32), viewport: (f32, f32), aspect: f32) -> Self {
        self.zoom_to(self.magnification * factor, anchor, viewport, aspect)
    }

    /// Zooms to `magnification`, keeping the photo's point under `anchor` in
    /// place. Anything below fitting snaps back to fit.
    pub fn zoom_to(self, magnification: f32, anchor: (f32, f32), viewport: (f32, f32), aspect: f32) -> Self {
        let magnification = magnification.clamp(1., MAX_MAGNIFICATION);
        let before = self.rect(viewport, aspect);
        // The photo's point under the anchor, from 0 to 1.
        let point = ((anchor.0 - before.x) / before.width, (anchor.1 - before.y) / before.height);
        let (fit_width, fit_height) = fit(viewport, aspect);
        let (width, height) = (fit_width * magnification, fit_height * magnification);
        let (x, y) = (anchor.0 - point.0 * width, anchor.1 - point.1 * height);
        let center = ((viewport.0 / 2. - x) / width, (viewport.1 / 2. - y) / height);
        Self { magnification, center }.clamped(viewport, aspect)
    }

    /// Moves the photo by `delta` in viewport units, as a drag or a
    /// two-finger scroll does.
    pub fn pan(self, delta: (f32, f32), viewport: (f32, f32), aspect: f32) -> Self {
        let rect = self.rect(viewport, aspect);
        let center = (self.center.0 - delta.0 / rect.width, self.center.1 - delta.1 / rect.height);
        Self { center, ..self }.clamped(viewport, aspect)
    }

    /// Keeps the photo covering the viewport on each axis where it's
    /// bigger, and centered where it's smaller, so panning never shows
    /// empty space past an edge.
    fn clamped(self, viewport: (f32, f32), aspect: f32) -> Self {
        if self.magnification <= 1. {
            return Self::FIT;
        }
        let (fit_width, fit_height) = fit(viewport, aspect);
        let axis = |center: f32, size: f32, view: f32| {
            if size <= view {
                0.5
            } else {
                let margin = view / 2. / size;
                center.clamp(margin, 1. - margin)
            }
        };
        let center = (
            axis(self.center.0, fit_width * self.magnification, viewport.0),
            axis(self.center.1, fit_height * self.magnification, viewport.1),
        );
        Self { center, ..self }
    }
}

/// The photo's size when it just fits the viewport.
fn fit(viewport: (f32, f32), aspect: f32) -> (f32, f32) {
    let aspect = if aspect > 0. { aspect } else { 1. };
    if viewport.0 / viewport.1.max(1.) > aspect {
        (viewport.1 * aspect, viewport.1)
    } else {
        (viewport.0, viewport.0 / aspect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: (f32, f32) = (1000., 500.);
    const WIDE: f32 = 1.5;

    #[test]
    fn fit_centers_the_whole_photo() {
        // 1.5:1 in a 2:1 viewport: full height, bars left and right.
        assert_eq!(Zoom::FIT.rect(VIEW, WIDE), Rect { x: 125., y: 0., width: 750., height: 500. });
    }

    #[test]
    fn zooming_keeps_the_point_under_the_fingers() {
        let anchor = (300., 100.);
        let before = Zoom::FIT.rect(VIEW, WIDE);
        let point = ((anchor.0 - before.x) / before.width, (anchor.1 - before.y) / before.height);

        let zoomed = Zoom::FIT.zoom_at(4., anchor, VIEW, WIDE);
        let after = zoomed.rect(VIEW, WIDE);
        assert_eq!(zoomed.magnification(), 4.);
        assert!((after.x + point.0 * after.width - anchor.0).abs() < 0.01);
        assert!((after.y + point.1 * after.height - anchor.1).abs() < 0.01);
    }

    #[test]
    fn panning_stops_at_the_edges_and_zooming_out_snaps_to_fit() {
        let zoomed = Zoom::FIT.zoom_at(2., (500., 250.), VIEW, WIDE);
        let far = zoomed.pan((10_000., 10_000.), VIEW, WIDE).rect(VIEW, WIDE);
        assert_eq!((far.x, far.y), (0., 0.), "the photo's top left corner meets the viewport's");

        let back = zoomed.zoom_at(0.1, (10., 10.), VIEW, WIDE);
        assert_eq!(back, Zoom::FIT);
        assert!(back.is_fit());
    }

    #[test]
    fn a_photo_narrower_than_the_viewport_stays_centered_across() {
        // At 1.2x the 750-wide photo is 900 wide, still narrower than 1000.
        let rect = Zoom::FIT.zoom_at(1.2, (0., 0.), VIEW, WIDE).pan((300., 0.), VIEW, WIDE).rect(VIEW, WIDE);
        assert!((rect.x - 50.).abs() < 0.01, "{}", rect.x);
    }
}
