//! `miao-term-widget` — a native `winit` + `wgpu` render loop for the engine.
//!
//! Unlike the eframe/egui bootstrap in `miaotty-app`, the terminal grid here is
//! **self-drawn**: `term-widget` owns the window and surface, and the PTY reader
//! thread wakes the loop the moment output arrives (via an `EventLoopProxy`), so
//! echo is drawn on the next frame. Nothing goes through egui's immediate-mode
//! frame flow. See `docs/decisions/0011-native-render-loop.md`.

use std::error::Error;
use std::sync::Arc;

use miao_term_core::aterm::Color as TermColor;
use miao_term_core::{ATerm, Terminal};
use miao_term_render::{MetricsProbe, Quad, QuadRenderer, Span, TermRenderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

/// Nord defaults, matching the terminal core's theme.
const BG: (u8, u8, u8) = (0x2e, 0x34, 0x40);
const FG: (u8, u8, u8) = (0xd8, 0xde, 0xe9);
const PALETTE: [(u8, u8, u8); 16] = [
    (0x3b, 0x42, 0x52),
    (0xbf, 0x61, 0x6a),
    (0xa3, 0xbe, 0x8c),
    (0xeb, 0xcb, 0x8b),
    (0x81, 0xa1, 0xc1),
    (0xb4, 0x8e, 0xad),
    (0x88, 0xc0, 0xd0),
    (0xe5, 0xe9, 0xf0),
    (0x4c, 0x56, 0x6a),
    (0xbf, 0x61, 0x6a),
    (0xa3, 0xbe, 0x8c),
    (0xeb, 0xcb, 0x8b),
    (0x81, 0xa1, 0xc1),
    (0xb4, 0x8e, 0xad),
    (0x8f, 0xbc, 0xbb),
    (0xec, 0xef, 0xf4),
];
const FONT_SIZE: f32 = 13.0;
const LINE_RATIO: f32 = 1.3;

/// Run a native terminal window until it is closed.
pub fn run(title: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let event_loop = EventLoop::<()>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut host = Host {
        title: title.to_string(),
        proxy,
        state: None,
    };
    event_loop.run_app(&mut host)?;
    Ok(())
}

struct Host {
    title: String,
    proxy: EventLoopProxy<()>,
    state: Option<State>,
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    glyphs: TermRenderer,
    quads: QuadRenderer,
    term: Terminal,
    cw: f32,
    ch: f32,
    mods: ModifiersState,
}

impl State {
    fn cell_size() -> (f32, f32) {
        let mut probe = MetricsProbe::new();
        probe.cell(
            FONT_SIZE,
            (FONT_SIZE * LINE_RATIO).round(),
            Some("JetBrains Mono"),
        )
    }

    fn resize(&mut self) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        let cols = (size.width as f32 / self.cw).floor().max(1.0) as u16;
        let rows = (size.height as f32 / self.ch).floor().max(1.0) as u16;
        self.term.resize(rows, cols);
    }

    fn write_input(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.term.write(bytes);
            self.window.request_redraw();
        }
    }

    fn render(&mut self) {
        let _ = self.term.process_pending();

        let (rows, cols) = self.term.screen().size();
        let scale = self.window.scale_factor() as f32;
        let cw = self.cw * scale;
        let ch = self.ch * scale;

        // Backgrounds / selection / cursor as instanced quads (physical px).
        let mut quads: Vec<Quad> = Vec::new();
        let cursor = self.term.screen().cursor_position();
        let show_cursor = !self.term.screen().hide_cursor();
        for row in 0..rows {
            for col in 0..cols {
                let Some(cell) = self.term.screen().cell(row, col) else {
                    continue;
                };
                let bg = map_color(cell.bg, false);
                if bg != BG {
                    quads.push(cell_quad(row, col, cw, ch, bg));
                }
            }
        }
        if show_cursor && cursor.0 < rows && cursor.1 < cols {
            quads.push(cell_quad(cursor.0, cursor.1, cw, ch, FG));
        }

        let rows_data = build_rows(self.term.screen());
        self.quads.prepare(
            &self.device,
            &self.queue,
            (self.config.width, self.config.height),
            &quads,
        );
        self.glyphs.prepare(
            &self.device,
            &self.queue,
            (self.config.width, self.config.height),
            scale,
            FONT_SIZE,
            (FONT_SIZE * LINE_RATIO).round(),
            self.cw,
            0.0,
            0.0,
            FG,
            Some("JetBrains Mono"),
            &rows_data,
        );

        let Ok(frame) = self.surface.get_current_texture() else {
            return;
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terminal"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: BG.0 as f64 / 255.0,
                            g: BG.1 as f64 / 255.0,
                            b: BG.2 as f64 / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            self.quads.render(&mut pass);
            self.glyphs.render(&mut pass);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}

impl ApplicationHandler for Host {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(1100.0, 720.0));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("miaotty-native: create_window failed: {e}");
                event_loop.exit();
                return;
            }
        };

        let proxy = self.proxy.clone();
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = proxy.send_event(());
        });

        let (cw, ch) = State::cell_size();
        let size = window.inner_size();
        let cols = (size.width as f32 / cw).floor().max(1.0) as u16;
        let rows = (size.height as f32 / ch).floor().max(1.0) as u16;
        let term = match Terminal::new(None, cols, rows, 10_000, None, &[], waker) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("miaotty-native: failed to spawn shell: {e}");
                event_loop.exit();
                return;
            }
        };

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let surface = match instance.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("miaotty-native: create_surface failed: {e}");
                event_loop.exit();
                return;
            }
        };
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })) {
                Some(a) => a,
                None => {
                    eprintln!("miaotty-native: no suitable wgpu adapter");
                    event_loop.exit();
                    return;
                }
            };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .expect("device");
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            // Lowest-latency present the display allows (falls back internally).
            present_mode: wgpu::PresentMode::AutoNoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        let glyphs = TermRenderer::new(&device, &queue, format);
        let quads = QuadRenderer::new(&device, format);

        self.state = Some(State {
            window,
            surface,
            device,
            queue,
            config,
            glyphs,
            quads,
            term,
            cw,
            ch,
            mods: ModifiersState::empty(),
        });
        self.state.as_ref().unwrap().window.request_redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) => {
                state.resize();
                state.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => state.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                let app_cursor = state.term.screen().application_cursor();
                let bytes = encode_key(&event, state.mods, app_cursor);
                state.write_input(&bytes);
            }
            WindowEvent::RedrawRequested => state.render(),
            _ => {}
        }
    }
}

/// Map an alacritty color to RGB using the default theme.
fn map_color(c: TermColor, foreground: bool) -> (u8, u8, u8) {
    use miao_term_core::aterm::NamedColor;
    match c {
        TermColor::Spec(rgb) => (rgb.r, rgb.g, rgb.b),
        TermColor::Indexed(i) => indexed(i),
        TermColor::Named(n) => match n {
            NamedColor::Foreground | NamedColor::BrightForeground => FG,
            NamedColor::Background => BG,
            NamedColor::Black => PALETTE[0],
            NamedColor::Red => PALETTE[1],
            NamedColor::Green => PALETTE[2],
            NamedColor::Yellow => PALETTE[3],
            NamedColor::Blue => PALETTE[4],
            NamedColor::Magenta => PALETTE[5],
            NamedColor::Cyan => PALETTE[6],
            NamedColor::White => PALETTE[7],
            NamedColor::BrightBlack => PALETTE[8],
            NamedColor::BrightRed => PALETTE[9],
            NamedColor::BrightGreen => PALETTE[10],
            NamedColor::BrightYellow => PALETTE[11],
            NamedColor::BrightBlue => PALETTE[12],
            NamedColor::BrightMagenta => PALETTE[13],
            NamedColor::BrightCyan => PALETTE[14],
            NamedColor::BrightWhite => PALETTE[15],
            NamedColor::DimForeground => (0x7a, 0x82, 0x8e),
            NamedColor::Cursor => FG,
            _ => {
                if foreground {
                    FG
                } else {
                    BG
                }
            }
        },
    }
}

fn indexed(i: u8) -> (u8, u8, u8) {
    match i {
        0..=15 => PALETTE[i as usize],
        16..=231 => {
            // 6x6x6 color cube.
            let i = i as u16 - 16;
            let r = (i / 36) % 6;
            let g = (i / 6) % 6;
            let b = i % 6;
            let v = |x: u16| (x as u8) * 40 + if x > 0 { 55 } else { 0 };
            (v(r), v(g), v(b))
        }
        232..=255 => {
            let v = (i as u16 - 232) * 10 + 8;
            (v as u8, v as u8, v as u8)
        }
    }
}

fn cell_quad(row: u16, col: u16, cw: f32, ch: f32, color: (u8, u8, u8)) -> Quad {
    Quad::new(
        (col as f32 * cw, row as f32 * ch),
        ((col as f32 + 1.0) * cw, (row as f32 + 1.0) * ch),
        (color.0, color.1, color.2, 255),
    )
}

/// Build per-row, cell-pinned spans (wide chars get their own span).
fn build_rows(screen: &ATerm) -> Vec<Vec<Span>> {
    let (rows, cols) = screen.size();
    let mut out = Vec::with_capacity(rows as usize);
    for row in 0..rows {
        let mut spans: Vec<Span> = Vec::new();
        let mut run_col: Option<u16> = None;
        let mut run_color = (0u8, 0u8, 0u8);
        let mut run_text = String::new();
        let mut col = 0u16;
        while col < cols {
            let Some(cell) = screen.cell(row, col) else {
                col += 1;
                continue;
            };
            let width = unicode_width_of(cell.ch);
            let mut buf = [0u8; 4];
            let text = cell.ch.encode_utf8(&mut buf).to_string();
            let color = if cell.inverse {
                BG
            } else {
                map_color(cell.fg, true)
            };
            if width >= 2 {
                if let Some(rc) = run_col.take() {
                    spans.push(Span::new(rc, std::mem::take(&mut run_text), run_color));
                }
                spans.push(Span::new(col, text, color));
                col += 2;
            } else {
                if let Some(rc) = run_col {
                    if run_color == color {
                        run_text.push_str(&text);
                    } else {
                        spans.push(Span::new(rc, std::mem::take(&mut run_text), run_color));
                        run_col = Some(col);
                        run_color = color;
                        run_text = text;
                    }
                } else {
                    run_col = Some(col);
                    run_color = color;
                    run_text = text;
                }
                col += 1;
            }
        }
        if let Some(rc) = run_col.take() {
            spans.push(Span::new(rc, run_text, run_color));
        }
        out.push(spans);
    }
    out
}

fn unicode_width_of(c: char) -> u16 {
    if (c as u32) >= 0x1100
        && ((c as u32) <= 0x115f
            || (0x2e80..=0xa4cf).contains(&(c as u32))
            || (0xac00..=0xd7a3).contains(&(c as u32))
            || (0xf900..=0xfaff).contains(&(c as u32))
            || (0xfe30..=0xfe4f).contains(&(c as u32))
            || (0xff00..=0xff60).contains(&(c as u32))
            || (0xffe0..=0xffe6).contains(&(c as u32))
            || (0x1f300..=0x1faff).contains(&(c as u32))
            || (0x20000..=0x3fffd).contains(&(c as u32)))
    {
        2
    } else {
        1
    }
}

/// Minimal key encoder (text, control bytes, navigation, editing keys).
fn encode_key(event: &KeyEvent, mods: ModifiersState, app_cursor: bool) -> Vec<u8> {
    if event.state != ElementState::Pressed {
        return Vec::new();
    }
    let ctrl = mods.control_key();
    let alt = mods.alt_key();
    let mut out: Vec<u8> = Vec::new();
    if alt {
        out.push(0x1b);
    }
    if let Key::Character(s) = &event.logical_key {
        if ctrl {
            if let Some(c) = s.chars().next() {
                let c = c.to_ascii_uppercase();
                if ('@'..='_').contains(&c) {
                    out.push(c as u8 - 0x40);
                    return out;
                }
            }
        }
        out.extend_from_slice(s.as_bytes());
        return out;
    }
    if let Some(text) = &event.text {
        out.extend_from_slice(text.as_bytes());
        return out;
    }
    let seq: &[u8] = match &event.logical_key {
        Key::Named(NamedKey::Enter) => b"\r",
        Key::Named(NamedKey::Backspace) => &[0x7f],
        Key::Named(NamedKey::Tab) => b"\t",
        Key::Named(NamedKey::Escape) => &[0x1b],
        Key::Named(NamedKey::ArrowUp) => {
            if app_cursor {
                b"\x1bOA"
            } else {
                b"\x1b[A"
            }
        }
        Key::Named(NamedKey::ArrowDown) => {
            if app_cursor {
                b"\x1bOB"
            } else {
                b"\x1b[B"
            }
        }
        Key::Named(NamedKey::ArrowRight) => {
            if app_cursor {
                b"\x1bOC"
            } else {
                b"\x1b[C"
            }
        }
        Key::Named(NamedKey::ArrowLeft) => {
            if app_cursor {
                b"\x1bOD"
            } else {
                b"\x1b[D"
            }
        }
        Key::Named(NamedKey::Home) => b"\x1b[H",
        Key::Named(NamedKey::End) => b"\x1b[F",
        Key::Named(NamedKey::Delete) => b"\x1b[3~",
        Key::Named(NamedKey::PageUp) => b"\x1b[5~",
        Key::Named(NamedKey::PageDown) => b"\x1b[6~",
        _ => return out,
    };
    out.extend_from_slice(seq);
    out
}
