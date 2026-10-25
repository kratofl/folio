# Agent guide

Folio is a fast, cross-platform (macOS and Windows) photo organizer. A folder of photos becomes a library, and the app's state lives in a hidden `.folio/` folder at its root, like `.git`. Read `README.md` for the product pitch. This file covers how to work on the code.

The project is written almost entirely by AI agents. That makes the rules below more important, not less. The owner won't read every line, so tests and fixtures are what prove the code is correct.

## Current status

No code yet. Next step: the GPUI spike (see "Roadmap").

## Architecture

There's a Rust core, and a UI on top of it.

- **Core (Rust, decided).** It owns the `.folio/` format, scanning, preview extraction, grouping, filtering, move plans and undo.
  - **Pure orchestration.** The logic that decides what happens takes data in and returns data out: files and metadata in, groups, filter results and move plans out. It does no file I/O, uses no clock and depends on no UI. Unit test it.
  - **I/O at the edges.** File access, the `.folio/` store, the thumbnail cache, XMP reading and writing, and the Affinity launcher sit at the edges. They're allowed to be defensive. Everything inside them assumes clean data.
- **UI (GPUI if the spike passes, Tauri as the fallback).** The UI renders state and sends intents back to the core. No business logic in the UI.
- **Folders are the primary structure.** Files physically live in subject folders such as `Category/Outing/`. Ratings, labels, keywords and styles are metadata.
- **Per-machine data stays out of the library.** Thumbnails, the search index, window state and app paths go in the OS cache or app-data directory. All of it can be rebuilt from the library.

### `.folio/` format rules

These rules are set. The rest of the format is still being designed.

- **Plain-text files, split small.** Two machines edit the library through file sync (iCloud, Dropbox, and so on). Never put SQLite or any other single binary database in `.folio/`. It will corrupt or produce conflict copies. A database is fine in the local cache if it's rebuilt from `.folio/`.
- **Relative paths only, with `/` separators.**
- **Normalize to Unicode NFC** when storing and comparing paths. macOS often returns NFD, and Windows uses NFC. A folder like `Schäferlauf` breaks without this.
- **Don't rely on case sensitivity** when matching files.
- **Plan for files moved outside Folio.** Keep metadata attached through a re-link strategy, not just the path.
- **Detect sync conflict copies** (for example `name 2.json`) and merge them or show them to the user. Never ignore them.
- **Standard XMP fields go to the sidecars.** Ratings (`xmp:Rating`) and keywords also belong in XMP sidecars so other apps can read them. Which side is the source of truth is still undecided. Ask before choosing.

## Rules

- **Never touch a real photo library.** Develop against copies in fixture folders. Moving, renaming or deleting real photos needs an explicit go-ahead.
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
2. **Design the `.folio/` format** with the owner, before building features.
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
