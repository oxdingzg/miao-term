# miao-term

A cross-platform (macOS / Linux / Windows) terminal **engine** written in Rust, plus
the `miaotty` application built on top of it.

> Status: **scaffold**. Nothing usable yet — this is the R0 skeleton
> (`term-core` → `term-render` → `term-widget`).
>
> Design: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) ([简体中文](docs/ARCHITECTURE.zh-CN.md)) ·
> decisions: [`docs/decisions/`](docs/decisions/). Docs are English by default with a
> synced `*.zh-CN.md` Simplified Chinese version.

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

MIT OR Apache-2.0.
