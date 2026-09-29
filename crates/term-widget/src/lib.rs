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

const MENU_H: f32 = 24.0;
const TAB_H: f32 = 30.0;
const STATUS_H: f32 = 22.0;
const SIDEBAR_W: f32 = 200.0;
const DETAILS_W: f32 = 300.0;
const CARD_MARGIN: f32 = 6.0;
const CARD_PAD: f32 = 8.0;
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

/// A simple built-in text file editor (with a naive Markdown preview).
struct Editor {
    path: std::path::PathBuf,
    text: String,
    original: String,
    preview: bool,
    /// `(destination, remote path)` when editing a file over ssh.
    remote: Option<(String, String)>,
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
    rules: miao_term_config::view::RuleSet,
    cw: f32,
    ch: f32,
    font_size: f32,
    line_ratio: f32,
    font_family: Option<String>,
    lang: miao_term_ui::i18n::Lang,
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
    editor: Option<Editor>,
    open_path: String,
    show_open: bool,
    recipe_dialog: Option<bool>,
    recipe_name: String,
    recipe_list: Vec<String>,
    ssh_dialog: Option<String>,
    remote_dialog: Option<(String, String)>,
    details_tab: usize,
    git_cache: Cache,
    files_cache: Cache,
    ports_cache: Cache,
    prompts: Vec<String>,
    prompt_input: String,
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
    fn cell_size(font_size: f32, line_ratio: f32, family: Option<&str>) -> (f32, f32) {
        let mut probe = miao_term_render::MetricsProbe::new();
        probe.cell(font_size, (font_size * line_ratio).round(), family)
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
            y: MENU_H + TAB_H,
            w: (w - x - right).max(1.0),
            h: (h - MENU_H - TAB_H - STATUS_H).max(1.0),
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

    fn spawn_pane(&self, cwd: Option<std::path::PathBuf>) -> Option<Pane> {
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
        Terminal::new(None, cols, rows, 10_000, cwd, &env, waker)
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
        self.save_session();
    }

    fn new_tab(&mut self) {
        let Some(pane) = self.spawn_pane(None) else {
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
        let Some(pane) = self.spawn_pane(None) else {
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
                    let inner = card_inner(*r);
                    let cols = ((inner.w * scale) / cw).floor().max(1.0) as u16;
                    let rows = ((inner.h * scale) / ch).floor().max(1.0) as u16;
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

    /// Evaluate the view rule engine (ADR 0007) for a tab's active pane.
    fn view_for(&self, tab: &Tab) -> Option<miao_term_config::view::Resolved> {
        let pane = tab.panes.iter().find(|p| p.id == tab.active)?;
        let agent = self
            .mtp
            .agent_for(&pane.id)
            .and_then(|a| a.get("agent").and_then(|v| v.as_str()).map(str::to_string));
        let ctx = miao_term_config::view::Context {
            cwd: pane.term.cwd().map(str::to_string),
            command: None,
            agent,
            host: None,
            file: None,
            user: std::env::var("USER").ok(),
            shell: std::env::var("SHELL").ok(),
            branch: None,
            osc_title: pane.term.title().map(str::to_string),
            index: None,
        };
        self.rules.evaluate(&ctx)
    }

    fn title_of(&self, tab: &Tab) -> String {
        if let Some(res) = self.view_for(tab) {
            if !res.title.is_empty() {
                return res.title;
            }
            if let Some(alias) = res.alias {
                return alias;
            }
        }
        // Fall back to the program title, then the cwd folder, then "shell N".
        if let Some(t) = tab
            .panes
            .iter()
            .find(|p| p.id == tab.active)
            .and_then(|p| p.term.title().map(str::to_string))
            .filter(|s| !s.is_empty())
        {
            return t;
        }
        tab.panes
            .iter()
            .find(|p| p.id == tab.active)
            .and_then(|p| p.term.cwd().map(str::to_string))
            .and_then(|c| {
                std::path::Path::new(&c)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
            })
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
        let window_bg = theme.bg;
        let selection = self.selection.clone();
        let rects = self.pane_rects();
        let active_id = self.active_pane_id().unwrap_or_default();
        let mut draws: Vec<PaneDraw> = Vec::new();
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            for (id, r) in &rects {
                let Some(pane) = tab.panes.iter_mut().find(|p| &p.id == id) else {
                    continue;
                };
                let inner = card_inner(*r);
                let cols = ((inner.w * scale) / cw).floor().max(1.0) as u16;
                let rows = ((inner.h * scale) / ch).floor().max(1.0) as u16;
                pane.term.resize(rows, cols);
                pane.term.screen_mut().set_scrollback(pane.scroll);
                let (sr, sc) = pane.term.size();
                let ox = inner.x * scale;
                let oy = inner.y * scale;

                let mut quads: Vec<Quad> = Vec::new();
                // Container card (border + terminal background), Otty-style.
                let card = Rect {
                    x: r.x + CARD_MARGIN,
                    y: r.y + CARD_MARGIN,
                    w: (r.w - CARD_MARGIN * 2.0).max(1.0),
                    h: (r.h - CARD_MARGIN * 2.0).max(1.0),
                };
                let border = lighten(theme.bg, 0.12);
                let bg = theme.bg;
                quads.push(Quad::new(
                    (card.x * scale, card.y * scale),
                    ((card.x + card.w) * scale, (card.y + card.h) * scale),
                    (border.0, border.1, border.2, 255),
                ));
                quads.push(Quad::new(
                    (card.x * scale + 1.0, card.y * scale + 1.0),
                    (
                        (card.x + card.w) * scale - 1.0,
                        (card.y + card.h) * scale - 1.0,
                    ),
                    (bg.0, bg.1, bg.2, 255),
                ));
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
                    rect: inner,
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
                (self.font_size * self.line_ratio).round(),
                self.cw,
                d.rect.x,
                d.rect.y,
                (theme.fg.0, theme.fg.1, theme.fg.2),
                self.font_family.as_deref(),
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
                                r: window_bg.0 as f64 / 255.0,
                                g: window_bg.1 as f64 / 255.0,
                                b: window_bg.2 as f64 / 255.0,
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

    fn menu_bar(&mut self, ctx: &egui::Context) {
        use miao_term_ui::i18n::t;
        let lang = self.lang;
        let theme = self.theme.clone();
        let mut action: Option<Cmd> = None;
        egui::TopBottomPanel::top("menu")
            .exact_height(MENU_H)
            .frame(
                egui::Frame::default()
                    .fill(chrome::bg_color(theme.bg))
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        chrome::bg_color(lighten(theme.bg, 0.10)),
                    ))
                    .inner_margin(egui::Margin::symmetric(6.0, 1.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.visuals_mut().override_text_color = Some(chrome::fg_color(&theme));
                    ui.menu_button(t(lang, "File", "文件"), |ui| {
                        if ui.button(t(lang, "New Tab", "新建标签")).clicked() {
                            action = Some(Cmd::NewTab);
                            ui.close_menu();
                        }
                        if ui
                            .button(t(lang, "Close Pane / Tab", "关闭 Pane/标签"))
                            .clicked()
                        {
                            action = Some(Cmd::ClosePane);
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button(t(lang, "Quit", "退出")).clicked() {
                            action = Some(Cmd::Quit);
                            ui.close_menu();
                        }
                    });
                    ui.menu_button(t(lang, "Edit", "编辑"), |ui| {
                        if ui.button(t(lang, "Copy", "复制")).clicked() {
                            action = Some(Cmd::Copy);
                            ui.close_menu();
                        }
                        if ui.button(t(lang, "Paste", "粘贴")).clicked() {
                            action = Some(Cmd::Paste);
                            ui.close_menu();
                        }
                    });
                    ui.menu_button(t(lang, "View", "视图"), |ui| {
                        if ui.button(t(lang, "Toggle Sidebar", "开关侧栏")).clicked() {
                            action = Some(Cmd::ToggleSidebar);
                            ui.close_menu();
                        }
                        if ui.button(t(lang, "Toggle Details", "开关详情")).clicked() {
                            action = Some(Cmd::ToggleDetails);
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui
                            .button(t(lang, "Increase Font Size", "增大字号"))
                            .clicked()
                        {
                            action = Some(Cmd::FontUp);
                            ui.close_menu();
                        }
                        if ui
                            .button(t(lang, "Decrease Font Size", "减小字号"))
                            .clicked()
                        {
                            action = Some(Cmd::FontDown);
                            ui.close_menu();
                        }
                        if ui.button(t(lang, "Settings", "设置")).clicked() {
                            action = Some(Cmd::Settings);
                            ui.close_menu();
                        }
                    });
                    ui.menu_button(t(lang, "Shell", "终端"), |ui| {
                        if ui.button(t(lang, "Split Right", "向右分屏")).clicked() {
                            action = Some(Cmd::SplitRight);
                            ui.close_menu();
                        }
                        if ui.button(t(lang, "Split Down", "向下分屏")).clicked() {
                            action = Some(Cmd::SplitDown);
                            ui.close_menu();
                        }
                    });
                    ui.menu_button(t(lang, "Help", "帮助"), |ui| {
                        ui.hyperlink_to(
                            t(lang, "Documentation", "文档"),
                            "https://github.com/oxdingzg/miao-term#readme",
                        );
                    });
                });
            });
        if let Some(a) = action {
            self.run_command(a);
        }
    }

    fn chrome(&mut self, ctx: &egui::Context) {
        self.menu_bar(ctx);
        let lang = self.lang;
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
                    .fill(chrome::bg_color(theme.bg))
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        chrome::bg_color(lighten(theme.bg, 0.10)),
                    ))
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
            let heading = miao_term_ui::i18n::t(lang, "Sessions", "会话");
            let mut s = None;
            egui::SidePanel::left("sessions")
                .exact_width(SIDEBAR_W)
                .frame(
                    egui::Frame::default()
                        .fill(chrome::bg_color(theme.bg))
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            chrome::bg_color(lighten(theme.bg, 0.10)),
                        ))
                        .inner_margin(egui::Margin::same(6.0)),
                )
                .show(ctx, |ui| {
                    s = chrome::sidebar(ui, &theme, &titles, &badges, active, heading);
                });
            if let Some(i) = s {
                switch = Some(i);
            }
        }
        if self.show_details {
            use miao_term_ui::i18n::t;
            let tabs = [
                t(lang, "Info", "信息"),
                t(lang, "Agent", "Agent"),
                t(lang, "Outline", "大纲"),
                t(lang, "Git", "Git"),
                t(lang, "Files", "文件"),
                t(lang, "Ports", "端口"),
                t(lang, "Queue", "队列"),
            ];
            let active = self.details_tab.min(tabs.len() - 1);
            let (title, rows) = if active == 6 {
                ("Queue".to_string(), Vec::new())
            } else {
                let (t, r) = self.details_content(active);
                (t.to_string(), r)
            };
            let mut sel = None;
            let mut qev = chrome::QueueEvents::default();
            egui::SidePanel::right("details")
                .exact_width(DETAILS_W)
                .frame(
                    egui::Frame::default()
                        .fill(chrome::bg_color(theme.bg))
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            chrome::bg_color(lighten(theme.bg, 0.10)),
                        ))
                        .inner_margin(egui::Margin::same(8.0)),
                )
                .show(ctx, |ui| {
                    if let Some(i) = chrome::details_tabs(ui, &theme, &tabs, active) {
                        sel = Some(i);
                    }
                    ui.separator();
                    if active == 6 {
                        qev = chrome::queue(ui, &theme, &self.prompts, &mut self.prompt_input);
                    } else {
                        chrome::info(ui, &theme, &title, &rows);
                    }
                });
            if let Some(i) = sel {
                self.details_tab = i;
            }
            if qev.add && !self.prompt_input.is_empty() {
                self.prompts.push(std::mem::take(&mut self.prompt_input));
            }
            if let Some(i) = qev.send {
                if let Some(item) = self.prompts.get(i).cloned() {
                    self.write_input(format!("{item}\r").as_bytes());
                }
            }
            if qev.send_all {
                let items = std::mem::take(&mut self.prompts);
                for item in items {
                    self.write_input(format!("{item}\r").as_bytes());
                }
            }
            if let Some(i) = qev.remove {
                if i < self.prompts.len() {
                    self.prompts.remove(i);
                }
            }
            if qev.clear {
                self.prompts.clear();
            }
        }
        let status = self.status_text();
        egui::TopBottomPanel::bottom("status")
            .exact_height(STATUS_H)
            .frame(
                egui::Frame::default()
                    .fill(chrome::bg_color(theme.bg))
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        chrome::bg_color(lighten(theme.bg, 0.10)),
                    ))
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
            let (cw, ch) =
                State::cell_size(self.font_size, self.line_ratio, self.font_family.as_deref());
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
        self.open_dialog_window(ctx);
        self.editor_window(ctx);
        self.recipe_dialog_window(ctx);
        self.ssh_dialog_window(ctx);
        self.remote_dialog_window(ctx);
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
            5 => ("Ports", self.ports_rows_cached()),
            _ => ("Info", self.details_rows()),
        }
    }

    /// Persist tabs/panes/cwd/layout so the next launch restores the session.
    fn session_value(&self) -> serde_json::Value {
        let mut tabs = Vec::new();
        for tab in &self.tabs {
            let panes: Vec<serde_json::Value> = tab
                .panes
                .iter()
                .map(|p| serde_json::json!({ "id": p.id, "cwd": p.term.cwd() }))
                .collect();
            tabs.push(serde_json::json!({
                "title": tab.title,
                "active": tab.active,
                "layout": layout_to_json(&tab.layout),
                "panes": panes,
            }));
        }
        serde_json::json!({ "active_tab": self.active_tab, "tabs": tabs })
    }

    fn save_session(&self) {
        if let Some(path) = session_file() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(
                path,
                serde_json::to_vec(&self.session_value()).unwrap_or_default(),
            );
        }
    }

    /// Drop every tab (and its panes/shells), e.g. before opening a recipe.
    fn clear_tabs(&mut self) {
        self.tabs.clear();
        self.selection = None;
        self.active_tab = 0;
    }

    fn restore_from_value(&mut self, v: &serde_json::Value) -> bool {
        let Some(tabs) = v.get("tabs").and_then(|t| t.as_array()) else {
            return false;
        };
        for t in tabs {
            let title = t
                .get("title")
                .and_then(|x| x.as_str())
                .unwrap_or("shell")
                .to_string();
            let mut panes = Vec::new();
            let mut map = std::collections::HashMap::new();
            if let Some(arr) = t.get("panes").and_then(|p| p.as_array()) {
                for p in arr {
                    let cwd = p
                        .get("cwd")
                        .and_then(|x| x.as_str())
                        .map(std::path::PathBuf::from);
                    if let Some(pane) = self.spawn_pane(cwd) {
                        if let Some(old) = p.get("id").and_then(|x| x.as_str()) {
                            map.insert(old.to_string(), pane.id.clone());
                        }
                        panes.push(pane);
                    }
                }
            }
            if panes.is_empty() {
                continue;
            }
            let layout = t
                .get("layout")
                .and_then(|l| json_to_layout(l, &map))
                .unwrap_or_else(|| Layout::leaf(panes[0].id.clone()));
            let active = t
                .get("active")
                .and_then(|x| x.as_str())
                .map(|old| map.get(old).cloned().unwrap_or_else(|| old.to_string()))
                .filter(|id| panes.iter().any(|p| &p.id == id))
                .unwrap_or_else(|| panes[0].id.clone());
            self.tabs.push(Tab {
                layout,
                panes,
                active,
                title,
            });
        }
        if self.tabs.is_empty() {
            return false;
        }
        self.active_tab = v.get("active_tab").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        self.active_tab = self.active_tab.min(self.tabs.len() - 1);
        self.publish_panes();
        true
    }

    /// Restore a saved session; returns false if there is nothing to restore.
    fn restore_session(&mut self) -> bool {
        let Some(path) = session_file() else {
            return false;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return false;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return false;
        };
        self.restore_from_value(&v)
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

    fn ports_rows_cached(&mut self) -> Vec<(String, String)> {
        let pid = self.active_pane().and_then(|p| p.term.pid());
        let key = std::path::PathBuf::from(pid.map(|p| p.to_string()).unwrap_or_default());
        if let Some((t, p, rows)) = &self.ports_cache {
            if p == &key && t.elapsed() < Duration::from_millis(1000) {
                return rows.clone();
            }
        }
        let rows = match pid {
            Some(pid) => ports_rows(pid),
            None => Vec::new(),
        };
        self.ports_cache = Some((Instant::now(), key, rows.clone()));
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
    NewSsh,
    OpenRemote,
    SaveRecipe,
    OpenRecipe,
    OpenFile,
    Save,
    Copy,
    Paste,
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
            (Cmd::NewSsh, "New SSH Session…"),
            (Cmd::OpenRemote, "Open Remote File…"),
            (Cmd::SaveRecipe, "Save Recipe…"),
            (Cmd::OpenRecipe, "Open Recipe…"),
            (Cmd::OpenFile, "Open File…"),
            (Cmd::Save, "Save"),
            (Cmd::Copy, "Copy"),
            (Cmd::Paste, "Paste"),
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
                let (cw, ch) =
                    State::cell_size(self.font_size, self.line_ratio, self.font_family.as_deref());
                self.cw = cw;
                self.ch = ch;
                self.resize();
            }
            Cmd::NewSsh => self.ssh_dialog = Some(String::new()),
            Cmd::OpenRemote => self.remote_dialog = Some((String::new(), String::new())),
            Cmd::SaveRecipe => {
                self.recipe_name.clear();
                self.recipe_dialog = Some(true);
            }
            Cmd::OpenRecipe => {
                self.recipe_list = list_recipes();
                self.recipe_dialog = Some(false);
            }
            Cmd::OpenFile => self.show_open = true,
            Cmd::Save => {
                if let Some(ed) = self.editor.as_mut() {
                    match &ed.remote {
                        Some((dest, path)) => {
                            let _ = miao_term_ui::ssh::write_remote(dest, path, ed.text.as_bytes());
                        }
                        None => {
                            let _ = std::fs::write(&ed.path, &ed.text);
                        }
                    }
                    ed.original = ed.text.clone();
                }
            }
            Cmd::Copy => {
                let ctx = self.egui_ctx.clone();
                self.copy_selection(&ctx);
            }
            Cmd::Paste => {
                let text = self.egui_state.clipboard_text().unwrap_or_default();
                if !text.is_empty() {
                    self.paste(&text);
                }
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
        egui::Window::new(miao_term_ui::i18n::t(
            self.lang,
            "Command Palette",
            "命令面板",
        ))
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
        egui::Window::new(miao_term_ui::i18n::t(self.lang, "Settings", "设置"))
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(miao_term_ui::i18n::t(self.lang, "Font size", "字号"));
                ui.add(egui::Slider::new(&mut font, 6.0..=40.0));
                ui.separator();
                ui.label(miao_term_ui::i18n::t(self.lang, "Cursor", "光标"));
                ui.horizontal(|ui| {
                    for (s, n) in [
                        (
                            miao_term_ui::CursorStyle::Block,
                            miao_term_ui::i18n::t(self.lang, "Block", "方块"),
                        ),
                        (
                            miao_term_ui::CursorStyle::Bar,
                            miao_term_ui::i18n::t(self.lang, "Bar", "竖线"),
                        ),
                        (
                            miao_term_ui::CursorStyle::Underline,
                            miao_term_ui::i18n::t(self.lang, "Underline", "下划线"),
                        ),
                    ] {
                        if ui.radio(cursor == s, n).clicked() {
                            cursor = s;
                        }
                    }
                });
                ui.separator();
                ui.label(miao_term_ui::i18n::t(self.lang, "Theme", "主题"));
                for name in Theme::NAMES {
                    if ui.selectable_label(current_theme == name, name).clicked() {
                        chosen_theme = Some(name);
                    }
                }
            });
        if (font - self.font_size).abs() > 0.01 {
            self.font_size = font;
            let (cw, ch) =
                State::cell_size(self.font_size, self.line_ratio, self.font_family.as_deref());
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

    fn open_ssh(&mut self, input: &str) {
        let Some(target) = miao_term_ui::ssh::Target::parse(input) else {
            return;
        };
        let resolved = miao_term_ui::ssh::resolve(&target);
        let cmd =
            miao_term_ui::ssh::command(&resolved, &miao_term_ui::ssh::bootstrap("xterm-256color"));
        self.new_tab();
        if let Some(tab) = self.tabs.last_mut() {
            tab.title = resolved.destination();
            let active = tab.active.clone();
            if let Some(pane) = tab.panes.iter_mut().find(|p| p.id == active) {
                pane.term.write(format!("{cmd}\r").as_bytes());
            }
        }
        self.publish_panes();
    }

    fn ssh_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(ref mut input) = self.ssh_dialog else {
            return;
        };
        let mut open = true;
        let mut connect = false;
        egui::Window::new(miao_term_ui::i18n::t(
            self.lang,
            "New SSH Session",
            "新建 SSH 会话",
        ))
        .collapsible(false)
        .open(&mut open)
        .show(ctx, |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(input)
                    .hint_text("[user@]host[:port]")
                    .desired_width(280.0),
            );
            r.request_focus();
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                connect = true;
            }
            if ui
                .button(miao_term_ui::i18n::t(self.lang, "Connect", "连接"))
                .clicked()
            {
                connect = true;
            }
        });
        if connect {
            let target = input.clone();
            self.ssh_dialog = None;
            self.open_ssh(&target);
        } else if !open {
            self.ssh_dialog = None;
        }
    }

    fn remote_dialog_window(&mut self, ctx: &egui::Context) {
        let Some((dest, path)) = self.remote_dialog.as_mut() else {
            return;
        };
        let mut open = true;
        let mut do_open = false;
        egui::Window::new(miao_term_ui::i18n::t(
            self.lang,
            "Open Remote File",
            "打开远端文件",
        ))
        .collapsible(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("SSH");
                ui.add(
                    egui::TextEdit::singleline(dest)
                        .hint_text("host")
                        .desired_width(140.0),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Path");
                ui.add(
                    egui::TextEdit::singleline(path)
                        .hint_text("/etc/hosts")
                        .desired_width(240.0),
                );
            });
            if ui
                .button(miao_term_ui::i18n::t(self.lang, "Open", "打开"))
                .clicked()
            {
                do_open = true;
            }
        });
        if do_open {
            let (dest, path) = (dest.clone(), path.clone());
            self.remote_dialog = None;
            match miao_term_ui::ssh::read_remote(&dest, &path) {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes).to_string();
                    self.editor = Some(Editor {
                        path: std::path::PathBuf::from(&path),
                        original: text.clone(),
                        text,
                        preview: path.ends_with(".md"),
                        remote: Some((dest, path)),
                    });
                }
                Err(e) => eprintln!("remote read failed: {e}"),
            }
        } else if !open {
            self.remote_dialog = None;
        }
    }

    fn recipe_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(save) = self.recipe_dialog else {
            return;
        };
        use miao_term_ui::i18n::t;
        let lang = self.lang;
        let mut open = true;
        let mut do_save = false;
        let mut open_recipe: Option<String> = None;
        let title = if save {
            t(lang, "Save Recipe", "保存配方")
        } else {
            t(lang, "Open Recipe", "打开配方")
        };
        egui::Window::new(title)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                if save {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.recipe_name)
                            .hint_text("name")
                            .desired_width(240.0),
                    );
                    r.request_focus();
                    if ui.button(t(lang, "Save", "保存")).clicked() {
                        do_save = true;
                    }
                } else if self.recipe_list.is_empty() {
                    ui.label(t(lang, "No recipes yet", "还没有配方"));
                } else {
                    for name in &self.recipe_list {
                        if ui.button(name).clicked() {
                            open_recipe = Some(name.clone());
                        }
                    }
                }
            });
        if do_save && !self.recipe_name.is_empty() {
            if let Some(dir) = recipes_dir() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(
                    dir.join(format!("{}.json", self.recipe_name)),
                    serde_json::to_vec(&self.session_value()).unwrap_or_default(),
                );
            }
            self.recipe_dialog = None;
        }
        if let Some(name) = open_recipe {
            if let Some(dir) = recipes_dir() {
                if let Ok(bytes) = std::fs::read(dir.join(format!("{name}.json"))) {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        self.clear_tabs();
                        if !self.restore_from_value(&v) {
                            self.new_tab();
                        }
                    }
                }
            }
            self.recipe_dialog = None;
        }
        if !open {
            self.recipe_dialog = None;
        }
    }

    fn open_dialog_window(&mut self, ctx: &egui::Context) {
        if !self.show_open {
            return;
        }
        let mut open = true;
        let mut path = std::mem::take(&mut self.open_path);
        let mut do_open = false;
        egui::Window::new(miao_term_ui::i18n::t(self.lang, "Open File", "打开文件"))
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut path)
                        .hint_text("/path/to/file")
                        .desired_width(360.0),
                );
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    do_open = true;
                }
                if ui
                    .button(miao_term_ui::i18n::t(self.lang, "Open", "打开"))
                    .clicked()
                {
                    do_open = true;
                }
            });
        self.open_path = path;
        if do_open && !self.open_path.is_empty() {
            let p = std::path::PathBuf::from(&self.open_path);
            if let Ok(text) = std::fs::read_to_string(&p) {
                self.editor = Some(Editor {
                    path: p,
                    original: text.clone(),
                    text,
                    preview: false,
                    remote: None,
                });
                self.show_open = false;
            }
        }
        if !open {
            self.show_open = false;
        }
    }

    fn editor_window(&mut self, ctx: &egui::Context) {
        let Some(ed) = self.editor.as_mut() else {
            return;
        };
        let title = match &ed.remote {
            Some((dest, path)) => format!("{dest}:{path}"),
            None => ed
                .path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| ed.path.display().to_string()),
        };
        let modified = ed.text != ed.original;
        let mut open = true;
        let mut save = false;
        egui::Window::new(title)
            .open(&mut open)
            .default_size([640.0, 480.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .button(miao_term_ui::i18n::t(self.lang, "Save", "保存"))
                        .clicked()
                    {
                        save = true;
                    }
                    ui.checkbox(
                        &mut ed.preview,
                        miao_term_ui::i18n::t(self.lang, "Markdown preview", "Markdown 预览"),
                    );
                    if modified {
                        ui.label(
                            egui::RichText::new(miao_term_ui::i18n::t(
                                self.lang,
                                "modified",
                                "已修改",
                            ))
                            .color(egui::Color32::from_rgb(0xeb, 0xcb, 0x8b)),
                        );
                    }
                });
                ui.separator();
                if ed.preview {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| markdown_preview(ui, &ed.text));
                } else {
                    egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                let lines = ed.text.lines().count().max(1);
                                let mut nums = String::new();
                                for i in 1..=lines {
                                    nums.push_str(&format!("{i:>4}\n"));
                                }
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(nums)
                                            .monospace()
                                            .color(egui::Color32::from_gray(110)),
                                    )
                                    .selectable(false),
                                );
                                ui.add(
                                    egui::TextEdit::multiline(&mut ed.text)
                                        .code_editor()
                                        .desired_width(f32::INFINITY),
                                );
                            });
                        });
                }
            });
        if save {
            match &ed.remote {
                Some((dest, path)) => {
                    let _ = miao_term_ui::ssh::write_remote(dest, path, ed.text.as_bytes());
                }
                None => {
                    let _ = std::fs::write(&ed.path, &ed.text);
                }
            }
            ed.original = ed.text.clone();
        }
        if !open {
            self.editor = None;
        }
    }

    fn rename_window(&mut self, ctx: &egui::Context, i: usize) {
        let mut open = true;
        let mut buf = std::mem::take(&mut self.rename_buf);
        let mut commit = false;
        egui::Window::new(miao_term_ui::i18n::t(self.lang, "Rename Tab", "重命名标签"))
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let resp = ui.text_edit_singleline(&mut buf);
                resp.request_focus();
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit = true;
                }
                if ui
                    .button(miao_term_ui::i18n::t(self.lang, "Rename", "重命名"))
                    .clicked()
                {
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

        // Config (ADR: read `~/.config/miaotty/config.toml`).
        let cfg = miao_term_config::Config::load();
        let font_size = cfg.font_size;
        let line_ratio = cfg.line_height;
        let font_family = cfg.font_family.clone();
        let lang = miao_term_ui::i18n::Lang::parse(cfg.language.as_deref());
        let theme = theme_from_config(&cfg.theme, cfg.cursor_style);
        let theme_name = "Nord".to_string();
        let (cw, ch) = State::cell_size(font_size, line_ratio, font_family.as_deref());

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
            theme,
            rules: miao_term_config::view::RuleSet::load(),
            cw,
            ch,
            font_size,
            line_ratio,
            font_family,
            lang,
            mods: ModifiersState::empty(),
            selection: None,
            dragging: false,
            divider_drag: None,
            cursor: (0.0, 0.0),
            show_sidebar: true,
            show_details: true,
            renaming: None,
            rename_buf: String::new(),
            theme_name,
            show_palette: false,
            palette_query: String::new(),
            show_settings: false,
            editor: None,
            open_path: String::new(),
            show_open: false,
            recipe_dialog: None,
            recipe_name: String::new(),
            recipe_list: Vec::new(),
            ssh_dialog: None,
            remote_dialog: None,
            details_tab: 0,
            git_cache: None,
            files_cache: None,
            ports_cache: None,
            prompts: Vec::new(),
            prompt_input: String::new(),
            last_title: None,
            focused: false,
            cursor_on: true,
            last_blink: Instant::now(),
            egui_ctx,
            egui_state,
            egui_renderer,
        };
        if !state.restore_session() {
            state.new_tab();
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
                state.save_session();
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
                        let inner = card_inner(*r);
                        let col = ((px - inner.x * scale) / cw).floor().max(0.0) as u16;
                        let row = ((py - inner.y * scale) / ch).floor().max(0.0) as u16;
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
                        let (cw, ch) = State::cell_size(
                            state.font_size,
                            state.line_ratio,
                            state.font_family.as_deref(),
                        );
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
                    let active = state
                        .tabs
                        .get(state.active_tab)
                        .and_then(|t| t.panes.iter().find(|p| p.id == t.active));
                    let mods = input::Modifiers {
                        ctrl: state.mods.control_key(),
                        alt: state.mods.alt_key(),
                        shift: state.mods.shift_key(),
                        sup: state.mods.super_key(),
                    };
                    let opts = input::EncodeOpts {
                        app_cursor: active
                            .map(|p| p.term.screen().application_cursor())
                            .unwrap_or(false),
                        bracketed: active
                            .map(|p| p.term.screen().bracketed_paste())
                            .unwrap_or(false),
                        kitty: active
                            .map(|p| p.term.screen().kitty_disambiguate())
                            .unwrap_or(false),
                        has_selection: state.selection.is_some(),
                    };
                    let kind = winit_key_kind(&event);
                    // Special keys are encoded below; only plain text keys (letters,
                    // space, symbols) carry a `text` payload to send here — otherwise
                    // Enter/Tab/etc. would be sent twice.
                    let special = !matches!(kind, input::KeyKind::Char(_) | input::KeyKind::Other);
                    let mut bytes = Vec::new();
                    if !special {
                        if let Some(text) = &event.text {
                            if !mods.ctrl && !mods.sup && !text.is_empty() {
                                bytes.extend_from_slice(&input::encode_text(text));
                            }
                        }
                    }
                    bytes.extend_from_slice(&input::encode_key(kind, mods, opts));
                    state.write_input(&bytes);
                }
            }
            WindowEvent::RedrawRequested => state.render(),
            _ => {}
        }
    }
}

fn winit_key_kind(event: &KeyEvent) -> input::KeyKind {
    use input::KeyKind;
    match &event.logical_key {
        Key::Character(s) => s
            .chars()
            .next()
            .map(KeyKind::Char)
            .unwrap_or(KeyKind::Other),
        Key::Named(n) => match n {
            NamedKey::Enter => KeyKind::Enter,
            NamedKey::Backspace => KeyKind::Backspace,
            NamedKey::Tab => KeyKind::Tab,
            NamedKey::Escape => KeyKind::Escape,
            NamedKey::ArrowUp => KeyKind::Up,
            NamedKey::ArrowDown => KeyKind::Down,
            NamedKey::ArrowLeft => KeyKind::Left,
            NamedKey::ArrowRight => KeyKind::Right,
            NamedKey::Home => KeyKind::Home,
            NamedKey::End => KeyKind::End,
            NamedKey::Delete => KeyKind::Delete,
            NamedKey::PageUp => KeyKind::PageUp,
            NamedKey::PageDown => KeyKind::PageDown,
            _ => KeyKind::Other,
        },
        _ => KeyKind::Other,
    }
}

// ---- helpers ---------------------------------------------------------------

fn layout_to_json(l: &Layout) -> serde_json::Value {
    match l {
        Layout::Leaf(id) => serde_json::json!({ "leaf": id }),
        Layout::Split { dir, ratio, a, b } => serde_json::json!({
            "dir": match dir {
                SplitDir::Right => "right",
                SplitDir::Down => "down",
            },
            "ratio": ratio,
            "a": layout_to_json(a),
            "b": layout_to_json(b),
        }),
    }
}

fn json_to_layout(
    v: &serde_json::Value,
    map: &std::collections::HashMap<String, String>,
) -> Option<Layout> {
    if let Some(id) = v.get("leaf").and_then(|x| x.as_str()) {
        return Some(Layout::leaf(
            map.get(id).cloned().unwrap_or_else(|| id.to_string()),
        ));
    }
    let dir = match v.get("dir").and_then(|x| x.as_str())? {
        "right" => SplitDir::Right,
        _ => SplitDir::Down,
    };
    let ratio = v.get("ratio").and_then(|x| x.as_f64()).unwrap_or(0.5) as f32;
    let a = json_to_layout(v.get("a")?, map)?;
    let b = json_to_layout(v.get("b")?, map)?;
    Some(Layout::Split {
        dir,
        ratio,
        a: Box::new(a),
        b: Box::new(b),
    })
}

fn card_inner(r: Rect) -> Rect {
    let card = Rect {
        x: r.x + CARD_MARGIN,
        y: r.y + CARD_MARGIN,
        w: (r.w - CARD_MARGIN * 2.0).max(1.0),
        h: (r.h - CARD_MARGIN * 2.0).max(1.0),
    };
    Rect {
        x: card.x + CARD_PAD,
        y: card.y + CARD_PAD,
        w: (card.w - CARD_PAD * 2.0).max(1.0),
        h: (card.h - CARD_PAD * 2.0).max(1.0),
    }
}

fn lighten(c: miao_term_ui::theme::Rgb, f: f32) -> miao_term_ui::theme::Rgb {
    let l = |v: u8| (v as f32 + (255.0 - v as f32) * f).clamp(0.0, 255.0) as u8;
    miao_term_ui::theme::Rgb(l(c.0), l(c.1), l(c.2))
}

/// A naive Markdown renderer: headings, bullets, quotes and fenced code.
fn markdown_preview(ui: &mut egui::Ui, text: &str) {
    let mut in_code = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            ui.separator();
            continue;
        }
        if in_code {
            ui.label(egui::RichText::new(line).monospace().size(12.0));
            continue;
        }
        let trimmed = line.trim_end();
        if let Some(h) = trimmed.strip_prefix("### ") {
            ui.label(egui::RichText::new(h).size(15.0).strong());
        } else if let Some(h) = trimmed.strip_prefix("## ") {
            ui.label(egui::RichText::new(h).size(18.0).strong());
        } else if let Some(h) = trimmed.strip_prefix("# ") {
            ui.label(egui::RichText::new(h).size(22.0).strong());
        } else if let Some(b) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            ui.label(format!("\u{2022} {b}"));
        } else if let Some(q) = trimmed.strip_prefix("> ") {
            ui.label(
                egui::RichText::new(q)
                    .italics()
                    .color(egui::Color32::from_gray(150)),
            );
        } else if trimmed.is_empty() {
            ui.add_space(6.0);
        } else {
            ui.label(trimmed);
        }
    }
}

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

fn ports_rows(pid: u32) -> Vec<(String, String)> {
    let out = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-a", "-p", &pid.to_string()])
        .output();
    let Ok(out) = out else {
        return vec![("ports".into(), "lsof unavailable".into())];
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut rows = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if let Some(name) = cols.get(8) {
            rows.push((
                cols.first().copied().unwrap_or("").to_string(),
                name.to_string(),
            ));
        }
    }
    if rows.is_empty() {
        rows.push(("ports".into(), "no listeners".into()));
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

fn theme_from_config(t: &miao_term_config::Theme, cursor: miao_term_config::CursorStyle) -> Theme {
    use miao_term_ui::theme::Rgb;
    let rgb = |c: miao_term_config::Rgb| Rgb(c.0, c.1, c.2);
    Theme {
        bg: rgb(t.background),
        fg: rgb(t.foreground),
        palette: t.palette.map(rgb),
        selection: Rgb(0x43, 0x4c, 0x5e),
        cursor: match cursor {
            miao_term_config::CursorStyle::Block => miao_term_ui::CursorStyle::Block,
            miao_term_config::CursorStyle::Bar => miao_term_ui::CursorStyle::Bar,
            miao_term_config::CursorStyle::Underline => miao_term_ui::CursorStyle::Underline,
        },
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

fn recipes_dir() -> Option<std::path::PathBuf> {
    window_file().map(|p| p.with_file_name("recipes"))
}

fn list_recipes() -> Vec<String> {
    let Some(dir) = recipes_dir() else {
        return Vec::new();
    };
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = read
        .flatten()
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        })
        .collect();
    out.sort();
    out
}

fn session_file() -> Option<std::path::PathBuf> {
    window_file().map(|p| p.with_file_name("native-session.json"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_json_round_trip() {
        let mut l = Layout::leaf("a");
        assert!(l.split("a", "b", SplitDir::Right));
        assert!(l.split("b", "c", SplitDir::Down));
        let map: std::collections::HashMap<String, String> = [("a", "x"), ("b", "y"), ("c", "z")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let back = json_to_layout(&layout_to_json(&l), &map).unwrap();
        let mut ids = back.ids();
        ids.sort();
        assert_eq!(ids, vec!["x".to_string(), "y".to_string(), "z".to_string()]);
    }
}
