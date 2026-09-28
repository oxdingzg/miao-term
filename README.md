# miao-term

**A fast, embeddable, cross-platform terminal emulator and engine, written in Rust.**

[![CI](https://github.com/oxdingzg/miao-term/actions/workflows/ci.yml/badge.svg)](https://github.com/oxdingzg/miao-term/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](rust-toolchain.toml)
[![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)](#requirements)

**English** · [简体中文](README.zh-CN.md)

---

## Overview

`miao-term` is two things in one repository:

- a **terminal engine** — `miao-term-core` owns the hot path from the PTY to the
  screen and depends on no windowing or GPU code;
- **`miaotty`** — a batteries-included terminal application built on that engine,
  with tabs, panes, side panels, a settings window, shell integration and a
  scriptable control plane.

The engine and the application are deliberately decoupled: `miaotty` is the
engine's first consumer, and the engine is designed to be embedded by others.
See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the full design.

> **Project status — pre-release.** The version is `0.0.0` and the API is not yet
> stable. macOS is the primary, verified platform; Linux and Windows build in
> CI but are not yet fully tested in practice.

---

## Features

**Terminal core**
- PTY-backed shells with VT parsing via `alacritty_terminal`.
- GPU glyph-grid rendering (`wgpu` + `glyphon`), reusing egui's device, queue
  and surface.
- Scrollback with a scrollbar indicator, drag and double-click selection,
  copy/paste, and wide-character / CJK layout with a system CJK font fallback.
- Find (`⌘F`) with match highlighting and a result count.
- The [kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)
  (CSI-u) and modifier-aware navigation keys when a program requests them.

**Window and workspace**
- Inline tab bar (`+`, `×`) plus a Tabs sidebar with a context menu
  (Rename / Duplicate / Close / Close Others).
- A recursive split tree: `⌘D` splits right and `⇧⌘D` splits down, with
  draggable dividers between panes.
- A right-hand details panel: working directory, agent state, and per-pane
  command history.
- A settings window (`⌘,`) for font size, font family and theme, persisted to
  `config.toml`.

**Configuration and integration**
- Configuration at `~/.config/miaotty/config.toml`: font size, font family,
  built-in named themes, or explicit colors.
- Automatic import of ghostty `config` and alacritty `alacritty.toml` when no
  miaotty config exists.
- zsh shell integration (cwd via OSC 7, command history) installed through a
  `ZDOTDIR` shim — the user's dotfiles are never modified.

**Automation**
- The **MTP control plane** over a per-user Unix socket (Windows named pipe is
  the planned transport): `core.ping/health`, `agent.state.*`, `history.*`,
  `pane.list/send/run/focus/close`.
- **`miaotty-cli`**, a cross-platform client for the control plane.

---

## Architecture

The engine is layered so that dependencies point inward only
(`widget → render → core`), and OS-specific code is confined to a small number
of modules.

| Crate | Responsibility |
|-------|----------------|
| [`miao-term-core`](crates/term-core) | PTY, VT parsing, grid/scrollback, selection, search, OSC, input encoding. No GPU or windowing. |
| [`miao-term-render`](crates/term-render) | `wgpu` + `glyphon` glyph-grid renderer. |
| [`miao-term-widget`](crates/term-widget) | `winit` integration, input/IME/clipboard, egui composition. |
| [`miao-term-config`](crates/term-config) | Configuration and themes, plus ghostty/alacritty import. |
| [`miao-term-mtp`](crates/term-mtp) | MTP protocol, host/client and transport. |
| [`miaotty-app`](miaotty-app) | The `miaotty` binary: tabs, panes, panels, settings. |
| [`miaotty-cli`](miaotty-cli) | The `miaotty-cli` control client. |

The hot path — `pty → vt → grid → renderer` — takes no locks and allocates
nothing per frame. Platform differences live only in `core::pty`,
`widget::platform` and `mtp::transport`.

### Principles

1. Engine and application are separate; `miaotty-app` is the engine's first consumer.
2. The hot path takes no locks and allocates nothing.
3. Build the application first, extract the library later — APIs are driven by real needs.
4. OS differences are confined to `core::pty`, `widget::platform`, `mtp::transport`.
5. The control plane (MTP / CLI) is decoupled from the engine.

---

## Requirements

- The Rust **stable** toolchain (pinned in [`rust-toolchain.toml`](rust-toolchain.toml); MSRV 1.80).
- A GPU and driver supporting Metal (macOS), Vulkan (Linux) or DX12 (Windows).
- On Linux, the usual `winit`/`wgpu` system libraries (X11 or Wayland development
  packages) must be available.

---

## Getting started

```sh
git clone https://github.com/oxdingzg/miao-term.git
cd miao-term

# Build and run the terminal
cargo run -p miaotty-app          # or: cargo build --release && ./target/release/miaotty
```

The first build compiles `wgpu`/`glyphon` and may take a few minutes.

### Testing and linting

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

CI runs `cargo check` and `cargo test` across macOS, Linux and Windows; see
[`.github/workflows/ci.yml`](.github/workflows/ci.yml).

### Packaging

```sh
scripts/package-macos.sh          # -> dist/miaotty.app (ad-hoc signed)
```

Release builds are produced by [`.github/workflows/release.yml`](.github/workflows/release.yml)
on `v*` tags. [`dist-workspace.toml`](dist-workspace.toml) is a
[cargo-dist](https://opensource.axo.dev/cargo-dist/) scaffold. See
[`docs/INSTALL.md`](docs/INSTALL.md).

---

## Configuration

miaotty reads `~/.config/miaotty/config.toml` (or
`$XDG_CONFIG_HOME/miaotty/config.toml`). Every key is optional; see
[`docs/config.example.toml`](docs/config.example.toml) for the full reference.

```toml
font-size   = 14
font-family = "JetBrains Mono"   # default: system monospace
theme       = "nord"             # nord | dracula | gruvbox | solarized | tokyo-night

[colors]                          # explicit colors override the named theme
background = "#2e3440"
foreground = "#d8dee9"
palette    = ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b",
              "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0",
              "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b",
              "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"]
```

If no miaotty config exists, ghostty's `config` and alacritty's
`alacritty.toml` are imported automatically.

### Shell integration

On startup, miaotty installs a zsh `ZDOTDIR` shim that reports the working
directory (OSC 7) and command history. The shim is written to a private,
user-only directory and restores the user's real `ZDOTDIR`, so existing dotfiles
are left untouched. No manual setup is required.

---

## Control plane

The **MTP** (miaotty terminal protocol) control plane speaks newline-delimited
JSON over `$XDG_RUNTIME_DIR/miaotty.sock` (falling back to `$TMPDIR`), and the
socket is created with owner-only permissions. The shell inherits
`MIAOTTY_SOCKET` and `MIAOTTY_PANE_ID`.

`miaotty-cli` is the reference client:

```sh
miaotty-cli ping
miaotty-cli pane list
miaotty-cli pane run --pane ID --data "echo hello"
miaotty-cli pane focus --pane ID
miaotty-cli state claude --state processing --pane ID
miaotty-cli state list
miaotty-cli history add --command "cargo test" --cwd "$PWD"
miaotty-cli history list --pane ID
```

Pass `--socket PATH` or set `MIAOTTY_SOCKET` to target a non-default socket.

---

## Keyboard shortcuts

| Shortcut | Action |
|----------|--------|
| `⌘T` | New tab |
| `⌘W` | Close the focused pane (or tab) |
| `⌘D` / `⇧⌘D` | Split right / split down |
| `⌥⌘→` / `⌥⌘←` (or `⌘⇧]` / `⌘⇧[`) | Cycle pane focus |
| `⌥⌘D` | Toggle the details panel |
| `⌘F` | Find |
| `⌘,` | Settings |
| `⌘+` / `⌘-` / `⌘0` | Increase / decrease / reset font size |
| `Shift+PgUp` / `Shift+PgDn` | Scroll the viewport |

On macOS, `⌘` is the command modifier; on other platforms, the equivalent
primary modifier is used.

---

## Documentation

Documentation is written in English by default, with a Simplified Chinese
version kept in sync as `*.zh-CN.md`.

| Document | English | 简体中文 |
|----------|---------|---------|
| Architecture & design | [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | [`docs/ARCHITECTURE.zh-CN.md`](docs/ARCHITECTURE.zh-CN.md) |
| Architecture decision records | [`docs/decisions/`](docs/decisions/README.md) | [`docs/decisions/README.zh-CN.md`](docs/decisions/README.zh-CN.md) |
| Installation | [`docs/INSTALL.md`](docs/INSTALL.md) | [`docs/INSTALL.zh-CN.md`](docs/INSTALL.zh-CN.md) |
| Example configuration | [`docs/config.example.toml`](docs/config.example.toml) | — |

---

## Roadmap

Not yet implemented:

- Windows named-pipe transport and ConPTY testing.
- Packaging: notarized macOS builds, Linux AppImage/Flatpak/`.deb`, Windows MSI.
- Renderer hardening: damage-driven atlas uploads and atlas trimming.
- Expanded config import and persisted window/split state.

---

## Contributing

Contributions are welcome. Before opening a pull request:

1. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets` and
   `cargo test --workspace`.
2. Keep documentation bilingual: when you change `doc.md`, update
   `doc.zh-CN.md` in the same change.
3. For changes to a *locked* design decision, add an ADR under
   [`docs/decisions/`](docs/decisions/README.md) rather than editing the record
   in place.

By contributing, you agree that your contributions are licensed under the
project's license (Apache-2.0).

---

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
