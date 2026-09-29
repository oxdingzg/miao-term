# miao-term

**A fast, embeddable, cross-platform terminal emulator and engine, written in Rust.**

[![CI](https://github.com/oxdingzg/miao-term/actions/workflows/ci.yml/badge.svg)](https://github.com/oxdingzg/miao-term/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](Cargo.toml)
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

The application ships as two hosts that share the engine and the chrome:

- **`miaotty`** (`miaotty-app`) — the feature-complete host, an `eframe`/`egui`
  application; the grid is drawn through an `egui-wgpu` paint callback.
- **`miaotty-native`** (`miao-term-widget`) — the newer host with its own
  `winit` + `wgpu` event loop that draws the grid directly and overlays the egui
  chrome in the same frame, for lower input latency. It adds picture-in-picture,
  hint mode, read-only panes and per-pane close buttons.

The engine and the application are deliberately decoupled: the hosts are the
engine's first consumers, and the engine is designed to be embedded by others.
See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the full design.

> **Project status — pre-release.** The version is `0.0.1` and the API is not yet
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
  Close / Close Other Tabs / Close Below / Remove from Group).
- A recursive split tree: `⌘D` splits right and `⇧⌘D` splits down, with
  draggable dividers and a close button on every pane. `⌘⇧T` toggles a scratch
  Quick Terminal.
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
  grid by both hosts — they scroll with the content and are clipped to the pane.
  Toggle with `graphics` (on by default).
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
  matching command; a second launch — of either host — is forwarded to the
  running instance (single instance, including "focus pane" and "quick"
  intents) through a shared inbox beside the control socket.
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
(`host → render → core`), and the two hosts share everything above the engine.

| Crate | Responsibility |
|-------|----------------|
| [`miao-term-core`](crates/term-core) | PTY, VT parsing, grid/scrollback, selection, search, OSC, input encoding. No GPU or windowing. |
| [`miao-term-graphics`](crates/term-graphics) | Inline-graphics stream scanner and decoders (Sixel, Kitty, iTerm2). |
| [`miao-term-render`](crates/term-render) | `wgpu` + `glyphon` glyph-grid renderer, quad and image pipelines. |
| [`miao-term-ui`](crates/term-ui) | Host-agnostic UI shared by both hosts: theme, input encoding, selection, split layout, egui chrome, palette, hints, vim, markdown, ssh, update and agent-integration helpers. |
| [`miao-term-config`](crates/term-config) | Configuration and themes, ghostty/alacritty import, and the View-rule engine. |
| [`miao-term-mtp`](crates/term-mtp) | MTP protocol, host/client and transport (Unix socket, Windows named pipe, TCP). |
| [`miao-term-widget`](crates/term-widget) | The `miaotty-native` host: `winit` + `wgpu` render loop that draws the grid directly and composites the egui chrome. |
| [`miaotty-app`](miaotty-app) | The `miaotty` eframe host: tabs, panes, panels, settings. |
| [`miaotty-cli`](miaotty-cli) | The `miaotty-cli` control client. |

The hot path — `pty → vt → grid → renderer` — takes no locks and allocates
nothing per frame. Platform differences are confined to small `#[cfg]`-guarded
blocks in the crate that owns the concern: PTY spawning in `term-core`
(`src/term.rs`), the window/event loop in `term-widget` (`src/lib.rs`), and the
socket/named-pipe transport in `term-mtp` (`src/lib.rs`).

### Principles

1. Engine and hosts are separate; the hosts are the engine's first consumers.
2. The hot path takes no locks and allocates nothing.
3. Build the application first, extract the library later — APIs are driven by real needs.
4. OS differences are confined to `#[cfg]`-guarded blocks in `term-core`, `term-widget` and `term-mtp`.
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

# Build and run the terminal (either host)
cargo run -p miaotty-app                                        # the eframe host
cargo run -p miao-term-widget --bin miaotty-native --release    # the native host
```

The first build compiles `wgpu`/`glyphon` and may take a few minutes.

### Testing and linting

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

CI (`.github/workflows/ci.yml`) runs on every push, PR and nightly. On pushes it
runs `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
`cargo check` on Linux and macOS, and the privacy scan; the full three-OS
`cargo test --workspace`, the Linux software-Vulkan render test, the Windows job
and the release-mode performance gate run on pull requests, the nightly schedule
and manual dispatches. Docs-only pushes skip the compile and lint jobs.

### Packaging

```sh
scripts/package-macos.sh          # -> dist/miaotty.app (ad-hoc signed)
```

Release builds are produced by [`.github/workflows/release.yml`](.github/workflows/release.yml)
on `v*` tags: `miaotty` and `miaotty-cli`, the `miaotty-native` host, the macOS
app bundle, a Linux `.deb`/AppImage and a Windows MSI.
[`dist-workspace.toml`](dist-workspace.toml) is a
[cargo-dist](https://opensource.axo.dev/cargo-dist/) scaffold. See
[`docs/INSTALL.md`](docs/INSTALL.md) and [`docs/RELEASE.md`](docs/RELEASE.md).

---

## Configuration

miaotty reads `~/.config/miaotty/config.toml` (or
`$XDG_CONFIG_HOME/miaotty/config.toml`). Every key is optional; see
[`docs/config.example.toml`](docs/config.example.toml) for the full reference.

```toml
font-size   = 13                 # default 13
font-family = "JetBrains Mono"   # default; falls back to the system monospace
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
`MIAOTTY_SOCKET` and `MIAOTTY_PANE_ID`. Every response carries a state
`revision`; `core.wait` blocks until it moves past a given value, so a client
can follow agent state, panes or history without polling in a loop.

`miaotty-cli` is the reference client:

```sh
miaotty-cli ping
miaotty-cli wait --since 42               # block until the state revision moves
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
| `⌘⇧Z` | Reopen the last closed tab |
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

- **Release chain**: run the pipeline end to end with real credentials. The
  workflows are already wired and the minisign **public** key is committed and
  attached to every release; the missing pieces are the secrets — the minisign
  secret key, Apple notarization (Developer ID certificate + app password) and
  Windows MSI signing (a CA certificate). See [`docs/RELEASE.md`](docs/RELEASE.md).
- **Platform verification**: the Linux wgpu render path runs in CI via Mesa
  software Vulkan (lavapipe); a real Linux desktop, the Wayland portal hotkey
  and Windows IME/GUI still need an interactive session.
- **Update install**: implemented on all three platforms (macOS app bundle,
  Windows MSI/zip helper, Linux AppImage). The MSI and the `.deb` are verified on
  real hosts; the AppImage install and the self-replace path still need an
  end-to-end run (`docs/RELEASE.md`).
- **CI performance baseline**: bound through `actions/cache` today; a durable
  baseline store would make the gate robust across cache eviction.
- **Native parity** (`miaotty-native`): both hosts parse argv intents, share the
  forwarding inbox, implement the Quick Terminal and render inline IME
  composition at the cursor; neither is registered with the OS yet, so a link
  only reaches them when the launcher passes the URL as an argument.
  `background-opacity` works only where the surface offers straight alpha.
- **Inline graphics**: Kitty z-index is not modelled (images paint over the
  grid); anchors are exact up to the scrollback cap and approximate past it
  (alacritty exposes no scroll counter without a patch), and session restore
  keeps no images (they would not match the restored content).
- **Markdown**: Mermaid renders a `graph`/`flowchart` subset (or fully via
  `mermaid-command`); other diagram types show a placeholder.
- **i18n**: the main chrome is covered; a few example/hint strings stay English.
- **MTP**: `file.read/write` support binary (base64) with `offset`/`length`
  chunking. Clients follow changes without busy-polling via the `core.wait`
  long-poll, which returns as soon as the state revision moves; true server-push
  streaming is not implemented. Per-capability authorization is available via
  `MIAOTTY_MTP_ALLOW`.

---

## Contributing

Contributions are welcome. Before opening a pull request:

1. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets` and
   `cargo test --workspace`.
2. Keep documentation bilingual: when you change a `doc`, update its
   `doc.zh-CN.md` counterpart in the same change (for example `README.md` /
   `README.zh-CN.md`, or `docs/RELEASE.md` / `docs/RELEASE.zh-CN.md`).
3. For changes to a *locked* design decision, add an ADR under
   [`docs/decisions/`](docs/decisions/README.md) rather than editing the record
   in place.

By contributing, you agree that your contributions are licensed under the
project's license (Apache-2.0).

---

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
