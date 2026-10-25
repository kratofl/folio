//! Development aid, built only with `--features screenshot`. Lets an agent
//! see the real app without screen-recording permission.
//!
//! `FOLIO_SCREENSHOT=out.png` waits until the visible thumbnails have
//! loaded, presses the keys in `FOLIO_KEYS`, waits again, renders the window
//! to a PNG and quits. Keys are space separated, in keybinding form (`right`,
//! `cmd-2`, `shift-cmd-z`). A key starting with `'` types the rest as text,
//! with `_` for spaces (`'Christmas_market`), through the input handler like
//! the platform's text input. `pinch+`, `pinch-`,
//! `swipe-left` and `swipe-right` send trackpad gestures at the window's
//! center.

use std::time::Duration;

use gpui::{
    App, AsyncWindowContext, Entity, EntityInputHandler, Keystroke, Modifiers, PinchEvent, PlatformInput, ScrollDelta, ScrollEvent, TouchPhase,
    Window, point, px,
};

use crate::app::Folio;

pub fn capture_when_ready(folio: Entity<Folio>, window: &mut Window, cx: &mut App) {
    let Ok(out) = std::env::var("FOLIO_SCREENSHOT") else { return };
    let keys = std::env::var("FOLIO_KEYS").unwrap_or_default();

    window
        .spawn(cx, async move |cx| {
            let tick = Duration::from_millis(200);
            while !folio.read_with(cx, |folio, _| folio.is_settled()) {
                cx.background_executor().timer(tick).await;
            }
            // Draw after every key like real typing does. The list only knows
            // the height of rows it has drawn, which scrolling relies on.
            for step in keys.split_whitespace().flat_map(steps) {
                let _ = match step {
                    Step::Key(keystroke) => folio.update_in(cx, |folio, window, cx| folio.press(&keystroke, window, cx)),
                    Step::Text(text) => folio.update_in(cx, |folio, window, cx| folio.replace_text_in_range(None, &text, window, cx)),
                    Step::Gesture(events) => cx.update(|window, cx| {
                        for event in events {
                            window.dispatch_event(event, cx);
                        }
                    }),
                };
                let _ = cx.update(|window, cx| window.draw(cx).clear(cx));
            }
            // Keys may have scrolled to photos that aren't loaded yet.
            while !folio.read_with(cx, |folio, _| folio.is_settled()) {
                cx.background_executor().timer(tick).await;
            }
            save(&out, cx);
        })
        .detach();
}

/// The welcome screen has nothing to wait for, so it's captured at once.
pub fn capture_welcome(window: &mut Window, cx: &mut App) {
    let Ok(out) = std::env::var("FOLIO_SCREENSHOT") else { return };
    window.spawn(cx, async move |cx| save(&out, cx)).detach();
}

/// Renders the window to `out` and quits.
fn save(out: &str, cx: &mut AsyncWindowContext) {
    // A hidden or occluded window may not have drawn lately, so draw the
    // current state explicitly. Twice, because the list measures rows it
    // hasn't seen on the first pass.
    let saved = cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
        window.render_to_image()
    });
    match saved {
        Ok(Ok(image)) => match image.save(out) {
            Ok(()) => eprintln!("screenshot saved to {out}"),
            Err(error) => eprintln!("can't save {out}: {error}"),
        },
        Ok(Err(error)) => eprintln!("can't render the window: {error}"),
        Err(error) => eprintln!("window is gone: {error}"),
    }
    let _ = cx.update(|_, cx| cx.quit());
}

enum Step {
    Key(Keystroke),
    /// Typed text, as an input method hands it over.
    Text(String),
    /// Trackpad events, sent to the window as the platform would.
    Gesture(Vec<PlatformInput>),
}

/// One token: `pinch+` and `pinch-` pinch at the window's center,
/// `swipe-left` and `swipe-right` swipe two fingers across it, `'text`
/// types, anything else is a keystroke.
fn steps(token: &str) -> Vec<Step> {
    let center = point(px(720.), px(450.));
    let pinch = |delta: f32| {
        let event = |phase| PlatformInput::Pinch(PinchEvent { position: center, delta, modifiers: Modifiers::default(), phase });
        vec![Step::Gesture(vec![event(TouchPhase::Started), event(TouchPhase::Moved), event(TouchPhase::Ended)])]
    };
    let swipe = |dx: f32| {
        let event = |dx: f32, touch_phase| {
            PlatformInput::Scroll(ScrollEvent {
                position: center,
                delta: ScrollDelta::Pixels(point(px(dx), px(0.))),
                touch_phase,
                ..ScrollEvent::default()
            })
        };
        let moves = (0..6).map(|_| event(dx / 6., TouchPhase::Moved));
        vec![Step::Gesture(std::iter::once(event(0., TouchPhase::Started)).chain(moves).chain([event(0., TouchPhase::Ended)]).collect())]
    };
    match token {
        "pinch+" => return pinch(0.5),
        "pinch-" => return pinch(-0.5),
        "swipe-left" => return swipe(-200.),
        "swipe-right" => return swipe(200.),
        _ => {}
    }
    if let Some(text) = token.strip_prefix('\'') {
        return vec![Step::Text(text.replace('_', " "))];
    }
    vec![Step::Key(keystroke(token))]
}

fn keystroke(token: &str) -> Keystroke {
    let mut parts: Vec<&str> = token.split('-').collect();
    let key = parts.pop().unwrap_or_default().to_owned();
    let mut modifiers = Modifiers::default();
    for part in parts {
        match part {
            "cmd" => modifiers.platform = true,
            "ctrl" => modifiers.control = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            _ => {}
        }
    }
    let key_char = (key.chars().count() == 1 && !modifiers.platform && !modifiers.control).then(|| key.clone());
    Keystroke { modifiers, key, key_char }
}
