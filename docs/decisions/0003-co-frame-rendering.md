# ADR 0003 — Terminal + egui co-frame rendering

Status: accepted.

## Context

We want egui for chrome (panels, settings) and a custom GPU renderer for the
terminal grid, in one window without fighting over the swapchain.

## Decision

- **egui drives the frame** (eframe + wgpu). The terminal is drawn in the same
  `wgpu::RenderPass` via an **`egui_wgpu` `PaintCallback`**.
- The glyph pipeline lives in `term-render` (wgpu + glyphon) and is stored in
  egui's `callback_resources`; it shares egui's `Device`/`Queue`/`Surface`
  (same `wgpu` version — currently 23).
- Backgrounds, selection and cursor are drawn by the egui painter *underneath*;
  the callback draws the glyphs on top.

## Consequences

- One pass, one device; no second surface or swapchain.
- `wgpu` must stay pinned to egui-wgpu's version (a mismatch would defeat device
  sharing) — hence `glyphon` is pinned to a wgpu-23-compatible release (0.7).
- Each pane owns its own `TermRenderer` (its own glyph atlas), so a pane keeps
  its prepared glyphs; glyphs are re-shaped only when that pane's content is
  dirty (damage), and idle frames submit no text work.
- Update: for the native host, ADR 0030 superseded this — the grid is drawn by
  `term-widget`'s native `winit` + `wgpu` loop instead of an egui
  `PaintCallback`. Co-frame rendering still applies to the `eframe` host.
