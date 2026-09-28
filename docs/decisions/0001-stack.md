# ADR 0001 — Stack selection

Status: accepted.

## Context

We need a cross-platform (macOS/Linux/Windows) terminal. Prior art: Alacritty,
WezTerm, Rio (all Rust), Ghostty (Zig + platform UI). Rebuilding a VT core is
expensive and not the differentiator; rendering and app features are the work.

## Decision

- **PTY**: `portable-pty` (MIT; Unix `forkpty`, Windows **ConPTY**).
- **VT / grid / scrollback**: `alacritty_terminal` (Apache-2.0) + `vte` (Apache-2.0).
- **Window/input**: `winit`.
- **GPU**: `wgpu` (Metal / Vulkan / DX12).
- **Glyph rendering**: custom grid on wgpu via `glyphon` / `cosmic-text` / `swash`.
- **Chrome (panels/settings)**: `egui` + `egui-wgpu`, co-rendered with the
  terminal in one frame (terminal via `PaintCallback`).
- **Reference only**: Rio's `rio-vt` / `sugarloaf`, WezTerm's `termwiz`, and
  `libghostty` are *not* used as the base (churn / instability / no Windows).

## Consequences

- Fast path to a usable, fast terminal; Apache/MIT throughout (embeddable).
- We own the renderer and app layer; Ghostty's render/shell/config ecosystem is
  not reused (provide config import for migration).
- Risk concentrated in the self-drawn renderer and Windows IME — gated by R0 /
  R0.5 spikes.
