# ADR 0030 — Native render loop (self-drawn grid)

Status: accepted.

## Context

`miaotty-app` bootstrapped on `eframe`/`egui`: the terminal grid is drawn through
an egui `PaintCallback` and every interaction rides egui's immediate-mode frame.
That was fast to build but caps the input-to-photon latency at the framework's
event-loop + present cadence, which is visibly slower than a native terminal
(Ghostty / Alacritty).

The engine was designed for this: `term-core` owns the hot path with no GPU or
windowing dependency, and `term-render` owns fonts/atlas/draw passes with no
event loop (see ARCHITECTURE §3, D3).

## Decision

Move the terminal grid onto a **native `winit` + `wgpu` render loop** owned by
`term-widget` (the `miao-term-widget` package, bin `miaotty-native`), and stop
routing it through egui:

- `term-widget` owns the `winit` event loop and the `wgpu` surface.
- `term-render` owns the passes: background/selection/cursor quads plus glyphs,
  drawn directly into the surface (no egui mesh).
- The render loop is **damage-driven and event-driven**: the PTY reader thread
  wakes the loop (`EventLoopProxy` → `request_redraw`) the moment output arrives,
  so echo is drawn on the next frame; idle costs ~0.
- egui remains available for chrome (panels/settings) and composes in the *same*
  frame once the chrome exists (D3), but the grid never depends on it.
- Present mode is `AutoNoVsync` and `desired_maximum_frame_latency = 1`, to
  minimise latency.

## Consequences

- Input echo is one frame from PTY output → latency comparable to native
  terminals; the grid is decoupled from egui's frame cadence.
- `term-widget` grows a real host: window, surface, input, IME, clipboard,
  resize, redraw scheduling. `term-render` gains a quad pipeline next to the
  glyph renderer.
- The existing `eframe` app stays as the feature-complete chrome host until the
  native host reaches parity; both consume the same engine crates.
