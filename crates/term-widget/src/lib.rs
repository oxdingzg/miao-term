//! `miao-term-widget` — a native `winit` + `wgpu` host for the engine.
//!
//! The terminal grid is **self-drawn** (no egui immediate-mode frame flow): the
//! host owns the window and surface, and the PTY reader threads wake the loop so
//! echo is drawn on the next frame. The surrounding UI (tabs, sidebar, details,
//! status) is an **egui overlay composed in the same wgpu frame**. Shared,
//! host-agnostic pieces (theme, input encoding, selection, split layout, row
//! building, chrome widgets) live in `miao-term-ui`.

use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use miao_term_core::Terminal;
use miao_term_render::{Quad, QuadRenderer, Span, TermRenderer};
use miao_term_ui::layout::{Layout, Rect, SplitDir};
use miao_term_ui::{build_rows, chrome, input, theme::Theme, Selection};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const FONT_SIZE: f32 = 13.0;
const LINE_RATIO: f32 = 1.3;
const TAB_H: f32 = 30.0;
const STATUS_H: f32 = 22.0;
const SIDEBAR_W: f32 = 200.0;
const DETAILS_W: f32 = 300.0;
const GUTTER: f32 = 4.0;
const BLINK: Duration = Duration::from_millis(530);

/// Run a native terminal window until it is closed.
pub fn run(title: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    // MTP control plane (ADR 0005): the shell inherits `MIAOTTY_SOCKET`, so
    // `miaotty-cli`, plugins and agent hooks work exactly as with the eframe app.
    let socket = miao_term_mtp::default_socket();
    std::env::set_var("MIAOTTY_SOCKET", &socket);
    let mtp = miao_term_mtp::ServerState::new();
    match miao_term_mtp::serve(&socket, mtp.clone()) {
        Ok(()) => eprintln!("miaotty-native: MTP host on {}", socket.display()),
        Err(e) => eprintln!("miaotty-native: MTP host failed: {e}"),
    }

    let event_loop = EventLoop::<()>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut host = Host {
        title: title.to_string(),
        proxy,
        mtp,
        state: None,
    };
    event_loop.run_app(&mut host)?;
    Ok(())
}

struct Host {
    title: String,
    proxy: EventLoopProxy<()>,
    mtp: Arc<miao_term_mtp::ServerState>,
    state: Option<State>,
}

/// A time-stamped, path-keyed details cache (git status / directory listing).
type Cache = Option<(Instant, std::path::PathBuf, Vec<(String, String)>)>;

struct Pane {
    id: String,
    term: Terminal,
    scroll: usize,
}

struct Tab {
    layout: Layout,
    panes: Vec<Pane>,
    active: String,
    title: String,
}

struct PaneDraw {
    id: String,
    rect: Rect,
    quads: Vec<Quad>,
    rows: Vec<Vec<Span>>,
}

struct State {
    window: Arc<Window>,
    proxy: EventLoopProxy<()>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    quads: QuadRenderer,
    renderers: HashMap<String, TermRenderer>,
    mtp: Arc<miao_term_mtp::ServerState>,
    tabs: Vec<Tab>,
    active_tab: usize,
    theme: Theme,
    cw: f32,
    ch: f32,
    font_size: f32,
    mods: ModifiersState,
    selection: Option<(String, Selection)>,
    dragging: bool,
    divider_drag: Option<(Vec<bool>, SplitDir, Rect)>,
    cursor: (f64, f64),
    show_sidebar: bool,
    show_details: bool,
    renaming: Option<usize>,
    rename_buf: String,
    theme_name: String,
    show_palette: bool,
    palette_query: String,
    show_settings: bool,
    details_tab: usize,
    git_cache: Cache,
    files_cache: Cache,
    last_title: Option<String>,
    focused: bool,
    cursor_on: bool,
    last_blink: Instant,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

struct Shortcut {
    new_tab: bool,
    close: bool,
    select: Option<usize>,
    font: f32,
    split_right: bool,
    split_down: bool,
    cycle: i32,
    toggle_sidebar: bool,
    toggle_details: bool,
    palette: bool,
    settings: bool,
}

fn shortcut(event: &KeyEvent, mods: ModifiersState) -> Option<Shortcut> {
    if event.state != ElementState::Pressed || !mods.super_key() {
        return None;
    }
    let mut s = Shortcut {
        new_tab: false,
        close: false,
        select: None,
        font: 0.0,
        split_right: false,
        split_down: false,
        cycle: 0,
        toggle_sidebar: false,
        toggle_details: false,
        palette: false,
        settings: false,
    };
    let mut matched = true;
    match &event.logical_key {
        Key::Character(c) => match c.as_str() {
            "t" => s.new_tab = true,
            "k" => s.palette = true,
            "p" if mods.shift_key() => s.palette = true,
            "," => s.settings = true,
            "w" => s.close = true,
            "d" => {
                if mods.shift_key() {
                    s.split_down = true;
                } else {
                    s.split_right = true;
                }
            }
            "l" if mods.shift_key() => s.toggle_sidebar = true,
            "r" if mods.shift_key() => s.toggle_details = true,
            "]" => s.cycle = 1,
            "[" => s.cycle = -1,
            "+" | "=" => s.font = 1.0,
            "-" => s.font = -1.0,
            "1" => s.select = Some(0),
            "2" => s.select = Some(1),
            "3" => s.select = Some(2),
            "4" => s.select = Some(3),
            "5" => s.select = Some(4),
            "6" => s.select = Some(5),
            "7" => s.select = Some(6),
            "8" => s.select = Some(7),
            "9" => s.select = Some(8),
            _ => matched = false,
        },
        _ => matched = false,
    }
    if matched {
        Some(s)
    } else {
        None
    }
}

impl State {
    fn cell_size(font_size: f32) -> (f32, f32) {
        let mut probe = miao_term_render::MetricsProbe::new();
        probe.cell(
            font_size,
            (font_size * LINE_RATIO).round(),
            Some("JetBrains Mono"),
        )
    }

    fn window_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width.max(1), s.height.max(1))
    }

    /// The central grid area in logical points (window minus chrome).
    fn grid_area(&self) -> Rect {
        let size = self.window.inner_size();
        let scale = self.window.scale_factor() as f32;
        let w = size.width as f32 / scale;
        let h = size.height as f32 / scale;
        let x = if self.show_sidebar { SIDEBAR_W } else { 0.0 };
        let right = if self.show_details { DETAILS_W } else { 0.0 };
        Rect {
            x,
            y: TAB_H,
            w: (w - x - right).max(1.0),
            h: (h - TAB_H - STATUS_H).max(1.0),
        }
    }

    fn pane_rects(&self) -> Vec<(String, Rect)> {
        match self.tabs.get(self.active_tab) {
            Some(tab) => tab.layout.rects(self.grid_area()),
            None => Vec::new(),
        }
    }

    fn active_pane_id(&self) -> Option<String> {
        self.tabs.get(self.active_tab).map(|t| t.active.clone())
    }

    fn spawn_pane(&self) -> Option<Pane> {
        let (cw, ch) = (
            self.cw * self.window.scale_factor() as f32,
            self.ch * self.window.scale_factor() as f32,
        );
        let area = self.grid_area();
        let cols = ((area.w * self.window.scale_factor() as f32) / cw)
            .floor()
            .max(1.0) as u16;
        let rows = ((area.h * self.window.scale_factor() as f32) / ch)
            .floor()
            .max(1.0) as u16;
        let id = gen_id();
        let proxy = self.proxy.clone();
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = proxy.send_event(());
        });
        let env = vec![("MIAOTTY_PANE_ID".to_string(), id.clone())];
        Terminal::new(None, cols, rows, 10_000, None, &env, waker)
            .ok()
            .map(|term| Pane {
                id,
                term,
                scroll: 0,
            })
    }

    /// Advertise the panes to the MTP control plane.
    fn publish_panes(&self) {
        let mut panes = Vec::new();
        for tab in &self.tabs {
            for p in &tab.panes {
                let title = p
                    .term
                    .title()
                    .map(str::to_string)
                    .unwrap_or_else(|| tab.title.clone());
                panes.push(serde_json::json!({ "id": p.id, "title": title }));
            }
        }
        self.mtp.set_panes(panes);
    }

    fn new_tab(&mut self) {
        let Some(pane) = self.spawn_pane() else {
            return;
        };
        let id = pane.id.clone();
        let n = self.tabs.len() + 1;
        self.tabs.push(Tab {
            layout: Layout::leaf(id.clone()),
            panes: vec![pane],
            active: id,
            title: format!("shell {n}"),
        });
        self.active_tab = self.tabs.len() - 1;
        self.selection = None;
        self.publish_panes();
    }

    fn close_pane(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        let target = tab.active.clone();
        if tab.panes.len() <= 1 {
            // Close the tab.
            if self.tabs.len() > 1 {
                self.tabs.remove(self.active_tab);
                self.active_tab = self.active_tab.min(self.tabs.len() - 1);
                self.selection = None;
                self.publish_panes();
            }
            return;
        }
        tab.panes.retain(|p| p.id != target);
        let _ = tab.layout.remove(&target);
        tab.active = tab.layout.ids().first().cloned().unwrap_or_default();
        self.selection = None;
        self.publish_panes();
    }

    fn split(&mut self, dir: SplitDir) {
        let Some(pane) = self.spawn_pane() else {
            return;
        };
        let new_id = pane.id.clone();
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            let target = tab.active.clone();
            if tab.layout.split(&target, &new_id, dir) {
                tab.panes.push(pane);
                tab.active = new_id;
                self.selection = None;
            }
        }
        self.resize();
        self.publish_panes();
    }

    fn cycle_pane(&mut self, forward: bool) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            let ids = tab.layout.ids();
            if ids.len() > 1 {
                let cur = ids.iter().position(|x| x == &tab.active).unwrap_or(0);
                let next = if forward {
                    (cur + 1) % ids.len()
                } else {
                    (cur + ids.len() - 1) % ids.len()
                };
                tab.active = ids[next].clone();
                self.selection = None;
            }
        }
    }

    fn resize(&mut self) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        let scale = self.window.scale_factor() as f32;
        let cw = self.cw * scale;
        let ch = self.ch * scale;
        let rects = self.pane_rects();
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            for (id, r) in &rects {
                if let Some(pane) = tab.panes.iter_mut().find(|p| &p.id == id) {
                    let cols = ((r.w * scale - GUTTER * 2.0) / cw).floor().max(1.0) as u16;
                    let rows = ((r.h * scale - GUTTER * 2.0) / ch).floor().max(1.0) as u16;
                    pane.term.resize(rows, cols);
                }
            }
        }
    }

    fn write_input(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            if let Some(pane) = tab.panes.iter_mut().find(|p| p.id == tab.active) {
                pane.term.write(bytes);
                pane.scroll = 0;
            }
        }
        self.window.request_redraw();
    }

    fn copy_selection(&self, ctx: &egui::Context) {
        let Some((pane_id, sel)) = &self.selection else {
            return;
        };
        if let Some(tab) = self.tabs.get(self.active_tab) {
            if let Some(pane) = tab.panes.iter().find(|p| &p.id == pane_id) {
                let (r1, c1, r2, c2) = sel.ordered();
                let text = pane.term.screen().contents_between(r1, c1, r2, c2);
                if !text.is_empty() {
                    ctx.copy_text(text);
                }
            }
        }
    }

    fn paste(&mut self, text: &str) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            if let Some(pane) = tab.panes.iter_mut().find(|p| p.id == tab.active) {
                let body = text.replace('\n', "\r");
                let bytes = if pane.term.screen().bracketed_paste() {
                    format!("\x1b[200~{body}\x1b[201~")
                } else {
                    body
                };
                pane.term.write(bytes.as_bytes());
                pane.scroll = 0;
            }
        }
        self.window.request_redraw();
    }

    fn title_of(&self, tab: &Tab) -> String {
        tab.panes
            .iter()
            .find(|p| p.id == tab.active)
            .and_then(|p| p.term.title().map(str::to_string))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| tab.title.clone())
    }

    fn render(&mut self) {
        // MTP `pane.send` / `pane.run` — inject bytes into the target pane.
        for (pane_id, data) in self.mtp.take_writes() {
            for tab in &mut self.tabs {
                if let Some(p) = tab.panes.iter_mut().find(|p| p.id == pane_id) {
                    p.term.write(&data);
                    p.scroll = 0;
                }
            }
        }
        // Drain every pane.
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                if pane.term.process_pending() {
                    pane.scroll = 0;
                }
            }
        }

        // Sync the window title from the active pane's OSC 0/2.
        let title = self
            .tabs
            .get(self.active_tab)
            .map(|t| self.title_of(t))
            .filter(|s| !s.is_empty());
        if title != self.last_title {
            if let Some(t) = &title {
                self.window.set_title(t);
            }
            self.last_title = title;
        }

        // Build per-pane draw data (owning; releases the self.tabs borrow).
        let scale = self.window.scale_factor() as f32;
        let cw = self.cw * scale;
        let ch = self.ch * scale;
        let theme = self.theme.clone();
        let selection = self.selection.clone();
        let rects = self.pane_rects();
        let active_id = self.active_pane_id().unwrap_or_default();
        let mut draws: Vec<PaneDraw> = Vec::new();
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            for (id, r) in &rects {
                let Some(pane) = tab.panes.iter_mut().find(|p| &p.id == id) else {
                    continue;
                };
                let cols = ((r.w * scale - GUTTER * 2.0) / cw).floor().max(1.0) as u16;
                let rows = ((r.h * scale - GUTTER * 2.0) / ch).floor().max(1.0) as u16;
                pane.term.resize(rows, cols);
                pane.term.screen_mut().set_scrollback(pane.scroll);
                let (sr, sc) = pane.term.screen().size();
                let ox = (r.x * scale) + GUTTER;
                let oy = (r.y * scale) + GUTTER;

                let mut quads: Vec<Quad> = Vec::new();
                for row in 0..sr {
                    for col in 0..sc {
                        let Some(cell) = pane.term.screen().cell(row, col) else {
                            continue;
                        };
                        let bg = theme.color(cell.bg, false);
                        if bg != theme.bg {
                            quads.push(quad(ox, oy, row, col, cw, ch, (bg.0, bg.1, bg.2)));
                        }
                    }
                }
                if let Some((pid, sel)) = &selection {
                    if pid == id {
                        for (row, col) in sel.cells(sc) {
                            let s = theme.selection;
                            quads.push(quad(ox, oy, row, col, cw, ch, (s.0, s.1, s.2)));
                        }
                    }
                }
                // Scrollbar indicator.
                let sb = pane.term.screen().scrollback_len();
                if sb > 0 {
                    let total = (sb + sr as usize) as f32;
                    let track = r.h * scale;
                    let thumb = (track * sr as f32 / total).max(12.0);
                    let pos = pane.term.screen().scroll_offset() as f32 / sb as f32;
                    let y = oy + (track - thumb) * (1.0 - pos);
                    quads.push(Quad::new(
                        (ox + (r.w * scale) - 8.0, y),
                        (ox + (r.w * scale) - 4.0, y + thumb),
                        (0x4c, 0x56, 0x6a, 200),
                    ));
                }
                // Cursor (only on the focused pane, at the bottom).
                let cur = pane.term.screen().cursor_position();
                let show = id == &active_id
                    && self.cursor_on
                    && pane.scroll == 0
                    && !pane.term.screen().hide_cursor()
                    && cur.0 < sr
                    && cur.1 < sc;
                if show {
                    match theme.cursor {
                        miao_term_ui::CursorStyle::Block => {
                            let f = theme.fg;
                            quads.push(quad(ox, oy, cur.0, cur.1, cw, ch, (f.0, f.1, f.2)));
                        }
                        miao_term_ui::CursorStyle::Bar => {
                            let f = theme.fg;
                            quads.push(Quad::new(
                                (ox + cur.1 as f32 * cw, oy + cur.0 as f32 * ch),
                                (ox + cur.1 as f32 * cw + 2.0, oy + (cur.0 as f32 + 1.0) * ch),
                                (f.0, f.1, f.2, 255),
                            ));
                        }
                        miao_term_ui::CursorStyle::Underline => {
                            let f = theme.fg;
                            quads.push(Quad::new(
                                (ox + cur.1 as f32 * cw, oy + (cur.0 as f32 + 1.0) * ch - 2.0),
                                (
                                    ox + (cur.1 as f32 + 1.0) * cw,
                                    oy + (cur.0 as f32 + 1.0) * ch,
                                ),
                                (f.0, f.1, f.2, 255),
                            ));
                        }
                    }
                }

                let cursor_cell = if show { Some(cur) } else { None };
                let rows_data = build_rows(pane.term.screen(), &theme, cursor_cell);
                // Glyph origin is relative to the pane's viewport (the egui-wgpu
                // callback sets the viewport), so we render per-pane with the
                // viewer origin at 0 for the glyph renderer.
                draws.push(PaneDraw {
                    id: id.clone(),
                    rect: *r,
                    quads,
                    rows: rows_data,
                });
            }
        }

        // GPU: quads (all panes) then per-pane glyphs.
        let all_quads: Vec<Quad> = draws.iter().flat_map(|d| d.quads.iter().copied()).collect();
        self.quads
            .prepare(&self.device, &self.queue, self.window_size(), &all_quads);
        let win_size = self.window_size();
        let format = self.config.format;
        for d in &draws {
            let device = &self.device;
            let queue = &self.queue;
            let renderer = self
                .renderers
                .entry(d.id.clone())
                .or_insert_with(|| TermRenderer::new(device, queue, format));
            // The pass viewport is the whole surface, so glyphon needs the full
            // resolution and the pane's origin as a logical offset.
            renderer.prepare(
                device,
                queue,
                win_size,
                scale,
                self.font_size,
                (self.font_size * LINE_RATIO).round(),
                self.cw,
                d.rect.x + GUTTER / scale,
                d.rect.y + GUTTER / scale,
                (theme.fg.0, theme.fg.1, theme.fg.2),
                Some("JetBrains Mono"),
                &d.rows,
            );
        }

        // egui chrome.
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
                                r: theme.bg.0 as f64 / 255.0,
                                g: theme.bg.1 as f64 / 255.0,
                                b: theme.bg.2 as f64 / 255.0,
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
            for d in &draws {
                if let Some(renderer) = self.renderers.get(&d.id) {
                    renderer.render(&mut pass);
                }
            }
            self.egui_renderer.render(&mut pass, &paint_jobs, &screen);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        for id in &output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }

        for ev in &events {
            match ev {
                egui::Event::Copy => self.copy_selection(&self.egui_ctx),
                egui::Event::Paste(text) => self.paste(text),
                _ => {}
            }
        }
    }

    fn chrome(&mut self, ctx: &egui::Context) {
        let theme = self.theme.clone();
        let titles: Vec<String> = self.tabs.iter().map(|t| self.title_of(t)).collect();
        let active = self.active_tab;
        let mut switch = None;
        let mut close = None;
        let mut rename: Option<usize> = None;
        let mut new_tab = false;
        let mut font_delta = 0.0f32;

        egui::TopBottomPanel::top("tabs")
            .exact_height(TAB_H)
            .frame(
                egui::Frame::default()
                    .fill(chrome::bg_color(miao_term_ui::theme::Rgb(0x24, 0x29, 0x33)))
                    .inner_margin(egui::Margin::symmetric(6.0, 3.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let ev = chrome::tab_bar(ui, &theme, &titles, active);
                    switch = ev.switch;
                    close = ev.close;
                    rename = ev.rename;
                    new_tab = ev.new_tab;
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("A+").clicked() {
                            font_delta = 1.0;
                        }
                        if ui.button("A-").clicked() {
                            font_delta = -1.0;
                        }
                        if ui
                            .button("\u{2630}")
                            .on_hover_text("Toggle sidebar")
                            .clicked()
                        {
                            self.show_sidebar = !self.show_sidebar;
                        }
                        if ui
                            .button("\u{25a4}")
                            .on_hover_text("Toggle details")
                            .clicked()
                        {
                            self.show_details = !self.show_details;
                        }
                    });
                });
            });

        if self.show_sidebar {
            let badges: Vec<Option<miao_term_ui::theme::Rgb>> = self
                .tabs
                .iter()
                .map(|t| self.agent_badge(&t.active))
                .collect();
            let mut s = None;
            egui::SidePanel::left("sessions")
                .exact_width(SIDEBAR_W)
                .frame(
                    egui::Frame::default()
                        .fill(chrome::bg_color(miao_term_ui::theme::Rgb(0x24, 0x29, 0x33)))
                        .inner_margin(egui::Margin::same(6.0)),
                )
                .show(ctx, |ui| {
                    s = chrome::sidebar(ui, &theme, &titles, &badges, active);
                });
            if let Some(i) = s {
                switch = Some(i);
            }
        }
        if self.show_details {
            const TABS: [&str; 5] = ["Info", "Agent", "Outline", "Git", "Files"];
            let active = self.details_tab.min(TABS.len() - 1);
            let (title, rows) = self.details_content(active);
            let mut sel = None;
            egui::SidePanel::right("details")
                .exact_width(DETAILS_W)
                .frame(
                    egui::Frame::default()
                        .fill(chrome::bg_color(miao_term_ui::theme::Rgb(0x24, 0x29, 0x33)))
                        .inner_margin(egui::Margin::same(8.0)),
                )
                .show(ctx, |ui| {
                    if let Some(i) = chrome::details_tabs(ui, &theme, &TABS, active) {
                        sel = Some(i);
                    }
                    ui.separator();
                    chrome::info(ui, &theme, title, &rows);
                });
            if let Some(i) = sel {
                self.details_tab = i;
            }
        }
        let status = self.status_text();
        egui::TopBottomPanel::bottom("status")
            .exact_height(STATUS_H)
            .frame(
                egui::Frame::default()
                    .fill(chrome::bg_color(miao_term_ui::theme::Rgb(0x1f, 0x23, 0x2b)))
                    .inner_margin(egui::Margin::symmetric(8.0, 2.0)),
            )
            .show(ctx, |ui| {
                ui.visuals_mut().override_text_color = Some(chrome::fg_color(&theme));
                ui.label(egui::RichText::new(status).size(11.0));
            });

        if let Some(i) = switch {
            self.active_tab = i;
            self.selection = None;
        }
        if new_tab {
            self.new_tab();
        }
        if let Some(i) = close {
            self.active_tab = i;
            self.close_pane();
        }
        if font_delta != 0.0 {
            self.font_size = (self.font_size + font_delta).clamp(6.0, 40.0);
            let (cw, ch) = State::cell_size(self.font_size);
            self.cw = cw;
            self.ch = ch;
            self.resize();
        }
        if let Some(i) = rename {
            self.renaming = Some(i);
            self.rename_buf = titles.get(i).cloned().unwrap_or_default();
        }
        self.palette_window(ctx);
        self.settings_window(ctx);
        if let Some(i) = self.renaming {
            self.rename_window(ctx, i);
        }
    }

    fn agent_badge(&self, pane_id: &str) -> Option<miao_term_ui::theme::Rgb> {
        use miao_term_ui::theme::Rgb;
        let a = self.mtp.agent_for(pane_id)?;
        let state = a.get("state").and_then(|v| v.as_str())?;
        Some(match state {
            "processing" => Rgb(0x81, 0xa1, 0xc1),
            "idle" => Rgb(0xa3, 0xbe, 0x8c),
            "awaiting" => Rgb(0xeb, 0xcb, 0x8b),
            "error" => Rgb(0xbf, 0x61, 0x6a),
            _ => Rgb(0x88, 0x88, 0x88),
        })
    }

    fn details_content(&mut self, tab: usize) -> (&'static str, Vec<(String, String)>) {
        match tab {
            1 => ("Agent", self.agent_rows()),
            2 => ("Outline", self.outline_rows()),
            3 => ("Git", self.git_rows_cached()),
            4 => ("Files", self.files_rows_cached()),
            _ => ("Info", self.details_rows()),
        }
    }

    fn active_pane(&self) -> Option<&Pane> {
        let tab = self.tabs.get(self.active_tab)?;
        tab.panes.iter().find(|p| p.id == tab.active)
    }

    fn cwd(&self) -> Option<std::path::PathBuf> {
        self.active_pane()
            .and_then(|p| p.term.cwd().map(std::path::PathBuf::from))
    }

    fn agent_rows(&self) -> Vec<(String, String)> {
        let mut rows = Vec::new();
        if let Some(p) = self.active_pane() {
            match self.mtp.agent_for(&p.id) {
                Some(a) => {
                    for k in ["agent", "state", "session_id", "tty"] {
                        if let Some(v) = a.get(k).and_then(|v| v.as_str()) {
                            rows.push((k.to_string(), v.to_string()));
                        }
                    }
                    if rows.is_empty() {
                        rows.push(("state".into(), a.to_string()));
                    }
                }
                None => rows.push(("agent".into(), "—".into())),
            }
        }
        rows
    }

    fn outline_rows(&self) -> Vec<(String, String)> {
        let mut rows = Vec::new();
        if let Some(p) = self.active_pane() {
            for e in self.mtp.history_for(&p.id).iter().rev().take(200) {
                let cmd = e.get("command").and_then(|v| v.as_str()).unwrap_or("");
                let cwd = e.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
                rows.push((cwd.to_string(), cmd.to_string()));
            }
        }
        rows
    }

    fn git_rows_cached(&mut self) -> Vec<(String, String)> {
        let Some(cwd) = self.cwd() else {
            return Vec::new();
        };
        if let Some((t, p, rows)) = &self.git_cache {
            if p == &cwd && t.elapsed() < Duration::from_millis(1000) {
                return rows.clone();
            }
        }
        let rows = git_rows(&cwd);
        self.git_cache = Some((Instant::now(), cwd, rows.clone()));
        rows
    }

    fn files_rows_cached(&mut self) -> Vec<(String, String)> {
        let Some(cwd) = self.cwd() else {
            return Vec::new();
        };
        if let Some((t, p, rows)) = &self.files_cache {
            if p == &cwd && t.elapsed() < Duration::from_millis(1000) {
                return rows.clone();
            }
        }
        let rows = files_rows(&cwd);
        self.files_cache = Some((Instant::now(), cwd, rows.clone()));
        rows
    }

    fn details_rows(&self) -> Vec<(String, String)> {
        let mut rows = Vec::new();
        if let Some(tab) = self.tabs.get(self.active_tab) {
            if let Some(pane) = tab.panes.iter().find(|p| p.id == tab.active) {
                rows.push(("Title".into(), self.title_of(tab)));
                rows.push((
                    "Directory".into(),
                    pane.term.cwd().unwrap_or("—").to_string(),
                ));
                let (r, c) = pane.term.screen().size();
                rows.push(("Size".into(), format!("{c} × {r}")));
                rows.push(("Pane".into(), pane.id.clone()));
            }
        }
        rows
    }

    fn status_text(&self) -> String {
        let panes = self
            .tabs
            .get(self.active_tab)
            .map(|t| t.panes.len())
            .unwrap_or(0);
        format!(
            "miaotty-native · tab {}/{} · {} pane(s) · {} tabs",
            self.active_tab + 1,
            self.tabs.len(),
            panes,
            self.tabs.len()
        )
    }
}

/// A palette command.
enum Cmd {
    NewTab,
    SplitRight,
    SplitDown,
    ClosePane,
    ToggleSidebar,
    ToggleDetails,
    FontUp,
    FontDown,
    Settings,
    Quit,
}

impl State {
    fn handles(&self) -> Vec<miao_term_ui::layout::Handle> {
        self.tabs
            .get(self.active_tab)
            .map(|t| t.layout.handles(self.grid_area()))
            .unwrap_or_default()
    }

    fn commands(&self) -> Vec<(Cmd, &'static str)> {
        vec![
            (Cmd::NewTab, "New Tab"),
            (Cmd::SplitRight, "Split Right"),
            (Cmd::SplitDown, "Split Down"),
            (Cmd::ClosePane, "Close Pane / Tab"),
            (Cmd::ToggleSidebar, "Toggle Sidebar"),
            (Cmd::ToggleDetails, "Toggle Details"),
            (Cmd::FontUp, "Increase Font Size"),
            (Cmd::FontDown, "Decrease Font Size"),
            (Cmd::Settings, "Settings"),
            (Cmd::Quit, "Quit"),
        ]
    }

    fn run_command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::NewTab => self.new_tab(),
            Cmd::SplitRight => self.split(SplitDir::Right),
            Cmd::SplitDown => self.split(SplitDir::Down),
            Cmd::ClosePane => self.close_pane(),
            Cmd::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
            Cmd::ToggleDetails => self.show_details = !self.show_details,
            Cmd::FontUp | Cmd::FontDown => {
                let d = if matches!(cmd, Cmd::FontUp) {
                    1.0
                } else {
                    -1.0
                };
                self.font_size = (self.font_size + d).clamp(6.0, 40.0);
                let (cw, ch) = State::cell_size(self.font_size);
                self.cw = cw;
                self.ch = ch;
                self.resize();
            }
            Cmd::Settings => self.show_settings = true,
            Cmd::Quit => {
                let s = self.window.inner_size();
                save_window_size(
                    s.width as f32 / self.window.scale_factor() as f32,
                    s.height as f32 / self.window.scale_factor() as f32,
                );
                std::process::exit(0);
            }
        }
        self.window.request_redraw();
    }

    fn palette_window(&mut self, ctx: &egui::Context) {
        if !self.show_palette {
            return;
        }
        let cmds = self.commands();
        let mut query = std::mem::take(&mut self.palette_query);
        let mut chosen: Option<usize> = None;
        let mut open = true;
        egui::Window::new("Command Palette")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 120.0])
            .open(&mut open)
            .show(ctx, |ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut query)
                        .hint_text("Type a command…")
                        .desired_width(420.0),
                );
                resp.request_focus();
                let q = query.to_lowercase();
                let filtered: Vec<usize> = (0..cmds.len())
                    .filter(|&i| q.is_empty() || cmds[i].1.to_lowercase().contains(&q))
                    .collect();
                for &i in &filtered {
                    if ui.selectable_label(false, cmds[i].1).clicked() {
                        chosen = Some(i);
                    }
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    chosen = filtered.first().copied();
                }
            });
        self.palette_query = query;
        if let Some(i) = chosen {
            if let Some((cmd, _)) = cmds.into_iter().nth(i) {
                self.run_command(cmd);
            }
            self.show_palette = false;
            self.palette_query.clear();
        }
        if !open {
            self.show_palette = false;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let mut font = self.font_size;
        let mut cursor = self.theme.cursor;
        let current_theme = self.theme_name.clone();
        let mut chosen_theme: Option<&'static str> = None;
        egui::Window::new("Settings")
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Font size");
                ui.add(egui::Slider::new(&mut font, 6.0..=40.0));
                ui.separator();
                ui.label("Cursor");
                ui.horizontal(|ui| {
                    for (s, n) in [
                        (miao_term_ui::CursorStyle::Block, "Block"),
                        (miao_term_ui::CursorStyle::Bar, "Bar"),
                        (miao_term_ui::CursorStyle::Underline, "Underline"),
                    ] {
                        if ui.radio(cursor == s, n).clicked() {
                            cursor = s;
                        }
                    }
                });
                ui.separator();
                ui.label("Theme");
                for name in Theme::NAMES {
                    if ui.selectable_label(current_theme == name, name).clicked() {
                        chosen_theme = Some(name);
                    }
                }
            });
        if (font - self.font_size).abs() > 0.01 {
            self.font_size = font;
            let (cw, ch) = State::cell_size(self.font_size);
            self.cw = cw;
            self.ch = ch;
            self.resize();
        }
        self.theme.cursor = cursor;
        if let Some(n) = chosen_theme {
            if let Some(mut t) = Theme::named(n) {
                t.cursor = self.theme.cursor;
                self.theme = t;
                self.theme_name = n.to_string();
                self.window.request_redraw();
            }
        }
        if !open {
            self.show_settings = false;
        }
    }

    fn rename_window(&mut self, ctx: &egui::Context, i: usize) {
        let mut open = true;
        let mut buf = std::mem::take(&mut self.rename_buf);
        let mut commit = false;
        egui::Window::new("Rename Tab")
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let resp = ui.text_edit_singleline(&mut buf);
                resp.request_focus();
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit = true;
                }
                if ui.button("Rename").clicked() {
                    commit = true;
                }
            });
        self.rename_buf = buf;
        if commit {
            if let Some(tab) = self.tabs.get_mut(i) {
                if !self.rename_buf.is_empty() {
                    tab.title = self.rename_buf.clone();
                }
            }
            self.renaming = None;
            self.publish_panes();
        }
        if !open {
            self.renaming = None;
        }
    }
}

fn quad(ox: f32, oy: f32, row: u16, col: u16, cw: f32, ch: f32, color: (u8, u8, u8)) -> Quad {
    Quad::new(
        (ox + col as f32 * cw, oy + row as f32 * ch),
        (ox + (col as f32 + 1.0) * cw, oy + (row as f32 + 1.0) * ch),
        (color.0, color.1, color.2, 255),
    )
}

fn gen_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    format!("pane{}", N.fetch_add(1, Ordering::SeqCst))
}

impl ApplicationHandler for Host {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let (init_w, init_h) = load_window_size().unwrap_or((1100.0, 720.0));
        let attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(init_w, init_h));
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
                    eprintln!("miaotty-native: no wgpu adapter");
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
            quads,
            renderers: HashMap::new(),
            mtp: self.mtp.clone(),
            tabs: Vec::new(),
            active_tab: 0,
            theme: Theme::nord(),
            cw,
            ch,
            font_size: FONT_SIZE,
            mods: ModifiersState::empty(),
            selection: None,
            dragging: false,
            divider_drag: None,
            cursor: (0.0, 0.0),
            show_sidebar: true,
            show_details: true,
            renaming: None,
            rename_buf: String::new(),
            theme_name: "Nord".to_string(),
            show_palette: false,
            palette_query: String::new(),
            show_settings: false,
            details_tab: 0,
            git_cache: None,
            files_cache: None,
            last_title: None,
            focused: false,
            cursor_on: true,
            last_blink: Instant::now(),
            egui_ctx,
            egui_state,
            egui_renderer,
        };
        state.new_tab();
        state.window.request_redraw();
        self.state = Some(state);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
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
        if !matches!(event, WindowEvent::RedrawRequested) {
            let resp = state.egui_state.on_window_event(&state.window, &event);
            if resp.repaint {
                state.window.request_redraw();
            }
        }
        match event {
            WindowEvent::CloseRequested => {
                let s = state.window.inner_size();
                save_window_size(
                    s.width as f32 / state.window.scale_factor() as f32,
                    s.height as f32 / state.window.scale_factor() as f32,
                );
                event_loop.exit();
            }
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
            } => match button {
                MouseButton::Left => {
                    if es == ElementState::Pressed {
                        // Prefer a divider under the pointer, else a selection.
                        let scale = state.window.scale_factor() as f32;
                        let (px, py) = (state.cursor.0 as f32, state.cursor.1 as f32);
                        let hit = state.handles().into_iter().find(|h| {
                            px >= h.rect.x * scale
                                && px < (h.rect.x + h.rect.w) * scale
                                && py >= h.rect.y * scale
                                && py < (h.rect.y + h.rect.h) * scale
                        });
                        match hit {
                            Some(h) => state.divider_drag = Some((h.path, h.dir, h.area)),
                            None => {
                                state.dragging = true;
                                state.selection = None;
                            }
                        }
                    } else {
                        if state.divider_drag.take().is_none() {
                            let ctx = state.egui_ctx.clone();
                            state.copy_selection(&ctx);
                        }
                        state.dragging = false;
                    }
                }
                MouseButton::Right if es == ElementState::Pressed => {
                    if state.selection.is_some() {
                        let ctx = state.egui_ctx.clone();
                        state.copy_selection(&ctx);
                        state.selection = None;
                    } else {
                        // Paste.
                        let text = state.egui_state.clipboard_text().unwrap_or_default();
                        if !text.is_empty() {
                            state.paste(&text);
                        }
                    }
                }
                _ => {}
            },
            WindowEvent::CursorMoved { position, .. } => {
                state.cursor = (position.x, position.y);
                let scale = state.window.scale_factor() as f32;
                let (px, py) = (position.x as f32, position.y as f32);
                if let Some((path, dir, area)) = state.divider_drag.clone() {
                    let r = match dir {
                        SplitDir::Right => (px / scale - area.x) / area.w.max(1.0),
                        SplitDir::Down => (py / scale - area.y) / area.h.max(1.0),
                    };
                    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                        tab.layout.set_ratio(&path, r);
                    }
                    state.window.request_redraw();
                } else if state.dragging {
                    let rects = state.pane_rects();
                    if let Some((id, r)) = rects.iter().find(|(_, r)| {
                        px >= r.x * scale
                            && px < (r.x + r.w) * scale
                            && py >= r.y * scale
                            && py < (r.y + r.h) * scale
                    }) {
                        let cw = state.cw * scale;
                        let ch = state.ch * scale;
                        let col = ((px - r.x * scale - GUTTER) / cw).floor().max(0.0) as u16;
                        let row = ((py - r.y * scale - GUTTER) / ch).floor().max(0.0) as u16;
                        let cell = (row, col);
                        match &mut state.selection {
                            Some((sid, sel)) if sid == id => sel.end = cell,
                            _ => state.selection = Some((id.clone(), Selection::cell(cell))),
                        }
                        state.window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                use winit::event::MouseScrollDelta;
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as i32,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 12.0) as i32,
                };
                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                    if let Some(pane) = tab.panes.iter_mut().find(|p| p.id == tab.active) {
                        let max = pane.term.screen().scrollback_len();
                        if lines > 0 {
                            pane.scroll = (pane.scroll + lines as usize).min(max);
                        } else {
                            pane.scroll = pane.scroll.saturating_sub((-lines) as usize);
                        }
                    }
                }
                state.window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(s) = shortcut(&event, state.mods) {
                    if s.new_tab {
                        state.new_tab();
                    }
                    if s.close {
                        state.close_pane();
                    }
                    if let Some(i) = s.select {
                        if i < state.tabs.len() {
                            state.active_tab = i;
                            state.selection = None;
                        }
                    }
                    if s.font != 0.0 {
                        state.font_size = (state.font_size + s.font).clamp(6.0, 40.0);
                        let (cw, ch) = State::cell_size(state.font_size);
                        state.cw = cw;
                        state.ch = ch;
                        state.resize();
                    }
                    if s.split_right {
                        state.split(SplitDir::Right);
                    }
                    if s.split_down {
                        state.split(SplitDir::Down);
                    }
                    if s.cycle != 0 {
                        state.cycle_pane(s.cycle > 0);
                    }
                    if s.toggle_sidebar {
                        state.show_sidebar = !state.show_sidebar;
                    }
                    if s.toggle_details {
                        state.show_details = !state.show_details;
                    }
                    if s.palette {
                        state.show_palette = true;
                        state.palette_query.clear();
                    }
                    if s.settings {
                        state.show_settings = true;
                    }
                    state.window.request_redraw();
                } else {
                    let app_cursor = state
                        .tabs
                        .get(state.active_tab)
                        .and_then(|t| t.panes.iter().find(|p| p.id == t.active))
                        .map(|p| p.term.screen().application_cursor())
                        .unwrap_or(false);
                    let key = key_input(&event);
                    let mods = input::Modifiers {
                        ctrl: state.mods.control_key(),
                        alt: state.mods.alt_key(),
                        shift: state.mods.shift_key(),
                        sup: state.mods.super_key(),
                    };
                    let bytes = input::encode(&key, mods, app_cursor);
                    state.write_input(&bytes);
                }
            }
            WindowEvent::RedrawRequested => state.render(),
            _ => {}
        }
    }
}

fn key_input(event: &KeyEvent) -> input::KeyInput {
    let kind = match &event.logical_key {
        Key::Character(s) => s.chars().next().map(input::KeyKind::Char),
        Key::Named(n) => Some(match n {
            NamedKey::Enter => input::KeyKind::Enter,
            NamedKey::Backspace => input::KeyKind::Backspace,
            NamedKey::Tab => input::KeyKind::Tab,
            NamedKey::Escape => input::KeyKind::Escape,
            NamedKey::ArrowUp => input::KeyKind::Up,
            NamedKey::ArrowDown => input::KeyKind::Down,
            NamedKey::ArrowLeft => input::KeyKind::Left,
            NamedKey::ArrowRight => input::KeyKind::Right,
            NamedKey::Home => input::KeyKind::Home,
            NamedKey::End => input::KeyKind::End,
            NamedKey::Delete => input::KeyKind::Delete,
            NamedKey::PageUp => input::KeyKind::PageUp,
            NamedKey::PageDown => input::KeyKind::PageDown,
            _ => input::KeyKind::Other,
        }),
        _ => Some(input::KeyKind::Other),
    }
    .unwrap_or(input::KeyKind::Other);
    input::KeyInput {
        kind,
        text: event.text.as_ref().map(|t| t.to_string()),
    }
}

// ---- helpers ---------------------------------------------------------------

fn git_rows(cwd: &std::path::Path) -> Vec<(String, String)> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["status", "--porcelain=v1", "-b"])
        .output();
    let Ok(out) = out else {
        return vec![("git".into(), "unavailable".into())];
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut rows = Vec::new();
    for line in text.lines().take(200) {
        if let Some(branch) = line.strip_prefix("## ") {
            rows.push(("branch".into(), branch.to_string()));
        } else if line.len() > 3 {
            rows.push((line[..2].to_string(), line[3..].to_string()));
        }
    }
    if rows.is_empty() {
        rows.push(("status".into(), "clean".into()));
    }
    rows
}

fn files_rows(cwd: &std::path::Path) -> Vec<(String, String)> {
    let Ok(read) = std::fs::read_dir(cwd) else {
        return Vec::new();
    };
    let mut rows: Vec<(String, String)> = read
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .take(200)
        .map(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            (
                e.file_name().to_string_lossy().to_string(),
                if is_dir { "dir".into() } else { "file".into() },
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
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

fn window_file() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("miaotty").join("native-window"))
}

fn load_window_size() -> Option<(f32, f32)> {
    let text = std::fs::read_to_string(window_file()?).ok()?;
    let mut it = text.split_whitespace();
    let w: f32 = it.next()?.parse().ok()?;
    let h: f32 = it.next()?.parse().ok()?;
    Some((w, h))
}

fn save_window_size(w: f32, h: f32) {
    if let Some(path) = window_file() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, format!("{w:.0} {h:.0}\n"));
    }
}
