---
name: gpui
description: How to write GPUI (Zed's UI framework) code in Folio without guessing APIs. Use before writing or changing any UI code that uses gpui or gpui-component.
---

# Writing GPUI code

GPUI is Zed's GPU-rendered UI framework. It's pre-1.0, changes often and has thin docs, so models frequently invent APIs that don't exist or use old ones. Real source code is the only reliable reference.

## Before writing code

1. **Find the pinned version.** Check `Cargo.toml` for the exact git commit of `gpui`, and of `gpui-component` if it's used. GPUI must always be pinned to a commit, never a branch.
2. **Make sure the reference checkouts exist** at that same commit, in `.refs/` at the root of the main checkout. Git ignores that folder, since they're too big to vendor.
   - Zed: `git clone https://github.com/zed-industries/zed .refs/zed`, then `git -C .refs/zed checkout <commit>`.
   - gpui-component: `git clone https://github.com/longbridge/gpui-component .refs/gpui-component`, then check out its pinned commit.
   - In a git worktree, `.refs/` is the main checkout's, not the worktree's. `make paths` shows where it is. `make setup` warns when it's missing or at the wrong commit.
3. **Look up every API you plan to use** in those checkouts before you write code that calls it. The most useful places:
   - `.refs/zed/crates/gpui/examples/`: small runnable examples.
   - `.refs/zed/crates/gpui/src/`: the framework itself.
   - `.refs/zed/crates/`: Zed's own UI, which is real-world usage. Grep for the element or method you need.
   - `.refs/gpui-component/examples/`: ready-made components.

## While writing

- **Copy patterns from real code, then adapt them.** Don't write GPUI from memory.
- **If a method doesn't compile, look it up.** Search the checkout. Don't try variations until one compiles.
- **Keep views dumb.** They render state from the core and send intents back. No file I/O or business logic in GPUI code.
- **Use virtualized lists or grids for anything that scales with library size.** Never render all thumbnails at once.

## When to stop

If you're stuck on the same GPUI problem for more than about three attempts, stop. Report what you tried and what the source shows. During the spike, a pattern of this is the signal to fall back to Tauri, so the owner needs to hear about it.
