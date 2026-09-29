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
> stable. macOS is the primary platform. Windows is built, tested and driven over
> MTP on real hardware (see [`docs/WINDOWS-DEV.md`](docs/WINDOWS-DEV.md)); Linux
> builds and passes tests in CI.

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
- Inline tab bar (icons, agent badges, `+`, `×`, drag to reorder) plus a Tabs
  sidebar with a context menu (Rename / Prefix / Mark / Group / Duplicate /
  Close / Close Others).
- A recursive split tree: `⌘D` splits right and `⇧⌘D` splits down, with
  draggable dividers between panes. `⌘⇧T` toggles a scratch Quick Terminal.
- Sidebar **file tree** (double-click opens the reader) and **View rules**: map
  a pane's cwd/command/agent/host/file to an alias, icon, tab title and badge —
  see [`docs/VIEW-RULES.md`](docs/VIEW-RULES.md).
- **Open Quickly** (`⌘K`): one palette over tabs, agents, files, recents,
  content-search hits and commands.
- A right-hand details panel with tabs: **Info, Agent, Outline, Git, Files,
  Ports, Queue** (git status, directory listing, listening ports, prompt queue).
- **Composer** (`⌘⇧E`) and a **prompt queue** that sends to an agent pane once it
  is idle; **notifications** and a **sleep guard** driven by agent state.
- **Reader/editor** (`⌘`-open): read-only preview with line numbers and a
  jump-to-line highlight, edit mode with a gutter and an opt-in minimal vim mode
  (`editor-vim`), *Open Externally* / *Edit in
  Tab*, and a CommonMark renderer (`egui_commonmark`): headings, lists, quotes,
  tables, code, links, local and remote images, plus a `graph`/`flowchart`
  Mermaid subset (or full Mermaid via `mermaid-command`).
- **Inline terminal graphics**: Sixel, Kitty and iTerm2 images are drawn in the
  grid — they scroll with the content and are clipped to the pane. Toggle with
  `graphics` (on by default).
- **Recipes**: save and replay a whole workspace; config export.
- A settings window (`⌘,`): font size/family, opacity, line height, cursor
  style, theme, agent badges, notifications, sleep guard, agent integrations,
  View rules — persisted to `config.toml` / `views.json`.

**Configuration and integration**
- Configuration at `~/.config/miaotty/config.toml`: font size, font family,
  opacity, line height, cursor style, theme, colors, `language` (English or
  Simplified Chinese), `editor`, agent toggles, `quick-terminal-hotkey`,
  `update-check-url` / `update-pubkey` — see
  [`docs/config.example.toml`](docs/config.example.toml).
- Automatic import of ghostty `config` and alacritty `alacritty.toml` when no
  miaotty config exists.
- zsh shell integration (cwd via OSC 7, command history) installed through a
  `ZDOTDIR` shim — the user's dotfiles are never modified.
- **URL schemes**: `miaotty://`, `ssh://` and `x-man-page://` open a tab with the
  matching command; a second launch is forwarded to the running instance
  (single instance, including "focus pane" and "quick" intents).
- **Global Quick Terminal hotkey**: `global-hotkey` on macOS/Windows, the
  `GlobalShortcuts` desktop portal on Linux (plus compositor bindings for
  sway/hyprland/GNOME and friends).
- **Agent integrations**: detect claude/codex/opencode/miao, install a state hook
  script, copy the snippet that wires it into the agent's own config, and launch
  the agent — the user's agent config is never edited for them. `miao` reports
  its state from a built-in integration, so it needs no hook wiring.
- **Updates**: check a manifest, download the platform artifact, verify its
  SHA-256 (and a minisign signature when configured), and on macOS install and
  relaunch with a rollback helper — see [`docs/decisions`](docs/decisions).
- **Remote view/edit**: read and write a remote file over the pane's ssh
  ControlMaster connection, with a zero-install terminfo bootstrap.

**Automation**
- The **MTP control plane** over a per-user Unix socket (a named pipe on
  Windows): `core.ping/health`, `agent.state.*`, `history.*`,
  `pane.list/send/run/focus/close`, `app.view/edit` (open a file in the app) and
  `file.read/write` (bounded to 2 MB).
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
miaotty-cli view /path/to/file            # open it read-only in the app
miaotty-cli edit /path/to/file            # open it in the editor
miaotty-cli file read  --path /etc/hosts  # bounded to 2 MB per call
miaotty-cli file read  --path app.bin --base64 --offset 0 --length 65536
miaotty-cli file write --path /tmp/x --data "hello"
miaotty-cli file write --path /tmp/x --data-b64 "AAECAw=="   # binary
```

**Remote access**: set `remote-listen = "127.0.0.1:7273"` (and
`MIAOTTY_MTP_TOKEN`) to serve the control plane over TCP; connect with
`miaotty-cli --socket tcp://host:7273`. Without a token the TCP listener refuses
to start. Note the control plane runs commands in your shell, so keep the token
secret (and prefer a loopback/listen address you trust, or an ssh tunnel).

If the host was started with `MIAOTTY_MTP_TOKEN`, requests must carry it; the CLI
picks it up from the same variable. `MIAOTTY_MTP_ALLOW` (comma-separated, e.g.
`core.basic,file.read,history.read`) restricts which capabilities are accepted —
anything else returns `forbidden`. Unset means everything is allowed, and
`core.basic` (ping/health) is always allowed so clients can discover the host;
`ping` reports the effective set in `allowed`. The socket can be forwarded over
ssh (`ssh -R /tmp/fwd.sock:<host socket>`) so a remote client drives the host.

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
| `⌘K` | Open Quickly (tabs, agents, files, commands) |
| `⌘F` | Find |
| `⌘⇧E` | Composer (multi-line prompt to the focused pane) |
| `⌘⇧T` | Quick Terminal (scratch tab) |
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
| View rules (titles/icons/badges) | [`docs/VIEW-RULES.md`](docs/VIEW-RULES.md) | [`docs/VIEW-RULES.zh-CN.md`](docs/VIEW-RULES.zh-CN.md) |
| Performance budgets & gate | [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) | [`docs/PERFORMANCE.zh-CN.md`](docs/PERFORMANCE.zh-CN.md) |
| Windows dev/verification | [`docs/WINDOWS-DEV.md`](docs/WINDOWS-DEV.md) | [`docs/WINDOWS-DEV.zh-CN.md`](docs/WINDOWS-DEV.zh-CN.md) |
| Releasing & updates | [`docs/RELEASE.md`](docs/RELEASE.md) | [`docs/RELEASE.zh-CN.md`](docs/RELEASE.zh-CN.md) |
| Working agreement (privacy, checks) | [`AGENTS.md`](AGENTS.md) | — |
| Example configuration | [`docs/config.example.toml`](docs/config.example.toml) | — |

---

## Roadmap

Done recently: the Windows named-pipe transport and ConPTY path (verified on
real hardware), session restore, View rules, Open Quickly, the details panels,
the agent loop (notifications, sleep guard, prompt queue), recipes, remote
view/edit over ssh, update download/verify/install, URL schemes, the global
Quick Terminal hotkey, i18n, and a performance gate.

Still open:

- **Release chain**: obtain the signing credentials and run the pipeline end to
  end — a minisign key with its published public key, Apple notarization
  (Developer ID certificate + app password) and Windows MSI signing (a CA
  certificate). The workflows are already wired; only the secrets are missing
  (`docs/RELEASE.md`).
- **Platform verification**: the Linux wgpu render path runs in CI via Mesa
  software Vulkan (lavapipe); a real Linux desktop, the Wayland portal hotkey
  and Windows IME/GUI still need an interactive session.
- **Update install**: implemented on all three platforms (macOS app bundle,
  Windows MSI/zip helper, Linux AppImage); verified on macOS, the Windows/Linux
  paths still need a real host to confirm.
- **CI performance baseline**: bound through `actions/cache` today; a durable
  baseline store would make the gate robust across cache eviction.
- **Native parity**: URL schemes are handled from argv but the native binary is
  not registered with the OS. `background-opacity` works only where the surface
  offers straight alpha.
- **Inline graphics**: anchors are exact up to the scrollback cap and
  approximate past it (alacritty exposes no scroll counter without a patch);
  session restore keeps no images (they would not match the restored content).
- **Markdown**: Mermaid renders a `graph`/`flowchart` subset (or fully via
  `mermaid-command`); other diagram types show a placeholder.
- **i18n**: the main chrome is covered; a few example/hint strings stay English.
- **MTP**: `file.read/write` support binary (base64) with `offset`/`length`
  chunking; there is no server-push streaming. Per-capability authorization is
  available via `MIAOTTY_MTP_ALLOW`.

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
