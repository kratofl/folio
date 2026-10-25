//! Folio's desktop app. Usage: `folio [folder]`. Without a folder it
//! opens on the welcome screen.

mod app;
mod chrome;
mod filter_bar;
mod grid;
mod inspector;
mod loupe;
mod open;
mod sheet;
mod text_input;
mod theme;
mod welcome;
#[cfg(feature = "screenshot")]
mod screenshot;

use std::path::PathBuf;
use std::process::ExitCode;

use gpui::{App, KeyBinding, actions, prelude::*, px, size};

use crate::welcome::Welcome;

actions!(folio, [Quit]);

fn main() -> ExitCode {
    // With a folder, read it before any window opens, so a bad path fails
    // fast on the command line. Without one, the welcome screen asks.
    let loaded = match std::env::args_os().nth(1).map(PathBuf::from) {
        Some(root) => match open::load(&root) {
            Ok(loaded) => {
                for problem in &loaded.problems {
                    eprintln!("{problem}");
                }
                Some(loaded)
            }
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

    gpui_platform::application().run(move |cx: &mut App| {
        theme::load_fonts(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        let quit = if cfg!(target_os = "macos") { "cmd-q" } else { "ctrl-q" };
        cx.bind_keys([KeyBinding::new(quit, Quit, None)]);
        // One window per library. Closing the last one quits.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let opened = match loaded {
            Some(loaded) => open::open_window(loaded, cx).map(drop),
            None => open_welcome(cx),
        };
        if let Err(error) = opened {
            eprintln!("{error}");
            cx.quit();
        }
        cx.activate(true);
    });
    ExitCode::SUCCESS
}

fn open_welcome(cx: &mut App) -> Result<(), String> {
    let options = open::window_options("Folio".into(), size(px(720.), px(560.)), cx);
    cx.open_window(options, |window, cx| {
        let welcome = cx.new(|cx| Welcome::new(window, cx));
        #[cfg(feature = "screenshot")]
        screenshot::capture_welcome(window, cx);
        welcome
    })
    .map(drop)
    .map_err(|error| format!("Couldn't open a window: {error}"))
}
