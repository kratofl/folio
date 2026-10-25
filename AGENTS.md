# Agent guide

Folio is a fast, cross-platform (macOS and Windows) photo organizer. A folder of photos becomes a library, and the app's state lives in a hidden `.folio/` folder at its root, like `.git`. Read `README.md` for the product pitch. This file covers how to work on the code.

The project is written almost entirely by AI agents. That makes the rules below more important, not less. The owner won't read every line, so tests and fixtures are what prove the code is correct.

## Current status

The GPUI spike passed and the first organize features are in. The library format is in `docs/format.md`.

- `crates/folio-core`: scanning with sidecars and videos, EXIF (exposure, gear, GPS, time zone), preview extraction, the library model (sorted vs. unsorted folders, categories), outings grouped by time gap with their videos, outing drafts, move planning with crash-safe journals and undo, marks in XMP sidecars, merging of sync conflict copies, filters with suggestions, saved views, a disk thumbnail cache, recent libraries, library settings and the Affinity launcher, plus the pure layout, culling, thumbnail load-order, zoom, histogram and text-editing logic.
- `crates/folio`: the GPUI app. `folio <folder>` opens a library. Without a folder it starts on a welcome screen with Open folder and recent libraries. A library window has a sidebar (Library, Unsorted, categories, saved views) next to the "edge to edge" grid the owner picked.
  - Unsorted shows outings, split wherever photos are further apart than the library's gap (two hours at first). `[` and `]`, or the strip above the outings, make it shorter or longer. Enter accepts (asking for a category first), R renames, C sets the category, S splits at the selected photo, M merges into the outing above. ⌘Enter reviews the moves, ⌘Z and ⇧⌘Z undo and redo them. Videos shot during an outing move with it. Edited outings are saved as drafts, so quitting loses nothing.
  - Everywhere: arrows move, 1 to 5 rate, 0 clears, 6 to 9 set a label, P picks, X rejects, U clears the flag. Marks save to the sidecar at once. ⌘1, ⌘2 and ⌘3 to ⌘9 switch views. Space (or a double click) opens the loupe, ⌘E opens the photo in Affinity, ⌘O opens another library. ⌘= and ⌘- or a pinch resize the thumbnails. I shows the inspector on the right: histogram, exposure, marks and every metadata field, in folding groups.
  - Loupe: pinch, ⌘-scroll, Z (fit and 100%), + and - zoom at the pointer. Dragging or two fingers pan. Arrows, a two-finger swipe or the mouse's back and forward buttons step photos, keeping the zoom. Past the screen-sized preview it loads the full-size one.
  - Text fields have a caret and selection (arrows, ⌥ for words, ⌘ for the whole line, Shift extends), ⌘A, ⌘C, ⌘X and ⌘V. Typing goes through the platform's input handler, so dead keys and input methods compose.
  - Filters: ⌘K opens a bar that suggests saved views and conditions with photo counts (empty, it shows every view and two conditions of each kind). Enter opens a view or adds a condition as a chip, Backspace on an empty field removes the last, Esc closes and Esc again clears. ⌘S saves the filter as a view, ⌘⌫ deletes the view on screen. The dock's Filter button shows how many filters are on.
- Thumbnails are cached as small JPEGs in the OS cache folder, keyed by file name, size and modification time, so they survive moves. Only a window of about 360 thumbnails around the scroll position stays in memory.
- `scripts/bundle-macos.sh` builds `target/release/Folio.app` with the icon.

The app uses the Safelight identity (see `docs/brand.md`). Colors and fonts come from `crates/folio/src/theme.rs`, so views never hardcode a color. Shortcut labels show ⌘ on macOS and Ctrl on Windows.

Known gaps:

- Clicking in a text field doesn't move the caret. Editing is keyboard only.
- Windows only gets CI so far (`.github/workflows/ci.yml` runs the core tests and clippy on macOS and Windows). Nobody has run the app there yet, the Affinity launcher looks in the default install folders, and there's no installer.
- Trackpad gestures are tested with synthetic events in the screenshot harness (`pinch+`, `swipe-left`), not on real hardware.

## Building

`make` lists the everyday commands. They work the same in the main checkout and in any git worktree.

- `make check` runs the core tests and clippy (both crates, warnings are errors). Run it before calling work done. `make test` and `make lint` run the halves.
- `make run` opens the welcome screen, `make run LIB=<folder>` a library. `make run-scratch` makes a fresh scratch library and opens it.
- `make bundle` builds `target/release/Folio.app` with the icon.
- gpui is pinned to a Zed commit in the root `Cargo.toml`. The `runtime_shaders` feature means macOS builds don't need the full Xcode Metal toolchain.
- `cargo run --release -p folio-core --example thumbnail_speed -- <folder>` times the header pass and thumbnail extraction.
- Marking and moving write to the library, so test them on a scratch library. `make scratch` clones the fixtures into `/tmp/folio-scratch/<checkout>/` (an APFS clone, so it takes no space) and replaces any earlier one.
- To see the app without screen access, run `make screenshot KEYS="right cmd-2 c 'Events enter" SHOT=name`. It builds with `--features screenshot`, opens the scratch library (or `LIB`), presses the keys once the visible thumbnails load (keybinding form, `'` types text with `_` for spaces), waits for any it scrolled to, and saves `/tmp/folio-shots/<checkout>/name.png`. Without `KEYS` it captures the opening state. Run the binary with `FOLIO_SCREENSHOT=out.png` and no folder to capture the welcome screen. `pinch+`, `pinch-`, `swipe-left` and `swipe-right` in `KEYS` send trackpad gestures.
- Fixture photos live outside the repo in `../folio-fixtures/` next to the main checkout (a copy of the owner's `2024/` folder). They're too big to commit.

### Worktrees

Several agents can work at once, each in its own git worktree. A worktree only holds committed files, so commit before starting one.

- `make setup` prepares a checkout, and T3 Code runs it for every new worktree (see `t3.json`). It clones the main checkout's `target/` copy-on-write, so the first build compiles only Folio's own crates, in seconds instead of minutes. Then it fetches crates and checks the fixtures and the Zed reference checkout.
- Shared things are found through the main checkout, wherever the worktree lives: `../folio-fixtures` next to it and the reference checkouts in its git-ignored `.refs/`, not the worktree's. The Makefile works this out from git, and `make paths` shows the result. Override with `FIXTURES=` or `ZED_REF=`.
- Per-checkout things stay apart: each worktree has its own `target/`, scratch library and screenshot folder, named after the checkout's folder. Never point two checkouts at the same scratch library.
- The thumbnail cache and the recent libraries list are per machine and shared by every checkout. That's safe: thumbnails are keyed by file content, not path.

## Architecture

There's a Rust core, and a UI on top of it.

- **Core (Rust, decided).** It owns the `.folio/` format, scanning, preview extraction, grouping, filtering, move plans and undo.
  - **Pure orchestration.** The logic that decides what happens takes data in and returns data out: files and metadata in, groups, filter results and move plans out. It does no file I/O, uses no clock and depends on no UI. Unit test it.
  - **I/O at the edges.** File access, the `.folio/` store, the thumbnail cache, XMP reading and writing, and the Affinity launcher sit at the edges. They're allowed to be defensive. Everything inside them assumes clean data.
- **UI (GPUI if the spike passes, Tauri as the fallback).** The UI renders state and sends intents back to the core. No business logic in the UI.
- **Folders are the primary structure.** Files physically live in subject folders such as `Category/Outing/`. Ratings, labels, keywords and styles are metadata.
- **Per-machine data stays out of the library.** Thumbnails, the search index, window state and app paths go in the OS cache or app-data directory. All of it can be rebuilt from the library.

### `.folio/` format rules

The format is described in `docs/format.md`. Every change to it follows these rules.

- **Plain-text files, split small.** Two machines edit the library through file sync (iCloud, Dropbox, and so on). Never put SQLite or any other single binary database in `.folio/`. It will corrupt or produce conflict copies. A database is fine in the local cache if it's rebuilt from `.folio/`.
- **Relative paths only, with `/` separators.**
- **Normalize to Unicode NFC** when storing and comparing paths. macOS often returns NFD, and Windows uses NFC. A folder like `Märkte` breaks without this.
- **Don't rely on case sensitivity** when matching files.
- **Plan for files moved outside Folio.** Keep metadata attached through a re-link strategy, not just the path.
- **Detect sync conflict copies** (for example `name 2.json` from iCloud or `name (conflicted copy).json` from Nextcloud) and merge them automatically, per value, without any UI. Show the user something only when a copy can't be merged, such as when it's corrupt. Never ignore them. Folio doesn't sync and doesn't know about machines, so never word anything as "this Mac vs. that PC".
- **Avoid conflicts in the first place.** Split the format so two machines rarely touch the same file, and never rewrite a whole file from stale in-memory state. A network share has no conflict copies, but two open windows can overwrite each other.
- **Standard XMP fields go to the sidecars.** Ratings (`xmp:Rating`), labels and keywords belong in XMP sidecars so other apps can read them. The sidecar is the source of truth for marks, not `.folio/`.

## Rules

- **Never touch a real photo library.** Develop against copies in fixture folders. Moving, renaming or deleting real photos needs an explicit go-ahead.
- **Never commit the owner's photos.** That includes screenshots, mockups and thumbnails that show them. Keep those outside the repo (for example in `/tmp`). Images in the repo are brand artwork or synthetic test data only.
- **RAW files are read-only.** Folio never writes into NEF, CR3, DNG or any other RAW file. Metadata goes to `.xmp` sidecars or `.folio/`.
- **Moves always come with an undo log** that survives a crash partway through. Sidecars move with their RAW file.
- **Fixtures prove correctness.** Each fixture is a sample library plus its expected result, and core behaviour gets tested against fixtures. Keep tests focused. No snapshot dumps or smoke tests that assert nothing.
- **Make impossible states unrepresentable.** Use enums over structs full of `Option` fields. No `unwrap()` or `expect()` outside tests and startup code. Handle the error, or return it.
- **Keep it simple.** No abstractions, config options or plugin systems until a real need shows up. Prefer one well-known crate over several small ones, and don't add a crate to save ten lines.
- **Dependencies must be GPL-3.0 compatible.** MIT, Apache-2.0, BSD, MPL-2.0, LGPL and GPL are fine. Check the license before adding any dependency.
- **Comments** explain why, sit above functions and types, and get updated whenever the code changes.

## Writing style

This applies to commits, PR descriptions, docs and comments.

- Lead with what changed and why. No list of filenames.
- Use plain words and short sentences. No em dashes or en dashes.
- Use sentence-case headings.

## Roadmap

1. **GPUI spike (one to two days).** Build only the culling loop against a fixture copy of about 800 NEFs:
   - A virtualized thumbnail grid that scrolls smoothly.
   - Arrow keys move the selection, and 1 to 5 sets a star rating, kept in memory only.

   Put thumbnail extraction in the core crate from the start, so it carries over if the UI changes. Pass means the grid flies and progress was steady: continue with GPUI. Fail means hours lost fighting the API: stop, report, and propose Tauri on the same core.
2. **Design the `.folio/` format.** Done, see `docs/format.md`.
3. **v1 features:**
   - Open a folder as a library.
   - Scan files and read EXIF.
   - Group photos into outings by time gaps.
   - Show the contact sheet with suggested names and categories.
   - Merge, split and rename groups, then approve moves with undo.
   - Ratings, color labels and pick or reject flags, all from the keyboard.
   - Filters and saved views.
   - Open in Affinity.
4. **Later:** auto-categorisation with a local model, face recognition, similarity search.

Out of scope: RAW editing (that's the editor's job) and Folio's own cloud sync (Folio uses whatever syncs the folder).

### Decisions already made

Don't bring these up again without new information.

- **Two native apps (Swift plus WinUI and C#): rejected.** The owner wants one unified UI designed once.
- **Electron: rejected.** Speed matters more than its advantages.
- **Tauri: fallback only.** It renders through WebKit on macOS and WebView2 on Windows, so the UI needs testing on both.

## Skills

Project skills live in `.claude/skills/`. Use `gpui` before writing any GPUI code.
