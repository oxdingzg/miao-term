//! miaotty — a cross-platform terminal (engine: `miao-term-core`).
//!
//! R0/R1 bootstrap UI on eframe/egui: multi-tab, left sidebar, selection +
//! copy/paste, scrollback, wide-char (CJK) layout, cursor blink. The terminal
//! grid is drawn with egui for iteration speed; R1 replaces this with the
//! custom wgpu renderer (`term-render`) per the architecture.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui_wgpu;
use unicode_width::UnicodeWidthStr;

use miao_term_core::vt100;
use miao_term_core::Terminal;

fn main() -> eframe::Result<()> {
    // Start the MTP control plane and export the socket so shells (and the
    // existing `miaotty-cli`) inherit it.
    let socket = miao_term_mtp::default_socket();
    std::env::set_var("MIAOTTY_SOCKET", &socket);
    let state = miao_term_mtp::ServerState::new();
    match miao_term_mtp::serve(&socket, state.clone()) {
        Ok(()) => eprintln!("miaotty: MTP host listening on {}", socket.display()),
        Err(e) => eprintln!("miaotty: failed to start MTP host: {e}"),
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 660.0])
            .with_title("miaotty"),
        ..Default::default()
    };
    eframe::run_native(
        "miaotty",
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                let renderer = miao_term_render::TermRenderer::new(
                    &rs.device,
                    &rs.queue,
                    rs.target_format,
                );
                rs.renderer.write().callback_resources.insert(renderer);
            }
            Ok(Box::new(MiaottyApp::new(state)))
        }),
    )
}

/// Register a system CJK font as a fallback so Chinese/Japanese glyphs render.
fn install_fonts(ctx: &egui::Context) {
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("cjk".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("cjk".to_owned());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
}

/// Active theme colors (from `term-config`).
#[derive(Clone)]
struct Theme {
    bg: egui::Color32,
    fg: egui::Color32,
    palette: [egui::Color32; 16],
}

impl Theme {
    fn from_config(theme: &miao_term_config::Theme) -> Self {
        let c = |r: miao_term_config::Rgb| egui::Color32::from_rgb(r.0, r.1, r.2);
        let mut palette = [egui::Color32::BLACK; 16];
        for (i, p) in theme.palette.iter().enumerate() {
            palette[i] = c(*p);
        }
        Self {
            bg: c(theme.background),
            fg: c(theme.foreground),
            palette,
        }
    }
}

const SELECTION: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x81, 0xa1, 0xc1, 0x55);

fn map_color(color: vt100::Color, foreground: bool, theme: &Theme) -> egui::Color32 {
    match color {
        vt100::Color::Default => {
            if foreground {
                theme.fg
            } else {
                theme.bg
            }
        }
        vt100::Color::Idx(i) => theme.palette[(i as usize) & 0x0f],
        vt100::Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
    }
}

/// How two panes in a tab are arranged.
#[derive(Clone, Copy, PartialEq)]
enum SplitDir {
    Right,
    Down,
}

struct Pane {
    term: Terminal,
    pane_id: String,
    /// Cached row runs for the GPU renderer; rebuilt only when `dirty`.
    rows: Arc<Vec<Vec<miao_term_render::Span>>>,
    dirty: bool,
}

/// Binary split tree inside a tab (leaves are pane ids).
enum Layout {
    Leaf(String),
    Split {
        dir: SplitDir,
        ratio: f32,
        a: Box<Layout>,
        b: Box<Layout>,
    },
}

fn layout_rects(layout: &Layout, rect: egui::Rect, out: &mut Vec<(String, egui::Rect)>) {
    match layout {
        Layout::Leaf(id) => out.push((id.clone(), rect)),
        Layout::Split { dir, ratio, a, b } => {
            let r = ratio.clamp(0.1, 0.9);
            match dir {
                SplitDir::Right => {
                    let w = rect.width() * r;
                    layout_rects(a, egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + w, rect.max.y)), out);
                    layout_rects(b, egui::Rect::from_min_max(egui::pos2(rect.left() + w, rect.top()), rect.max), out);
                }
                SplitDir::Down => {
                    let h = rect.height() * r;
                    layout_rects(a, egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.top() + h)), out);
                    layout_rects(b, egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + h), rect.max), out);
                }
            }
        }
    }
}

fn split_leaf(layout: &mut Layout, target: &str, new_id: &str, dir: SplitDir) -> bool {
    match layout {
        Layout::Leaf(id) if id == target => {
            let old = id.clone();
            *layout = Layout::Split {
                dir,
                ratio: 0.5,
                a: Box::new(Layout::Leaf(old)),
                b: Box::new(Layout::Leaf(new_id.to_string())),
            };
            true
        }
        Layout::Leaf(_) => false,
        Layout::Split { a, b, .. } => {
            split_leaf(a, target, new_id, dir) || split_leaf(b, target, new_id, dir)
        }
    }
}

/// Remove a leaf, collapsing its parent; `None` if the tree was just that leaf.
fn remove_leaf(layout: Layout, target: &str) -> Option<Layout> {
    match layout {
        Layout::Leaf(id) => (id != target).then_some(Layout::Leaf(id)),
        Layout::Split { dir, ratio, a, b } => {
            match (remove_leaf(*a, target), remove_leaf(*b, target)) {
                (Some(a), Some(b)) => Some(Layout::Split {
                    dir,
                    ratio,
                    a: Box::new(a),
                    b: Box::new(b),
                }),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            }
        }
    }
}

struct Tab {
    panes: Vec<Pane>,
    layout: Layout,
    /// Focused pane id within the tab.
    active: String,
    title: String,
}

impl Tab {
    fn focused(&self) -> Option<&Pane> {
        self.panes.iter().find(|p| p.pane_id == self.active)
    }
}

fn gen_pane_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:016x}{:04x}", nanos, COUNTER.fetch_add(1, Ordering::SeqCst))
}

/// A grid selection, in viewport (row, col) coordinates; end may be before start.
#[derive(Clone, Copy)]
struct Selection {
    start: (u16, u16),
    end: (u16, u16),
}

struct MiaottyApp {
    tabs: Vec<Tab>,
    active: usize,
    font_size: f32,
    selection: Option<Selection>,
    scroll: usize,
    cursor_on: bool,
    last_blink: Instant,
    show_details: bool,
    state: Arc<miao_term_mtp::ServerState>,
    theme: Theme,
    metrics: miao_term_render::MetricsProbe,
    renaming: Option<usize>,
    rename_buf: String,
}

impl MiaottyApp {
    fn new(state: Arc<miao_term_mtp::ServerState>) -> Self {
        let cfg = miao_term_config::Config::load();
        let mut app = Self {
            tabs: Vec::new(),
            active: 0,
            font_size: cfg.font_size,
            selection: None,
            scroll: 0,
            cursor_on: true,
            last_blink: Instant::now(),
            show_details: true,
            state,
            theme: Theme::from_config(&cfg.theme),
            metrics: miao_term_render::MetricsProbe::new(),
            renaming: None,
            rename_buf: String::new(),
        };
        app.push_tab("shell".to_owned(), 100, 30, None);
        app
    }

    fn spawn_terminal(
        cols: u16,
        rows: u16,
        pane_id: &str,
        cwd: Option<PathBuf>,
    ) -> Option<Terminal> {
        let env = vec![("MIAOTTY_PANE_ID".to_owned(), pane_id.to_owned())];
        match Terminal::new(None, cols, rows, 10_000, cwd, &env) {
            Ok(term) => Some(term),
            Err(e) => {
                eprintln!("failed to spawn shell: {e}");
                None
            }
        }
    }

    fn push_tab(&mut self, title: String, cols: u16, rows: u16, cwd: Option<PathBuf>) {
        let pane_id = gen_pane_id();
        if let Some(term) = Self::spawn_terminal(cols, rows, &pane_id, cwd) {
            self.tabs.push(Tab {
                panes: vec![Pane {
                    term,
                    rows: Arc::new(Vec::new()),
                    dirty: true,
                    pane_id: pane_id.clone(),
                }],
                layout: Layout::Leaf(pane_id.clone()),
                active: pane_id,
                title,
            });
            self.active = self.tabs.len() - 1;
            self.selection = None;
            self.scroll = 0;
            self.publish_panes();
        }
    }

    fn new_tab(&mut self) {
        let tab = &self.tabs[self.active];
        let (rows, cols) = tab.focused().map(|p| p.term.size()).unwrap_or((30, 100));
        let cwd = tab.focused().and_then(|p| p.term.cwd().map(PathBuf::from));
        let n = self.tabs.len() + 1;
        self.push_tab(format!("shell {n}"), cols, rows, cwd);
    }

    fn duplicate_tab(&mut self, i: usize) {
        let (rows, cols) = self.tabs[i].focused().map(|p| p.term.size()).unwrap_or((30, 100));
        let cwd = self.tabs[i].focused().and_then(|p| p.term.cwd().map(PathBuf::from));
        let title = self.tabs[i].title.clone();
        self.push_tab(title, cols, rows, cwd);
    }

    /// Split the focused pane in the active tab and focus the new pane.
    fn split_active(&mut self, dir: SplitDir) {
        let tab = &mut self.tabs[self.active];
        if tab.panes.len() >= 8 {
            return;
        }
        let Some((rows, cols, cwd)) = tab.focused().map(|p| {
            let (r, c) = p.term.size();
            (r, c, p.term.cwd().map(PathBuf::from))
        }) else {
            return;
        };
        let pane_id = gen_pane_id();
        if let Some(term) = Self::spawn_terminal(cols, rows, &pane_id, cwd) {
            let target = tab.active.clone();
            tab.panes.push(Pane {
                term,
                rows: Arc::new(Vec::new()),
                dirty: true,
                pane_id: pane_id.clone(),
            });
            split_leaf(&mut tab.layout, &target, &pane_id, dir);
            tab.active = pane_id;
            self.selection = None;
            self.scroll = 0;
            self.publish_panes();
        }
    }

    fn close_other_tabs(&mut self, i: usize) {
        let keep = self.tabs.remove(i);
        self.tabs.clear();
        self.tabs.push(keep);
        self.active = 0;
        self.selection = None;
        self.scroll = 0;
        self.publish_panes();
    }

    fn close_active(&mut self) {
        {
            let tab = &mut self.tabs[self.active];
            if tab.panes.len() > 1 {
                let active = tab.active.clone();
                tab.panes.retain(|p| p.pane_id != active);
                let layout = std::mem::replace(&mut tab.layout, Layout::Leaf(String::new()));
                if let Some(new_layout) = remove_leaf(layout, &active) {
                    tab.layout = new_layout;
                }
                tab.active = tab.panes[0].pane_id.clone();
                self.selection = None;
                self.scroll = 0;
                self.publish_panes();
                return;
            }
        }
        if self.tabs.len() <= 1 {
            return;
        }
        self.tabs.remove(self.active);
        self.active = self.active.min(self.tabs.len() - 1);
        self.publish_panes();
    }

    fn rename_window(&mut self, ctx: &egui::Context, i: usize) {
        let mut open = true;
        egui::Window::new("Rename Tab")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let resp = ui.text_edit_singleline(&mut self.rename_buf);
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter || ui.button("OK").clicked() {
                    if let Some(tab) = self.tabs.get_mut(i) {
                        tab.title = self.rename_buf.clone();
                    }
                    self.renaming = None;
                    self.publish_panes();
                }
            });
        if !open {
            self.renaming = None;
        }
    }

    /// Advertise the current tabs as MTP panes.
    fn publish_panes(&self) {
        let mut panes = Vec::new();
        for tab in &self.tabs {
            for (i, pane) in tab.panes.iter().enumerate() {
                let title = if tab.panes.len() > 1 {
                    format!("{} [{}]", tab.title, i + 1)
                } else {
                    tab.title.clone()
                };
                let mut value = serde_json::json!({ "id": pane.pane_id, "title": title });
                if let Some(cwd) = pane.term.cwd() {
                    value["cwd"] = serde_json::json!(cwd);
                }
                panes.push(value);
            }
        }
        self.state.set_panes(panes);
    }
}

impl eframe::App for MiaottyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain output for every tab (keeps channels from growing unbounded).
        let mut changed = false;
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                if pane.term.process_pending() {
                    pane.dirty = true;
                    changed = true;
                }
            }
        }
        if changed {
            // New output jumps to the bottom.
            self.scroll = 0;
            ctx.request_repaint();
        }

        // MTP `pane.send` / `pane.run` — inject bytes into the target pane.
        let writes = self.state.take_writes();
        if !writes.is_empty() {
            for (pane_id, data) in writes {
                'outer: for tab in &mut self.tabs {
                    for pane in &mut tab.panes {
                        if pane.pane_id == pane_id {
                            pane.term.write(&data);
                            break 'outer;
                        }
                    }
                }
            }
            ctx.request_repaint();
        }

        // Cursor blink (only when focused; keeps idle CPU low otherwise).
        if ctx.input(|i| i.focused) {
            if self.last_blink.elapsed() >= Duration::from_millis(530) {
                self.cursor_on = !self.cursor_on;
                self.last_blink = Instant::now();
                // The block cursor inverts a cell, so the row cache must rebuild.
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    let id = tab.active.clone();
                    if let Some(pane) = tab.panes.iter_mut().find(|p| p.pane_id == id) {
                        pane.dirty = true;
                    }
                }
                ctx.request_repaint();
            }
            ctx.request_repaint_after(Duration::from_millis(530));
        }

        self.sidebar(ctx);
        if self.show_details {
            self.details_panel(ctx);
        }
        self.terminal_panel(ctx);
        if let Some(i) = self.renaming {
            self.rename_window(ctx, i);
        }
    }
}

fn section(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .size(11.0)
        .strong()
        .color(egui::Color32::from_gray(150))
}

fn reveal_in_finder(path: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{path}"))
            .spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

impl MiaottyApp {
    fn sidebar(&mut self, ctx: &egui::Context) {
        let rows: Vec<(String, Option<egui::Color32>)> = self
            .tabs
            .iter()
            .map(|tab| {
                let title = tab
                    .focused()
                    .and_then(|p| p.term.cwd())
                    .and_then(|p| std::path::Path::new(p).file_name())
                    .map(|s| s.to_string_lossy().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| tab.title.clone());
                let color = tab
                    .focused()
                    .and_then(|p| self.state.agent_for(&p.pane_id))
                    .and_then(|a| {
                    a.get("state").and_then(|v| v.as_str()).map(|s| match s {
                        "processing" => egui::Color32::from_rgb(0x81, 0xa1, 0xc1),
                        "idle" => egui::Color32::from_rgb(0xa3, 0xbe, 0x8c),
                        "awaiting" => egui::Color32::from_rgb(0xeb, 0xcb, 0x8b),
                        "error" => egui::Color32::from_rgb(0xbf, 0x61, 0x6a),
                        _ => egui::Color32::GRAY,
                    })
                });
                (title, color)
            })
            .collect();
        let active = self.active;
        let fg = self.theme.fg;
        let can_close = self.tabs.len() > 1;
        let mut switch_to: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut close_others: Option<usize> = None;
        let mut duplicate: Option<usize> = None;
        let mut rename: Option<usize> = None;
        let mut rename_buf = String::new();
        let mut add = false;
        let mut toggle = false;

        egui::SidePanel::left("tabs")
            .resizable(true)
            .default_width(190.0)
            .frame(egui::Frame::default().fill(self.theme.bg).inner_margin(egui::Margin::same(6.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(section("TABS"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("+").on_hover_text("New Tab").clicked() {
                            add = true;
                        }
                        if ui.button("\u{25a4}").on_hover_text("Toggle Details").clicked() {
                            toggle = true;
                        }
                    });
                });
                ui.separator();
                for (i, (title, color)) in rows.iter().enumerate() {
                    ui.horizontal(|ui| {
                        match color {
                            Some(c) => {
                                ui.colored_label(*c, "\u{25cf}");
                            }
                            None => {
                                ui.label("  ");
                            }
                        }
                        let label = egui::RichText::new(title).size(13.0).color(fg);
                        let resp = ui.selectable_label(i == active, label);
                        if resp.clicked() {
                            switch_to = Some(i);
                        }
                        if resp.clicked_by(egui::PointerButton::Middle) && can_close {
                            close = Some(i);
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Rename\u{2026}").clicked() {
                                rename = Some(i);
                                rename_buf = title.clone();
                                ui.close_menu();
                            }
                            if ui.button("Duplicate").clicked() {
                                duplicate = Some(i);
                                ui.close_menu();
                            }
                            if ui.button("New Tab").clicked() {
                                add = true;
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Close Tab").clicked() {
                                close = Some(i);
                                ui.close_menu();
                            }
                            if ui.button("Close Other Tabs").clicked() {
                                close_others = Some(i);
                                ui.close_menu();
                            }
                        });
                    });
                }
            });

        if add {
            self.new_tab();
        }
        if toggle {
            self.show_details = !self.show_details;
        }
        if let Some(i) = close {
            self.active = i;
            self.close_active();
        } else if let Some(i) = close_others {
            self.close_other_tabs(i);
        } else if let Some(i) = duplicate {
            self.duplicate_tab(i);
        } else if let Some(i) = switch_to {
            self.active = i;
            self.selection = None;
            self.scroll = 0;
        }
        if let Some(i) = rename {
            self.renaming = Some(i);
            self.rename_buf = rename_buf;
        }
    }

    fn details_panel(&mut self, ctx: &egui::Context) {
        let cwd = self.tabs[self.active]
            .focused()
            .and_then(|p| p.term.cwd().map(str::to_string));
        let pane_id = self.tabs[self.active]
            .focused()
            .map(|p| p.pane_id.clone())
            .unwrap_or_default();
        let history = self.state.history_for(&pane_id);
        let agent = self.state.agent_for(&pane_id);
        let muted = egui::Color32::from_gray(120);
        let fg = self.theme.fg;
        egui::SidePanel::right("details")
            .resizable(true)
            .default_width(300.0)
            .frame(
                egui::Frame::default()
                    .fill(self.theme.bg)
                    .inner_margin(egui::Margin::same(10.0)),
            )
            .show(ctx, |ui| {
                ui.label(section("INFO"));
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Working Directory").strong().color(fg));
                match &cwd {
                    Some(p) => {
                        ui.label(egui::RichText::new(p).monospace().color(fg));
                    }
                    None => {
                        ui.label(egui::RichText::new("\u{2014}").color(muted));
                    }
                }
                if let Some(p) = &cwd {
                    ui.horizontal(|ui| {
                        if ui.button("Copy Path").clicked() {
                            ctx.copy_text(p.clone());
                        }
                        if ui.button("Reveal in Finder").clicked() {
                            reveal_in_finder(p);
                        }
                    });
                }

                if let Some(a) = &agent {
                    ui.add_space(10.0);
                    ui.label(section("AGENT"));
                    let st = a.get("state").and_then(|v| v.as_str()).unwrap_or("?");
                    let name = a.get("agent").and_then(|v| v.as_str()).unwrap_or("agent");
                    ui.label(egui::RichText::new(format!("{name} \u{00b7} {st}")).color(fg));
                }

                ui.add_space(10.0);
                ui.label(section(&format!("OUTLINE ({})", history.len())));
                ui.add_space(4.0);
                if history.is_empty() {
                    ui.label(egui::RichText::new("No commands yet").color(muted));
                } else {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for entry in history.iter().rev() {
                                if let Some(c) = entry.get("cwd").and_then(|v| v.as_str()) {
                                    ui.label(
                                        egui::RichText::new(c).monospace().size(10.0).color(muted),
                                    );
                                }
                                let cmd = entry
                                    .get("command")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                ui.label(egui::RichText::new(cmd).monospace().color(fg));
                                ui.add_space(2.0);
                            }
                        });
                }
            });
    }

    fn terminal_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(self.theme.bg))
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                // Cell size comes from the renderer's own font so glyphs line up.
                let line_height = (self.font_size * 1.25).round();
                let (cw, ch) = self.metrics.cell(self.font_size, line_height);
                if cw <= 0.0 || ch <= 0.0 {
                    return;
                }

                // Window-level shortcuts (⌘ on macOS).
                ctx.input(|i| {
                    for ev in &i.events {
                        if let egui::Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } = ev
                        {
                            if modifiers.mac_cmd {
                                match (*key, modifiers.shift, modifiers.alt) {
                                    (egui::Key::T, _, _) => self.new_tab(),
                                    (egui::Key::W, _, _) => self.close_active(),
                                    (egui::Key::D, true, _) => self.split_active(SplitDir::Down),
                                    (egui::Key::D, false, true) => {
                                        self.show_details = !self.show_details
                                    }
                                    (egui::Key::D, false, false) => {
                                        self.split_active(SplitDir::Right)
                                    }
                                    (egui::Key::Plus, _, _) | (egui::Key::Equals, _, _) => {
                                        self.font_size += 1.0
                                    }
                                    (egui::Key::Minus, _, _) => {
                                        self.font_size = (self.font_size - 1.0).max(6.0)
                                    }
                                    (egui::Key::Num0, _, _) => self.font_size = 14.0,
                                    _ => {}
                                }
                            }
                        }
                    }
                });

                // Pane rectangles for the active tab (from the split tree).
                let ti = self.active;
                let mut rects: Vec<(String, egui::Rect)> = Vec::new();
                layout_rects(&self.tabs[ti].layout, rect, &mut rects);
                for (id, pane_rect) in rects {
                    if let Some(pi) = self.tabs[ti].panes.iter().position(|p| p.pane_id == id) {
                        self.draw_pane(ui, ctx, ti, pi, pane_rect, cw, ch);
                    }
                }
            });
    }

    /// Draw and handle one pane at `rect`.
    #[allow(clippy::too_many_arguments)]
    fn draw_pane(
        &mut self,
        ui: &egui::Ui,
        ctx: &egui::Context,
        ti: usize,
        pi: usize,
        rect: egui::Rect,
        cw: f32,
        ch: f32,
    ) {
        let cols = ((rect.width() / cw).floor() as i64).clamp(1, 1000) as u16;
        let rows = ((rect.height() / ch).floor() as i64).clamp(1, 1000) as u16;
        let pane_id = self.tabs[ti].panes[pi].pane_id.clone();
        let focused = self.tabs[ti].active == pane_id;

        {
            let pane = &mut self.tabs[ti].panes[pi];
            let before = pane.term.size();
            pane.term.resize(rows, cols);
            if pane.term.size() != before {
                pane.dirty = true;
            }
        }

        let response = ui.interact(
            rect,
            ui.id().with(("pane", ti, pi)),
            egui::Sense::click_and_drag(),
        );
        if response.clicked() && !focused {
            self.tabs[ti].active = pane_id.clone();
            self.selection = None;
            self.scroll = 0;
        }

        // Scroll (only the focused pane): wheel + Shift+PgUp/PgDn.
        let mut scroll_changed = false;
        if focused {
            let delta = ui.input(|i| i.raw_scroll_delta.y);
            if delta != 0.0 {
                self.scroll = if delta > 0.0 {
                    (self.scroll + 3).min(200_000)
                } else {
                    self.scroll.saturating_sub(3)
                };
                scroll_changed = true;
            }
            let keys = ctx.input(|i| {
                let mut d: i64 = 0;
                for ev in &i.events {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = ev
                    {
                        if modifiers.shift && !modifiers.ctrl && !modifiers.mac_cmd {
                            match key {
                                egui::Key::PageUp => d += rows as i64 - 1,
                                egui::Key::PageDown => d -= rows as i64 - 1,
                                _ => {}
                            }
                        }
                    }
                }
                d
            });
            if keys > 0 {
                self.scroll = (self.scroll + keys as usize).min(200_000);
                scroll_changed = true;
            } else if keys < 0 {
                self.scroll = self.scroll.saturating_sub((-keys) as usize);
                scroll_changed = true;
            }
        }
        let applied_scroll = if focused { self.scroll } else { 0 };
        {
            let pane = &mut self.tabs[ti].panes[pi];
            pane.term.screen_mut().set_scrollback(applied_scroll);
            if scroll_changed {
                pane.dirty = true;
            }
        }

        if focused {
            let ptr = response.interact_pointer_pos();
            let cell_at = |p: egui::Pos2| -> (u16, u16) {
                let col =
                    (((p.x - rect.left()) / cw).floor() as i64).clamp(0, cols as i64 - 1) as u16;
                let row =
                    (((p.y - rect.top()) / ch).floor() as i64).clamp(0, rows as i64 - 1) as u16;
                (row, col)
            };
            if response.drag_started() {
                if let Some(p) = ptr {
                    let c = cell_at(p);
                    self.selection = Some(Selection { start: c, end: c });
                }
            } else if response.dragged() {
                if let (Some(p), Some(sel)) = (ptr, self.selection.as_mut()) {
                    sel.end = cell_at(p);
                }
            } else if response.double_clicked() {
                if let Some(p) = ptr {
                    let (r, c) = cell_at(p);
                    let sel = word_selection(self.tabs[ti].panes[pi].term.screen(), r, c, cols);
                    self.selection = Some(sel);
                }
            } else if response.clicked() {
                self.selection = None;
            }

            let (app_cursor, bracketed) = {
                let screen = self.tabs[ti].panes[pi].term.screen();
                (screen.application_cursor(), screen.bracketed_paste())
            };
            let mut out = Vec::new();
            let has_selection = self.selection.is_some();
            ctx.input(|i| {
                for ev in &i.events {
                    encode_input(ev, app_cursor, bracketed, has_selection, &mut out);
                }
            });
            if !out.is_empty() {
                self.tabs[ti].panes[pi].term.write(&out);
                ctx.request_repaint();
            }

            if ctx.input(|i| i.modifiers.command) {
                if let Some(sel) = self.selection {
                    let copied = ctx.input(|i| {
                        i.events.iter().any(|ev| {
                            matches!(
                                ev,
                                egui::Event::Key {
                                    key: egui::Key::C,
                                    pressed: true,
                                    modifiers,
                                    ..
                                } if modifiers.command
                            )
                        })
                    });
                    if copied {
                        let (r1, c1, r2, c2) = ordered(sel);
                    let text = self.tabs[ti].panes[pi]
                        .term
                        .screen()
                        .contents_between(r1, c1, r2, c2);
                        if !text.is_empty() {
                            ctx.copy_text(text);
                        }
                    }
                }
            }
        }

        // Draw backgrounds / selection / cursor, then the glyphs via the GPU.
        let draw_cursor;
        {
            let pane = &mut self.tabs[ti].panes[pi];
            let screen = pane.term.screen();
            draw_cursor = focused && self.scroll == 0 && self.cursor_on && !screen.hide_cursor();
            draw_screen(
                ui,
                screen,
                cw,
                ch,
                rect,
                if focused { self.selection } else { None },
                &self.theme,
                draw_cursor,
            );
            let cursor_cell = if draw_cursor {
                Some(screen.cursor_position())
            } else {
                None
            };
            if pane.dirty {
                pane.rows = Arc::new(build_rows(screen, &self.theme, cursor_cell));
                pane.dirty = false;
            }
        }
        let rows = Arc::clone(&self.tabs[ti].panes[pi].rows);
        let scale = ctx.pixels_per_point();
        let fg = self.theme.fg;
        ui.painter().add(egui::Shape::Callback(
            egui_wgpu::Callback::new_paint_callback(
                rect,
                TermCallback {
                    rows,
                    left: rect.left(),
                    top: rect.top(),
                    scale,
                    font_size: self.font_size,
                    line_height: ch,
                    default_color: (fg.r(), fg.g(), fg.b()),
                },
            ),
        ));
    }
}

fn word_selection(screen: &vt100::Screen, row: u16, col: u16, cols: u16) -> Selection {
    let is_word = |c: u16| -> bool {
        screen
            .cell(row, c)
            .map(|cell| {
                let s = cell.contents();
                s.chars()
                    .next()
                    .map(|ch| ch.is_alphanumeric() || "_-./~".contains(ch))
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    };
    if !is_word(col) {
        return Selection {
            start: (row, col),
            end: (row, col),
        };
    }
    let mut l = col;
    while l > 0 && is_word(l - 1) {
        l -= 1;
    }
    let mut r = col;
    while r + 1 < cols && is_word(r + 1) {
        r += 1;
    }
    Selection {
        start: (row, l),
        end: (row, r),
    }
}

fn ordered(sel: Selection) -> (u16, u16, u16, u16) {
    let (sr, sc) = sel.start;
    let (er, ec) = sel.end;
    if (er, ec) < (sr, sc) {
        (er, ec, sr, sc)
    } else {
        (sr, sc, er, ec)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_screen(
    ui: &egui::Ui,
    screen: &vt100::Screen,
    cw: f32,
    ch: f32,
    rect: egui::Rect,
    selection: Option<Selection>,
    theme: &Theme,
    draw_cursor: bool,
) {
    let painter = ui.painter_at(rect);
    let (rows, cols) = screen.size();
    let ox = rect.left();
    let oy = rect.top();

    // Selection highlight (linear range).
    if let Some(sel) = selection {
        let (r1, c1, r2, c2) = ordered(sel);
        let start = r1 as usize * cols as usize + c1 as usize;
        let end = r2 as usize * cols as usize + c2 as usize;
        for i in start..=end {
            let r = (i / cols as usize) as u16;
            let c = (i % cols as usize) as u16;
            if r >= rows || c >= cols {
                continue;
            }
            let cell_rect = egui::Rect::from_min_size(
                egui::pos2(ox + c as f32 * cw, oy + r as f32 * ch),
                egui::vec2(cw, ch),
            );
            painter.rect_filled(cell_rect, egui::Rounding::ZERO, SELECTION);
        }
    }

    // Backgrounds (non-default only).
    for row in 0..rows {
        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            let bg = map_color(cell.bgcolor(), false, theme);
            if bg != theme.bg {
                let cell_rect = egui::Rect::from_min_size(
                    egui::pos2(ox + col as f32 * cw, oy + row as f32 * ch),
                    egui::vec2(cw, ch),
                );
                painter.rect_filled(cell_rect, egui::Rounding::ZERO, bg);
            }
        }
    }

    // Block cursor (the glyph on top is drawn in the bg color by `build_rows`).
    if draw_cursor {
        let (crow, ccol) = screen.cursor_position();
        if crow < rows && ccol < cols {
            let cur_rect = egui::Rect::from_min_size(
                egui::pos2(ox + ccol as f32 * cw, oy + crow as f32 * ch),
                egui::vec2(cw, ch),
            );
            painter.rect_filled(cur_rect, egui::Rounding::ZERO, theme.fg);
        }
    }
}

/// Build per-row, per-color runs from the screen for the GPU renderer.
fn build_rows(
    screen: &vt100::Screen,
    theme: &Theme,
    cursor: Option<(u16, u16)>,
) -> Vec<Vec<miao_term_render::Span>> {
    let (rows, cols) = screen.size();
    let mut out = Vec::with_capacity(rows as usize);
    for row in 0..rows {
        let mut spans: Vec<miao_term_render::Span> = Vec::new();
        let mut col = 0u16;
        while col < cols {
            let Some(cell) = screen.cell(row, col) else {
                col += 1;
                continue;
            };
            let contents = cell.contents();
            let width = UnicodeWidthStr::width(contents).max(usize::from(!contents.is_empty())) as u16;
            let text = if contents.is_empty() { " " } else { contents };
            // The cell under a block cursor is drawn in the background color so
            // it reads as inverted against the cursor block.
            let color = if cell.inverse() || cursor == Some((row, col)) {
                let c = theme.bg;
                (c.r(), c.g(), c.b())
            } else {
                let c = map_color(cell.fgcolor(), true, theme);
                (c.r(), c.g(), c.b())
            };
            match spans.last_mut() {
                Some(last) if last.color == color => last.text.push_str(text),
                _ => spans.push(miao_term_render::Span::new(text, color)),
            }
            col += width.max(1);
        }
        out.push(spans);
    }
    out
}

/// egui→wgpu paint callback that draws the terminal glyphs via `term-render`.
struct TermCallback {
    rows: Arc<Vec<Vec<miao_term_render::Span>>>,
    left: f32,
    top: f32,
    scale: f32,
    font_size: f32,
    line_height: f32,
    default_color: (u8, u8, u8),
}

impl egui_wgpu::CallbackTrait for TermCallback {
    fn prepare(
        &self,
        device: &egui_wgpu::wgpu::Device,
        queue: &egui_wgpu::wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut egui_wgpu::wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<egui_wgpu::wgpu::CommandBuffer> {
        if let Some(renderer) = resources.get_mut::<miao_term_render::TermRenderer>() {
            renderer.prepare(
                device,
                queue,
                (screen.size_in_pixels[0], screen.size_in_pixels[1]),
                self.scale,
                self.font_size,
                self.line_height,
                self.left,
                self.top,
                self.default_color,
                self.rows.as_slice(),
            );
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::epaint::PaintCallbackInfo,
        pass: &mut egui_wgpu::wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(renderer) = resources.get::<miao_term_render::TermRenderer>() {
            renderer.render(pass);
        }
    }
}

fn ctrl_byte(key: egui::Key) -> Option<u8> {
    use egui::Key::*;
    Some(match key {
        A => 0x01, B => 0x02, C => 0x03, D => 0x04, E => 0x05, F => 0x06,
        G => 0x07, H => 0x08, I => 0x09, J => 0x0a, K => 0x0b, L => 0x0c,
        M => 0x0d, N => 0x0e, O => 0x0f, P => 0x10, Q => 0x11, R => 0x12,
        S => 0x13, T => 0x14, U => 0x15, V => 0x16, W => 0x17, X => 0x18,
        Y => 0x19, Z => 0x1a,
        _ => return None,
    })
}

fn encode_input(
    ev: &egui::Event,
    app_cursor: bool,
    bracketed: bool,
    has_selection: bool,
    out: &mut Vec<u8>,
) {
    match ev {
        egui::Event::Text(t) => {
            out.extend_from_slice(t.as_bytes());
        }
        egui::Event::Paste(text) => {
            // Normalize newlines to CR; shells expect carriage returns for Enter.
            let body = text.replace('\n', "\r");
            if bracketed {
                out.extend_from_slice(b"\x1b[200~");
                out.extend_from_slice(body.as_bytes());
                out.extend_from_slice(b"\x1b[201~");
            } else {
                out.extend_from_slice(body.as_bytes());
            }
        }
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => {
            // Consumed by the viewport scroller.
            if modifiers.shift && matches!(key, egui::Key::PageUp | egui::Key::PageDown) {
                return;
            }
            // Ctrl combos (Unix control bytes). On macOS, Cmd is reserved for copy/paste etc.
            if modifiers.ctrl && !modifiers.mac_cmd {
                if let Some(b) = ctrl_byte(*key) {
                    // Ctrl+C with a selection is a copy (handled by the caller).
                    if *key == egui::Key::C && has_selection {
                        return;
                    }
                    out.push(b);
                    return;
                }
            }
            let mut arrow = |c: u8| {
                if app_cursor {
                    out.extend_from_slice(&[0x1b, b'O', c]);
                } else {
                    out.extend_from_slice(&[0x1b, b'[', c]);
                }
            };
            match key {
                egui::Key::Enter => out.push(b'\r'),
                egui::Key::Backspace => out.push(0x7f),
                egui::Key::Tab => {
                    if modifiers.shift {
                        out.extend_from_slice(b"\x1b[Z");
                    } else {
                        out.push(b'\t');
                    }
                }
                egui::Key::Escape => out.push(0x1b),
                egui::Key::ArrowUp => arrow(b'A'),
                egui::Key::ArrowDown => arrow(b'B'),
                egui::Key::ArrowRight => arrow(b'C'),
                egui::Key::ArrowLeft => arrow(b'D'),
                egui::Key::Home => out.extend_from_slice(b"\x1b[H"),
                egui::Key::End => out.extend_from_slice(b"\x1b[F"),
                egui::Key::Delete => out.extend_from_slice(b"\x1b[3~"),
                egui::Key::PageUp => out.extend_from_slice(b"\x1b[5~"),
                egui::Key::PageDown => out.extend_from_slice(b"\x1b[6~"),
                _ => {}
            }
        }
        _ => {}
    }
}
