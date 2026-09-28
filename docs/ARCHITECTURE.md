# miao-term Architecture

> A cross-platform (macOS / Linux / Windows) terminal **engine** plus the `miaotty`
> application built on it.
> This is the pre-implementation design. **Locked** items are decisions; changing
> one requires an ADR under `docs/decisions/`.
> 简体中文版: [`ARCHITECTURE.zh-CN.md`](ARCHITECTURE.zh-CN.md).

## 0. Scope, in one sentence

**`portable-pty` + `alacritty_terminal` + `vte` as the core, `winit` + `wgpu` for the
window and drawing, the terminal grid self-drawn and the surrounding UI in egui; the
engine (`miao-term-*`) is separate from the app (`miaotty-app`), and the control plane
(MTP) is decoupled from the engine.**

## 1. Goals · Non-goals · Constraints

**Goals**
- One codebase on three platforms, **including Windows (ConPTY)**.
- Hot-path performance at Alacritty's level: input latency P95 ≤ 16 ms (target ≤ 8 ms),
  first frame ≤ 100 ms, no dropped frames while scrolling, idle CPU ≈ 0.
- The engine is embeddable (`examples/` proves it); `miaotty-app` is its first consumer.
- Reuse the existing control plane: `mtp` types, `miaotty-cli`, plugins, agent/shell hooks.

**Non-goals (for now)**
- Matching Ghostty's rendering polish or config ecosystem; macOS-only integrations
  (AppleScript/Sparkle).
- A plugin marketplace / "framework"-level extension system; SSH/remote access; cross-device sync.

**Constraints**
- Permissive licenses only (Apache-2.0/MIT/BSD/ISC); **no GPL/AGPL inside the engine**
  (e.g. Pebrel is read-only reference, never copied).
- Every new external crate passes a performance and license review.
- Outbound license: **Apache-2.0** for the engine and the app.
- Rust stable; MSRV in `rust-toolchain.toml`.

## 2. Locked decisions (summary)

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | Core uses `alacritty_terminal` + `vte` + `portable-pty` | Most mature, cross-platform (incl. ConPTY), same as Otty, permissive |
| D2 | Self-drawn rendering on `wgpu` (glyphs via `glyphon`/`cosmic-text`/`swash`) | Controllable performance; Metal/Vulkan/DX12 |
| D3 | Surrounding UI (panels/settings) uses `egui`, **co-rendered in one frame** with the terminal | Dev speed + a fast self-drawn terminal |
| D4 | **Own tab/split model** (not OS-native tabs) | Cross-platform consistency; matches Otty; controllable |
| D5 | Concurrency per **Alacritty's lock discipline** (`FairMutex<Term>` + `EventListener`) | Proven; avoids inventing a snapshot protocol |
| D6 | Platform differences only in `core::pty` / `widget::platform` / `mtp::transport` | Contain complexity |
| D7 | `term-mtp` decoupled from the engine (Unix socket / Windows named pipe) | A crash in one doesn't take down the other; reuse the protocol |
| D8 | Build the app first, extract the library later; phase the extension points | Real needs drive the API |
| D9 | Engine crates licensed `Apache-2.0` | Permissive; easy to embed |

## 3. Layering (DAG) and rules

```
            term-config      term-mtp          (independent, no engine deps)
                 │               │
   term-core ──► term-render ──► term-widget
        ▲                                ▲
        └──────────── miaotty-app ───────┘   (depends on all)
```

**Rules (enforced in CI)**
- Dependencies point inward only: `widget → render → core`; `core` depends on no other engine crate.
- `core` must not depend on `wgpu`/`winit`/`egui` (**it must build without a GPU**); `render` must not depend on `winit`.
- `config`/`mtp` must not depend on rendering or windowing.
- The app depends on the engine; the engine **never** depends on the app.
- `cargo-deny` for licenses/advisories; `cargo-machete` for unused deps.

## 4. Crate / module responsibilities

| Crate | Does | Does not |
|-------|------|----------|
| `term-core` | PTY, vte parsing, grid/scrollback/cursor/modes, selection/search, OSC/CSI semantics, key/mouse→bytes encoding, events (`EventSink`) | No GPU/window/config/business logic |
| `term-render` | Font load/shaping/atlas, grid instancing, draw passes, damage increments | No event loop/input |
| `term-widget` | winit event loop, wgpu surface, input/IME/clipboard/drag-drop, egui composition, `Host` callbacks | No tab/panel business |
| `term-config` | Config model, themes, ghostty/alacritty import | No UI |
| `term-mtp` | Protocol envelope, transport, server/client, agent/history registries, subscriptions | No engine dependency |
| `miaotty-app` | Windows/tabs/splits, left Tabs, right Details, badges, settings, OS integration, hook install | No terminal core duplication |

## 5. Core types and traits (Rust sketch)

```rust
// ---- term-core ----
pub struct Term { /* grid + scrollback + cursor + modes */ }
pub struct Damage { pub full: bool, pub lines: RangeSet<usize> }

pub enum TermEvent {
    Title(String), Cwd(PathBuf), Bell, Progress(Progress),
    ClipboardStore(String), ClipboardLoad(u8),
    PromptStart, CommandStart, CommandDone(i32),     // OSC 133
    PtyWrite(Vec<u8>), PtyExit(i32, Option<i32>),
    Wakeup,                                          // new output → needs redraw
}
pub trait EventSink: Send + 'static { fn send(&self, e: TermEvent); }

pub trait Pty: Send {
    fn write(&self, bytes: &[u8]) -> io::Result<()>;
    fn resize(&self, cols: u16, rows: u16) -> io::Result<()>;
    fn reader(&self) -> io::Result<Box<dyn Read + Send>>;  // owned by the reader thread
}

pub trait InputEncoder { fn encode(&self, ev: &InputEvent) -> Vec<u8>; }

// ---- term-render ----
pub struct GlyphAtlas { /* R8 texture + LRU */ }
pub trait Renderer {
    fn update(&mut self, term: &Term, damage: Damage);   // build instances (short lock)
    fn render(&mut self, frame: &mut wgpu::RenderPass);  // submit to GPU
}

// ---- term-widget ----
pub trait Host: Send + Sync {
    fn set_title(&self, s: &str);
    fn open_url(&self, url: &str);
    fn notify(&self, title: &str, body: &str);
    fn clipboard(&self, kind: Clipboard, data: Option<String>) -> Option<String>;
    fn request_redraw(&self);
}
```

## 6. Threading model and lock discipline

Use Alacritty's proven model (`FairMutex<Term>` + `EventListener`); do not invent a snapshot protocol.

```
┌─ PTY reader thread ────────────────────┐     ┌─ main thread (winit loop) ────────────┐
│ loop read():                           │     │ winit event:                          │
│   lock(term) { vte.process(chunk) }    │     │   Keyboard/IME → core::input → pty    │
│   mark Damage; sink.send(Wakeup)       │     │   RedrawRequested:                    │
│   (lock covers parsing only)           │     │     lock(term){ renderer.update }     │
└────────────────────────────────────────┘     │     renderer.render(frame); present   │
                                               └───────────────────────────────────────┘
```

**Invariants**
1. The `Term` lock covers **only** "parse one chunk" or "build render instances" — it never
   spans `read()`/`write()`/GPU submit.
2. The PTY reader thread **owns** the reader; writes use a separate handle (`Pty::write`) so
   reads and writes don't block each other (especially ConPTY).
3. No polling when idle; redraw is requested only by `Wakeup` / input / the cursor-blink timer.
4. Resize: the main thread computes `cols/rows` → `Pty::resize`; `Term` resizes in sync.
5. Exit status comes from **OSC 133;D**, not the process exit code (unreliable under ConPTY/wrappers).

## 7. Data flow

- **Output**: PTY → reader thread → `vte` → `Term` (+OSC → `TermEvent`) → `Wakeup` → main-thread render.
- **Input**: winit key/mouse/IME → `core::input` encode → `Pty::write`; selection/paste honor bracketed-paste mode.
- **Control**: the `term-mtp` server runs on its own thread; in-process UI talks to the registries directly,
  external CLI/plugins go over the socket/pipe.
- **Metadata**: cwd (OSC 7), title, agent state, command history → events/registries → panel subscriptions.

## 8. Rendering (term-render)

1. **Glyphs**: `cosmic-text` resolve/fallback, `swash` rasterize → **R8 atlas** (shelf packing, LRU;
   key `(glyph_id, style, px, subpixel)`).
2. **Instancing**: each cell → quads (bg / glyph / underline / strike / cursor / selection); instance
   buffers update by dirty lines only.
3. **Passes**: `bg` (theme color) → `glyph` (atlas sample) → `cursor/decoration`; opacity/background image later.
4. **Present**: prefer `Mailbox` (low latency), fall back to `Fifo` on frame drops; cursor blink via a timer.
5. **Composition with egui**: egui drives the frame; the terminal draws its own rect via an
   `egui-wgpu` **`PaintCallback`** (separate pipeline, shared `wgpu::Device/Queue/Surface`).
6. **Optimization order**: full rebuild → dirty lines → atlas hits → zero per-frame allocation;
   annotate hot spots with `tracy`/`puffin`.

## 9. Terminal model (term-core)

- Grid: `Cell { char + combining + fg/bg/attrs + underline style }`; ring scrollback; reflow.
- Modes: application cursor keys, bracketed paste, mouse reporting, alternate screen, kitty keyboard
  (CSI u), focus reporting.
- Semantics: title, hyperlinks (OSC 8), cwd (OSC 7), progress (OSC 9;4), shell-integration markers
  (OSC 133 A/B/C/D).
- Selection/search: line/block, word boundaries (CJK/grapheme via `unicode-width` + grapheme
  boundaries), search highlight.
- Graphics protocols (kitty graphics / sixel / iTerm2) are R1+ increments, not R0.

## 10. Platform abstraction layer

| Concern | Abstraction | macOS/Linux | Windows |
|---------|-------------|-------------|---------|
| PTY | `trait Pty` | `forkpty` | **ConPTY** |
| Transport | `mtp::transport` | Unix socket | `\\.\pipe\miaotty` |
| Clipboard | `trait Clipboard` | NSPasteboard / X11-Wayland | Win32 clipboard |
| Fonts | `term-render::font` | CoreText / fontconfig | DirectWrite (`font-kit`) |
| IME | `widget::input` | native | **TSF** (highest risk, see §19) |

`#[cfg(...)]` is **only** allowed in the modules above; all other code stays platform-neutral.

## 11. Config / themes

- `term-config`: its own TOML; keys aligned with ghostty/alacritty for import.
- Default look matches Otty (Nord background `#2e3440`, font size 14), overridable.
- Themes: common built-ins + custom palettes; background image/opacity later.

## 12. Control plane (MTP)

- `term-mtp` implements the server; transport is Unix socket (`$TMPDIR/miaotty.sock`) / Windows named pipe.
- Reuse the existing `mtp` messages and `miaotty-cli`; **in-process UI talks to the registries directly**,
  external callers go over the socket/pipe.
- Methods: `core.ping/health`, `agent.state.*`, `history.*`, `pane.list`; events: `agent.state`,
  `history.changed`, `cwd.changed`.
- Transport implementation candidate: `interprocess` (pending license/maintenance review), else a thin wrapper.

## 13. Application layer (miaotty-app)

- Model: `Window → Tab[] → SplitTree<Surface>`; a surface is one terminal instance (core + render view).
- Window: one OS window holds a set of tabs; splits are a tree inside a tab (ours, not OS tabs).
- Panels (egui): **left Tabs sidebar** (title/⌘N/prefix/mark/dividers/context menu), **right Details**
  (Info/Outline/Git/Files), badges, settings, command palette.
- Visual parity with Otty: panels share the terminal background, no divider line, hover highlight.
- OS integration: hook install, URL open, notifications, sleep inhibition (per-platform).

## 14. Extension points (phased; don't front-load)

- **Now**: the `Host` trait + `EventSink` (enough).
- **R4+**: MTP `provider.*` (custom Details components, kind=tui/web).
- **Later (only if demanded)**: renderer/panel plugins, theme packs — only then consider "framework"-izing.

## 15. Errors and security

- Libraries use `thiserror`, the app uses `anyhow`; **no panics on the hot path**; recoverable errors
  become events/logs.
- Security: paste confirmation, OSC injection surface, URL-scheme allowlist, clipboard policy,
  optional secure input.
- Crashes: install a panic hook that logs and best-effort cleans up PTY/child processes.

## 16. Testing / CI / performance gates

- **Conformance**: a subset of `vttest`/`esctest` + golden grid assertions; parser fuzzing (`cargo-fuzz`).
- **Integration**: spawn a real shell, feed byte sequences, assert grid/events.
- **Performance gates (three platforms; Windows separate)**: input latency P95 ≤ 16 ms, first frame ≤ 100 ms,
  no dropped frames on a large `cat`, idle CPU ≈ 0 (`criterion` + a latency harness, results stored to catch regressions).
- **CI**: mac/linux/windows matrix running `check`/`test`/`deny`/`fmt`/`clippy`.

## 17. Packaging / release / versioning

- Engine crates `Apache-2.0`, internal first, published to crates.io once the API is stable.
- App: macOS `.app` + notarize; Linux (AppImage/Flatpak/.deb); Windows MSI (`cargo-dist`/`cargo-wix`) + signing
  (Azure Artifact Signing or self-signed).
- `wgpu` DX12 needs a bundled `dxcompiler.dll` or static `static-dxc`.

## 18. Milestones

R0 minimal loop (pty→vt→grid→render→input, measure latency) → **R0.5 IME** → R1 usable terminal →
R2 three platforms + packaging → R3 `term-mtp` → R4 panels → R5 polish/signing. Each exit must pass its performance gate.

## 19. Risks

| Risk | Mitigation |
|------|------------|
| Self-drawn rendering not reaching Alacritty level | Start with glyphon → damage/vsync/atlas optimization; reference Alacritty/Rio |
| **IME/CJK (riskiest on Windows)** | R0.5 focus; winit `Ime` + `set_ime_cursor_area`; width/grapheme test set |
| ConPTY performance / exit codes | Concurrent read+write + OSC 133;D + separate baseline |
| egui co-frame compatibility/perf | `PaintCallback` approach; fall back to manual viewport if needed |
| Over-abstraction too early | Engine "good enough" first; phase the extension points |
| `alacritty_terminal` API churn | Pin versions; wrap with a `term-core` adapter layer |

## 20. Performance impact analysis (vs Ghostty / Alacritty)

**Conclusion: the choices themselves have no structural performance loss** (the core is Alacritty's,
rendering is GPU). Risks are: ① egui immediate-mode idling every frame if ungated; ② a first self-drawn
renderer is less polished than Ghostty's years-tuned Metal compute; ③ wgpu abstraction + macOS present
modes; ④ Windows ConPTY platform cost (inherent to any approach).

| Dimension | Baseline | Impact | Mitigation |
|-----------|----------|--------|------------|
| Input latency | Alacritty class | No structural loss | Event-driven (no polling) + prefer Mailbox/Immediate; R0 gate |
| Throughput | Alacritty class | **Lock contention** is the main risk | Chunked parsing, short critical sections; snapshot for render if needed |
| Rendering | vs Ghostty Metal | Weaker at first | glyphon start → damage/persistent instance buffers → compute if needed |
| **egui co-frame** | new overhead | Immediate mode may rebuild UI every frame | **Run egui only when chrome is dirty**; terminal redraw driven by damage; no continuous repaint |
| wgpu abstraction | vs direct Metal | Tiny (same Metal backend) | Keep a specialized path possible; benchmark head-to-head |
| present/vsync | macOS mode limits | Forced Fifo increases latency | Measure Mailbox/Immediate; choose dynamically |
| Fonts/first frame | — | Load + shaping slow the first frame | Parallel init + on-disk cache |
| Memory | parity | Atlas/instance buffers | LRU atlas, buffer reuse, dirty-region atlas uploads |
| Idle CPU | parity (≈0) | Blink/animation makes it nonzero | **Blink only when focused**; no timed polling |
| Windows | ConPTY cost | Inherently slower (ConHost + VT re-encode) | Concurrent I/O; a Windows-specific baseline |
| macOS polish | loses subpixel/emoji/blur | **Visual** loss, not performance | Dedicated later |

**Red line**: R0 must pass input latency P95 ≤ 16 ms / no dropped frames on large output / idle CPU ≈ 0
before later milestones. egui idling, present mode, and atlas uploads are the three key things to measure.

## 21. ADRs (`docs/decisions/`)

- 0001 Stack selection (frozen here in §2) — written
- 0002 Concurrency and lock discipline
- 0003 Terminal + egui co-frame rendering
- 0004 Own tab/split model (not OS-native)
- 0005 MTP transport (Unix socket / Windows named pipe)
- 0006 License and dependency policy — written
