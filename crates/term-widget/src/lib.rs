//! `miao-term-widget` — a native `winit` + `wgpu` render loop for the engine.
//!
//! The terminal grid is **self-drawn** (no egui immediate-mode frame flow):
//! `term-widget` owns the window and surface, and the PTY reader threads wake
//! the loop (`EventLoopProxy` → `request_redraw`) the moment output arrives, so
//! echo is drawn on the next frame. The surrounding UI (ADR 0030, step 1) is an
//! **egui overlay composed in the same wgpu frame**: the grid is drawn first,
//! then egui's meshes on top.

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use miao_term_core::aterm::Color as TermColor;
use miao_term_core::{ATerm, Terminal};
use miao_term_render::{MetricsProbe, Quad, QuadRenderer, Span, TermRenderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
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
const SEL_BG: (u8, u8, u8) = (0x43, 0x4c, 0x5e);
const FONT_SIZE: f32 = 13.0;
const LINE_RATIO: f32 = 1.3;
/// Height of the egui chrome bar (logical points), reserved above the grid.
const BAR_H: f32 = 30.0;
const BLINK: Duration = Duration::from_millis(530);

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

struct Tab {
    term: Terminal,
    title: String,
}

struct State {
    window: Arc<Window>,
    proxy: EventLoopProxy<()>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    glyphs: TermRenderer,
    quads: QuadRenderer,
    tabs: Vec<Tab>,
    active: usize,
    cw: f32,
    ch: f32,
    font_size: f32,
    mods: ModifiersState,
    scroll: usize,
    selection: Option<((u16, u16), (u16, u16))>,
    dragging: bool,
    last_title: Option<String>,
    focused: bool,
    cursor_on: bool,
    last_blink: Instant,
    // egui overlay (chrome).
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

impl State {
    fn cell_size(font_size: f32) -> (f32, f32) {
        let mut probe = MetricsProbe::new();
        probe.cell(
            font_size,
            (font_size * LINE_RATIO).round(),
            Some("JetBrains Mono"),
        )
    }

    fn grid_size(&self) -> (u16, u16) {
        let size = self.window.inner_size();
        let scale = self.window.scale_factor() as f32;
        let cols = (size.width as f32 / (self.cw * scale)).floor().max(1.0) as u16;
        let rows = ((size.height as f32 - BAR_H * scale) / (self.ch * scale))
            .floor()
            .max(1.0) as u16;
        (rows, cols)
    }

    fn spawn(&self, title: String) -> Option<Tab> {
        let (rows, cols) = self.grid_size();
        let proxy = self.proxy.clone();
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = proxy.send_event(());
        });
        Terminal::new(None, cols, rows, 10_000, None, &[], waker)
            .ok()
            .map(|term| Tab { term, title })
    }

    fn new_tab(&mut self) {
        let n = self.tabs.len() + 1;
        if let Some(tab) = self.spawn(format!("shell {n}")) {
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
            self.selection = None;
            self.scroll = 0;
        }
    }

    fn close_tab(&mut self, i: usize) {
        if self.tabs.len() <= 1 || i >= self.tabs.len() {
            return;
        }
        self.tabs.remove(i);
        self.active = self.active.min(self.tabs.len() - 1);
        self.selection = None;
        self.scroll = 0;
    }

    fn resize(&mut self) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        let (rows, cols) = self.grid_size();
        for tab in &mut self.tabs {
            tab.term.resize(rows, cols);
        }
    }

    fn write_input(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.term.write(bytes);
            }
            self.scroll = 0;
            self.window.request_redraw();
        }
    }

    fn cell_at(&self, pos: (f64, f64), rows: u16, cols: u16) -> (u16, u16) {
        let scale = self.window.scale_factor() as f32;
        let cw = self.cw * scale;
        let ch = self.ch * scale;
        let col = (pos.0 as f32 / cw).floor().clamp(0.0, cols as f32 - 1.0) as u16;
        let row = ((pos.1 as f32 - BAR_H * scale) / ch)
            .floor()
            .clamp(0.0, rows as f32 - 1.0) as u16;
        (row, col)
    }

    fn copy_selection(&self, ctx: &egui::Context) {
        if let (Some((a, b)), Some(tab)) = (self.selection, self.tabs.get(self.active)) {
            let (r1, c1, r2, c2) = ordered(a, b);
            let text = tab.term.screen().contents_between(r1, c1, r2, c2);
            if !text.is_empty() {
                ctx.copy_text(text);
            }
        }
    }

    /// The egui chrome overlay: a tab bar above the grid.
    fn chrome(&mut self, ctx: &egui::Context) {
        let mut switch: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut new_tab = false;
        let mut font_delta = 0.0f32;
        let active = self.active;
        let titles: Vec<String> = self
            .tabs
            .iter()
            .map(|t| {
                t.term
                    .title()
                    .map(str::to_string)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| t.title.clone())
            })
            .collect();
        let cwd = self
            .tabs
            .get(self.active)
            .and_then(|t| t.term.cwd().map(str::to_string))
            .unwrap_or_default();
        let fg = egui::Color32::from_rgb(FG.0, FG.1, FG.2);
        let bg = egui::Color32::from_rgb(0x24, 0x29, 0x33);
        egui::TopBottomPanel::top("bar")
            .exact_height(BAR_H)
            .frame(
                egui::Frame::default()
                    .fill(bg)
                    .inner_margin(egui::Margin::symmetric(6.0, 3.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.visuals_mut().override_text_color = Some(fg);
                    for (i, title) in titles.iter().enumerate() {
                        if ui.selectable_label(i == active, title).clicked() {
                            switch = Some(i);
                        }
                        if self.tabs.len() > 1 && ui.small_button("\u{00d7}").clicked() {
                            close = Some(i);
                        }
                    }
                    if ui.button("+").clicked() {
                        new_tab = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("A+").clicked() {
                            font_delta = 1.0;
                        }
                        if ui.button("A-").clicked() {
                            font_delta = -1.0;
                        }
                        if !cwd.is_empty() && ui.button("Reveal").clicked() {
                            reveal(&cwd);
                        }
                    });
                });
            });
        if let Some(i) = switch {
            self.active = i;
            self.selection = None;
            self.scroll = 0;
        }
        if new_tab {
            self.new_tab();
        }
        if let Some(i) = close {
            self.close_tab(i);
        }
        if font_delta != 0.0 {
            self.font_size = (self.font_size + font_delta).clamp(6.0, 40.0);
            let (cw, ch) = State::cell_size(self.font_size);
            self.cw = cw;
            self.ch = ch;
            self.resize();
        }
    }

    fn render(&mut self) {
        // Drain every tab; new output on the active tab jumps to the bottom.
        let mut active_changed = false;
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if tab.term.process_pending() && i == self.active {
                active_changed = true;
            }
        }
        if active_changed {
            self.scroll = 0;
        }

        let bracketed = {
            let Some(tab) = self.tabs.get_mut(self.active) else {
                return;
            };
            tab.term.screen_mut().set_scrollback(self.scroll);

            // Sync the window title from OSC 0/2.
            let title = tab.term.title().map(str::to_string);
            if title != self.last_title {
                if let Some(t) = &title {
                    self.window.set_title(t);
                }
                self.last_title = title;
            }

            let (rows, cols) = tab.term.screen().size();
            let scale = self.window.scale_factor() as f32;
            let cw = self.cw * scale;
            let ch = self.ch * scale;
            let top = BAR_H * scale;

            let mut quads: Vec<Quad> = Vec::new();
            for row in 0..rows {
                for col in 0..cols {
                    let Some(cell) = tab.term.screen().cell(row, col) else {
                        continue;
                    };
                    let bg = map_color(cell.bg, false);
                    if bg != BG {
                        quads.push(cell_quad(row, col, cw, ch, top, bg));
                    }
                }
            }
            if let Some((a, b)) = self.selection {
                let (r1, c1, r2, c2) = ordered(a, b);
                let start = r1 as usize * cols as usize + c1 as usize;
                let end = r2 as usize * cols as usize + c2 as usize;
                for i in start..=end {
                    let r = (i / cols as usize) as u16;
                    let c = (i % cols as usize) as u16;
                    quads.push(cell_quad(r, c, cw, ch, top, SEL_BG));
                }
            }
            let cursor = tab.term.screen().cursor_position();
            let show_cursor = self.scroll == 0
                && self.cursor_on
                && !tab.term.screen().hide_cursor()
                && cursor.0 < rows
                && cursor.1 < cols;
            if show_cursor {
                quads.push(cell_quad(cursor.0, cursor.1, cw, ch, top, FG));
            }

            let rows_data = build_rows(tab.term.screen());
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
                self.font_size,
                (self.font_size * LINE_RATIO).round(),
                self.cw,
                0.0,
                BAR_H,
                FG,
                Some("JetBrains Mono"),
                &rows_data,
            );
            tab.term.screen().bracketed_paste()
        };

        // egui overlay: run the chrome, then tessellate and upload.
        let raw = self.egui_state.take_egui_input(&self.window);
        let events = raw.events.clone();
        let egui_ctx = self.egui_ctx.clone();
        let output = egui_ctx.run(raw, |ctx| self.chrome(ctx));
        self.egui_state
            .handle_platform_output(&self.window, output.platform_output);
        let ppp = self.egui_ctx.pixels_per_point();
        let paint_jobs = self.egui_ctx.tessellate(output.shapes, ppp);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: ppp,
        };
        for (id, delta) in &output.textures_delta.set {
            self.egui_renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &paint_jobs,
            &screen,
        );

        let Ok(frame) = self.surface.get_current_texture() else {
            return;
        };
        let view = frame.texture.create_view(&Default::default());
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
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
                })
                .forget_lifetime();
            self.quads.render(&mut pass);
            self.glyphs.render(&mut pass);
            self.egui_renderer.render(&mut pass, &paint_jobs, &screen);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        for id in &output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }

        // Copy / paste delivered by egui-winit.
        for ev in &events {
            match ev {
                egui::Event::Copy => self.copy_selection(&self.egui_ctx),
                egui::Event::Paste(text) => {
                    let body = text.replace('\n', "\r");
                    let bytes = if bracketed {
                        format!("\x1b[200~{body}\x1b[201~")
                    } else {
                        body
                    };
                    if let Some(tab) = self.tabs.get_mut(self.active) {
                        tab.term.write(bytes.as_bytes());
                    }
                    self.window.request_redraw();
                }
                _ => {}
            }
        }
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
        window.set_ime_allowed(true);

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
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoNoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        let glyphs = TermRenderer::new(&device, &queue, format);
        let quads = QuadRenderer::new(&device, format);

        let egui_ctx = egui::Context::default();
        install_egui_fonts(&egui_ctx);
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, format, None, 1, false);

        let (cw, ch) = State::cell_size(FONT_SIZE);
        let mut state = State {
            window,
            proxy: self.proxy.clone(),
            surface,
            device,
            queue,
            config,
            glyphs,
            quads,
            tabs: Vec::new(),
            active: 0,
            cw,
            ch,
            font_size: FONT_SIZE,
            mods: ModifiersState::empty(),
            scroll: 0,
            selection: None,
            dragging: false,
            last_title: None,
            focused: false,
            cursor_on: true,
            last_blink: Instant::now(),
            egui_ctx,
            egui_state,
            egui_renderer,
        };
        if let Some(tab) = state.spawn("shell 1".to_string()) {
            state.tabs.push(tab);
        }
        state.window.request_redraw();
        self.state = Some(state);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Cursor blink while focused (idle otherwise → ~0% CPU).
        if let Some(state) = &mut self.state {
            if state.focused {
                if state.last_blink.elapsed() >= BLINK {
                    state.cursor_on = !state.cursor_on;
                    state.last_blink = Instant::now();
                    state.window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + BLINK));
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        // Let egui see input events (overlay + clipboard). Never forward
        // `RedrawRequested`: egui answers it with "repaint again" → 100% CPU.
        if !matches!(event, WindowEvent::RedrawRequested) {
            let resp = state.egui_state.on_window_event(&state.window, &event);
            if resp.repaint {
                state.window.request_redraw();
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(f) => state.focused = f,
            WindowEvent::Resized(_) => {
                state.resize();
                state.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => state.mods = m.state(),
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                state.write_input(text.as_bytes());
            }
            WindowEvent::MouseInput {
                state: es, button, ..
            } => {
                if button == MouseButton::Left {
                    state.dragging = es == ElementState::Pressed;
                    if es == ElementState::Pressed {
                        state.selection = None;
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if state.dragging {
                    let (rows, cols) = state
                        .tabs
                        .get(state.active)
                        .map(|t| t.term.screen().size())
                        .unwrap_or((0, 0));
                    let cell = state.cell_at((position.x, position.y), rows, cols);
                    match &mut state.selection {
                        Some((_, end)) => *end = cell,
                        None => state.selection = Some((cell, cell)),
                    }
                    state.window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                use winit::event::MouseScrollDelta;
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as i32,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 12.0) as i32,
                };
                let max = state
                    .tabs
                    .get(state.active)
                    .map(|t| t.term.screen().scrollback_len())
                    .unwrap_or(0);
                if lines > 0 {
                    state.scroll = (state.scroll + lines as usize).min(max);
                } else {
                    state.scroll = state.scroll.saturating_sub((-lines) as usize);
                }
                state.window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(bytes) = shortcut(&event, state.mods) {
                    match bytes {
                        Shortcut::NewTab => state.new_tab(),
                        Shortcut::CloseTab => state.close_tab(state.active),
                        Shortcut::SelectTab(i) => {
                            if i < state.tabs.len() {
                                state.active = i;
                                state.selection = None;
                                state.scroll = 0;
                            }
                        }
                        Shortcut::Font(delta) => {
                            state.font_size = (state.font_size + delta).clamp(6.0, 40.0);
                            let (cw, ch) = State::cell_size(state.font_size);
                            state.cw = cw;
                            state.ch = ch;
                            state.resize();
                        }
                    }
                    state.window.request_redraw();
                } else {
                    let app_cursor = state
                        .tabs
                        .get(state.active)
                        .map(|t| t.term.screen().application_cursor())
                        .unwrap_or(false);
                    let bytes = encode_key(&event, state.mods, app_cursor);
                    state.write_input(&bytes);
                }
            }
            WindowEvent::RedrawRequested => state.render(),
            _ => {}
        }
    }
}

enum Shortcut {
    NewTab,
    CloseTab,
    SelectTab(usize),
    Font(f32),
}

fn shortcut(event: &KeyEvent, mods: ModifiersState) -> Option<Shortcut> {
    if event.state != ElementState::Pressed || !mods.super_key() {
        return None;
    }
    match &event.logical_key {
        Key::Character(s) => match s.as_str() {
            "t" => Some(Shortcut::NewTab),
            "w" => Some(Shortcut::CloseTab),
            "+" | "=" => Some(Shortcut::Font(1.0)),
            "-" => Some(Shortcut::Font(-1.0)),
            "1" => Some(Shortcut::SelectTab(0)),
            "2" => Some(Shortcut::SelectTab(1)),
            "3" => Some(Shortcut::SelectTab(2)),
            "4" => Some(Shortcut::SelectTab(3)),
            "5" => Some(Shortcut::SelectTab(4)),
            "6" => Some(Shortcut::SelectTab(5)),
            "7" => Some(Shortcut::SelectTab(6)),
            "8" => Some(Shortcut::SelectTab(7)),
            "9" => Some(Shortcut::SelectTab(8)),
            _ => None,
        },
        _ => None,
    }
}

fn ordered(a: (u16, u16), b: (u16, u16)) -> (u16, u16, u16, u16) {
    if b < a {
        (b.0, b.1, a.0, a.1)
    } else {
        (a.0, a.1, b.0, b.1)
    }
}

fn reveal(path: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

fn install_egui_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "nerd".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../assets/fonts/SymbolsNerdFontMono-Regular.ttf"
        ))),
    );
    for path in [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "cjk".to_owned(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            break;
        }
    }
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        list.push("cjk".to_owned());
        list.push("nerd".to_owned());
    }
    ctx.set_fonts(fonts);
}

fn map_color(c: TermColor, foreground: bool) -> (u8, u8, u8) {
    use miao_term_core::aterm::NamedColor;
    match c {
        TermColor::Spec(rgb) => (rgb.r, rgb.g, rgb.b),
        TermColor::Indexed(i) => indexed(i),
        TermColor::Named(n) => match n {
            NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => FG,
            NamedColor::Background => BG,
            NamedColor::DimForeground => (0x7a, 0x82, 0x8e),
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
            let i = i as u16 - 16;
            let f = |x: u16| (x as u8) * 40 + if x > 0 { 55 } else { 0 };
            (f(i / 36), f((i / 6) % 6), f(i % 6))
        }
        _ => {
            let v = ((i as u16 - 232) * 10 + 8) as u8;
            (v, v, v)
        }
    }
}

fn cell_quad(row: u16, col: u16, cw: f32, ch: f32, top: f32, color: (u8, u8, u8)) -> Quad {
    Quad::new(
        (col as f32 * cw, top + row as f32 * ch),
        ((col as f32 + 1.0) * cw, top + (row as f32 + 1.0) * ch),
        (color.0, color.1, color.2, 255),
    )
}

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
    let u = c as u32;
    if u >= 0x1100
        && (u <= 0x115f
            || (0x2e80..=0xa4cf).contains(&u)
            || (0xac00..=0xd7a3).contains(&u)
            || (0xf900..=0xfaff).contains(&u)
            || (0xfe30..=0xfe4f).contains(&u)
            || (0xff00..=0xff60).contains(&u)
            || (0xffe0..=0xffe6).contains(&u)
            || (0x1f300..=0x1faff).contains(&u)
            || (0x20000..=0x3fffd).contains(&u))
    {
        2
    } else {
        1
    }
}

fn encode_key(event: &KeyEvent, mods: ModifiersState, app_cursor: bool) -> Vec<u8> {
    if event.state != ElementState::Pressed {
        return Vec::new();
    }
    // Cmd (super) is reserved for app shortcuts (handled by `shortcut`).
    if mods.super_key() {
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
