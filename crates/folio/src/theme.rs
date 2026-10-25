//! Folio's Safelight identity as code: the color tokens and fonts the views
//! use. `docs/brand.md` has the full palette and the rules for using it. Add
//! a token here when a view needs it, never a one-off color in a view.

use std::borrow::Cow;

use folio_core::culling::Label;
use gpui::{App, Rgba};

/// The photo grid and loupe. Pure neutral black, so nothing tints how the
/// photos look.
pub const CANVAS: Rgba = hex(0x000000);
/// A tile whose thumbnail hasn't loaded. Neutral like the canvas, because it
/// sits in the photo area.
pub const TILE_EMPTY: Rgba = hex(0x121212);

/// Chrome gets the warm darkroom tint. The sidebar sits on `SURFACE`, things
/// that float above the grid (dock, sheets) on `RAISED`.
pub const SURFACE: Rgba = hex(0x16110F);
pub const RAISED: Rgba = hex(0x201A17);
pub const LINE: Rgba = hex(0x2E2622);
pub const HOVER: Rgba = hex(0x3D332E);

pub const TEXT: Rgba = hex(0xF6EDE6);
pub const TEXT_MUTED: Rgba = hex(0xB3A79F);
pub const TEXT_DIM: Rgba = hex(0x7A6E68);
/// Empty stars.
pub const TEXT_OFF: Rgba = hex(0x4D4440);

/// Safelight: selection, stars and primary actions. Nothing else.
pub const ACCENT: Rgba = hex(0xFF5A36);
pub const ACCENT_HOVER: Rgba = hex(0xFF7452);
/// Text on dark areas that carry the accent.
pub const ACCENT_SOFT: Rgba = hex(0xFFB38F);
/// Text on accent.
pub const ON_ACCENT: Rgba = hex(0x16110F);
/// Behind badges drawn on top of photos.
pub const SCRIM: Rgba = Rgba { r: 0., g: 0., b: 0., a: 0.6 };

/// Color labels, kept away from Safelight so a label never reads as the selection.
pub fn label(label: Label) -> Rgba {
    match label {
        Label::Red => hex(0xE0366F),
        Label::Yellow => hex(0xE9C440),
        Label::Green => hex(0x43B76A),
        Label::Blue => hex(0x3F8EF2),
        Label::Purple => hex(0xA274F2),
    }
}

/// A histogram channel: the matching label color, see-through so the
/// channels show where they overlap.
pub fn histogram_fill(channel: Label) -> Rgba {
    Rgba { a: 0.55, ..label(channel) }
}

/// The interface font.
pub const FONT_UI: &str = "Inter";
/// Big headings. Inter Display is Inter's tighter cut for large sizes.
pub const FONT_DISPLAY: &str = "Inter Display";

/// Registers the bundled fonts, so Folio looks the same on Mac and Windows.
/// If that fails the system font takes over, which is ugly but usable.
pub fn load_fonts(cx: &App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Bold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/InterDisplay-ExtraBold.ttf")),
    ];
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        eprintln!("can't load the bundled fonts, using the system font: {error}");
    }
}

const fn hex(value: u32) -> Rgba {
    Rgba { r: channel(value, 16), g: channel(value, 8), b: channel(value, 0), a: 1. }
}

const fn channel(value: u32, shift: u32) -> f32 {
    ((value >> shift) & 0xff) as f32 / 255.
}
