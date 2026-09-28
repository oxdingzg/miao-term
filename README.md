# miao-term

A cross-platform (macOS / Linux / Windows) terminal **engine** written in Rust, plus
the `miaotty` application built on top of it.

> Status: **usable R0/R1 bootstrap** (macOS verified; Linux/Windows untested).
> 简体中文: [`README.zh-CN.md`](README.zh-CN.md).
>
> Design: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) ([简体中文](docs/ARCHITECTURE.zh-CN.md)) ·
> decisions: [`docs/decisions/`](docs/decisions/). Docs are English by default with a
> synced `*.zh-CN.md` Simplified Chinese version.

## What works today

- Window, shell on a PTY, VT parsing (`alacritty_terminal`), GPU grid rendering,
  keyboard input, resize.
- Tabs with a left sidebar (`+`, click to switch, middle-click to close,
  right-click for Rename / Duplicate / Close / Close Others).
- Splits: a recursive split tree (any number of panes), `⌘D` (right) /
  `⇧⌘D` (down), click to focus, drag the divider to resize, `⌘W` closes the
  focused pane.
- Right details panel: Info (working directory + Copy Path / Reveal in Finder),
  Agent state, Outline (per-pane command history).
- Selection (drag + double-click word), copy/paste, scrollback (wheel + Shift+PgUp/PgDn).
- Find (`⌘F`) with match highlighting and a count.
- Wide/CJK layout and a system CJK font fallback.
- MTP control plane over a Unix socket: `core.ping/health`, `agent.state.*`,
  `history.*`, `pane.list`, `pane.send/run`, `pane.focus/close` — interoperates
  with the existing `miaotty-cli`.
- zsh shell integration (cwd via OSC 7, command history) installed via a `ZDOTDIR` shim.
- Config at `~/.config/miaotty/config.toml` (font size + colors) — see
  [`docs/config.example.toml`](docs/config.example.toml).
- **GPU glyph rendering**: the terminal grid is drawn by `term-render` (wgpu +
  glyphon) through an egui `PaintCallback`, sharing egui's device/queue/surface.

Config import: ghostty `config` and alacritty `alacritty.toml` are picked up
automatically when there is no miaotty config.

Packaging: `scripts/package-macos.sh` builds an ad-hoc-signed `dist/miaotty.app`;
`dist-workspace.toml` is a cargo-dist scaffold. See [`docs/INSTALL.md`](docs/INSTALL.md).

Keyboard: `⌘T` new tab, `⌘W` close pane/tab, `⌘D`/`⇧⌘D` split, `⌥⌘→`/`⌥⌘←`
(or `⌘⇧[`/`⌘⇧]`) cycle panes, `⌥⌘D` toggle details, `⌘F` find,
`⌘+`/`⌘-`/`⌘0` font size. Modifier-aware arrows (Ctrl/Alt = word
movement), Ctrl/Alt+Backspace, and the **kitty keyboard protocol** (CSI-u for
Esc/Enter/Tab/Backspace and Ctrl+key) when a program enables it.

Not yet: Windows named-pipe transport + ConPTY testing, packaging
(MSI/AppImage/notarize), and further renderer hardening (damage uploads, atlas
trim).

## Run

```sh
cargo run -p miaotty-app      # or: ./target/debug/miaotty
```

## Layout

```
miao-term/
  crates/
    term-core     PTY + vte + grid + term + selection/search + OSC + input encoding (no GPU)
    term-render   wgpu glyph-grid renderer
    term-widget   winit integration + input/IME/clipboard + egui composition
    term-config   config + themes (+ ghostty/alacritty import)
    term-mtp      MTP protocol + host/client + transport (Unix socket / Windows named pipe)
  miaotty-app     the product: window/tab/split chrome, panels, badges, settings
```

## Principles

1. Engine and application are separate; `miaotty-app` is the engine's first consumer.
2. The hot path (`pty → vt → grid → renderer`) takes no locks and allocates nothing.
3. Build the app first, extract the library later — APIs are driven by real needs.
4. OS differences live only in `core::pty`, `widget::platform`, `mtp::transport`.
5. The control plane (MTP/CLI) is decoupled from the engine.

## Target stack

`portable-pty` · `alacritty_terminal` · `vte` · `winit` · `wgpu` · `glyphon`/`cosmic-text`/`swash` · `egui`.

## Build

```sh
cargo check
```

## License

Apache-2.0. See [`LICENSE`](LICENSE).
