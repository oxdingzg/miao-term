# ADR 0031 — Application menu in the macOS menu bar

Status: accepted.

## Context

The chrome drew its own menu row (File / Edit / View / …) inside the window, the
same way `miaotty-app` and `miaotty-native` draw the rest of the chrome. On macOS
that reads as foreign: other terminals show the menu in
the system menu bar, and the window itself has no menu strip. `winit` 0.30 has no
menu API, so this needs a platform crate: `muda` (MIT, the tauri menu library).

`muda` turned out to be hazardous in two ways, both reproduced here:

- Its macOS icon path `unwrap`s while converting an icon to an `NSImage`. A
  process without an application icon produces a zero-sized icon, `png` rejects
  it, and the panic happens inside an AppKit menu callback that cannot unwind —
  clicking the standard *About* item aborted the app with
  `FormatError { inner: ZeroWidth }`.
- A native `NSMenuItem` keeps a raw pointer to the Rust-side `MenuChild`. Dropping
  the `MenuItem` after appending it (the obvious loop) leaves that pointer
  dangling, so clicking one of our items aborted with
  `CFString cannot be created from a negative number of bytes` (SIGTRAP).

## Decision

Put the menu in the system menu bar on macOS, and keep it honest:

1. One table, `miao_term_ui::menu::menus(lang)`, describes the menus. The native
   host builds the OS menu bar from it; `chrome::render` draws the in-window menu
   from the same table when the host says so (`Chrome::draws_menu_bar`).
2. The OS menu bar is installed **only on macOS and only when the binary runs
   from inside an `.app` bundle** (`menu_in_os()`). That is where the application
   icon lives — which AppKit's about panel needs — and it keeps a bare
   `target/release/miaotty-native` running the in-window menu, so a development
   run can never abort on a menu click. The bundle is produced by
   `scripts/package-macos.sh` (`miaotty-native.app`).
3. `muda` is **vendored** at `vendor/muda` with a minimal, documented patch
   (`vendor/muda/PATCHES.miao.md`): the zero-sized icon is encoded as a
   transparent 1×1 image and an undecodable image falls back to an empty
   `NSImage`, instead of panicking inside the callback.
4. Every `MenuItem`/`Submenu` the host creates is **kept alive** in the `Host`
   for the whole run (`appmenu::MenuHandle`), because muda's native items point
   into them.

The window also asks the OS for the dark appearance (`with_theme(Dark)`), because
the chrome is dark and a light title bar next to it looks like a foreign strip —
This app themes its whole frame the same way.

## Consequences

- macOS gets a native menu bar (About / Services / Hide / Quit included) and the
  window has no menu strip; Linux and Windows keep the in-window
  menu, and the eframe host is unchanged.
- Menu items carry real accelerators, so AppKit owns those shortcuts while the
  menu is installed (our key handler remains as a fallback for bare runs).
- We carry ~380 KB of third-party source in the repository and must re-apply one
  small patch when bumping `muda` (see the patch file). Accepted as the price of
  not shipping a menu that aborts the app.
- Verified by hand on the bundled build: About, Hide Others, Show All,
  File → New Tab, Shell → Split Right, Edit → Select All, View → Toggle Sidebar
  and Quit all work and the process survives.
