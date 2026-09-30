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
use miao_term_render::{ImageInstance, ImageRenderer, Quad, QuadRenderer, Span, TermRenderer};
use miao_term_ui::layout::{Layout, Rect, SplitDir};
use miao_term_ui::{build_rows, chrome, input, theme::Theme, Selection};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};

/// Events posted to the loop: a repaint wake-up, or the global Quick Terminal
/// hotkey (which is the only thing that should toggle the scratch tab).
#[derive(Clone, Copy)]
enum HostEvent {
    Wake,
    Hotkey,
    /// A command from the OS menu bar (macOS, inside an app bundle).
    #[cfg(target_os = "macos")]
    Menu(miao_term_ui::chrome::MenuId),
}
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const MENU_H: f32 = 24.0;
const TAB_H: f32 = 30.0;
const STATUS_H: f32 = 22.0;
const SIDEBAR_W: f32 = 200.0;
const DETAILS_W: f32 = 300.0;
const CARD_MARGIN: f32 = 6.0;
const CARD_RADIUS: f32 = 9.0;
const CARD_PAD: f32 = 8.0;
const BLINK: Duration = Duration::from_millis(530);

/// Whether the app menu belongs in the OS menu bar: macOS, and only from inside
/// an app bundle — that is where the application icon lives that AppKit's about
/// panel wants, and `muda` needs one it can decode (ADR 0031).
fn menu_in_os() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::env::current_exe()
            .map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// The OS menu bar, built from the shared menu table (ADR 0031).
#[cfg(target_os = "macos")]
mod appmenu {
    use muda::accelerator::Accelerator;
    use muda::{Menu, MenuEvent, MenuId as MudaId, MenuItem, PredefinedMenuItem, Submenu};
    use std::str::FromStr;
    use winit::event_loop::EventLoopProxy;

    use super::HostEvent;

    /// Everything created for the menu bar. It must stay alive for the life of
    /// the app: muda's native items keep a raw pointer to the Rust-side
    /// `MenuChild`, so dropping an item leaves the click handler reading freed
    /// memory (`CFString cannot be created from a negative number of bytes`,
    /// SIGTRAP). See ADR 0031.
    pub struct MenuHandle {
        _menu: Menu,
        _items: Vec<MenuItem>,
        _submenus: Vec<Submenu>,
    }

    /// Build and install the application menu.
    pub fn install(
        lang: miao_term_ui::i18n::Lang,
        proxy: EventLoopProxy<HostEvent>,
    ) -> Option<MenuHandle> {
        let mut items: Vec<MenuItem> = Vec::new();
        let mut submenus: Vec<Submenu> = Vec::new();
        let menu = Menu::new();
        // AppKit treats the first submenu as the application menu.
        let app_sub = Submenu::new("miaotty", true);
        let _ = app_sub.append(&PredefinedMenuItem::about(None, None));
        let _ = app_sub.append(&PredefinedMenuItem::separator());
        let _ = app_sub.append(&PredefinedMenuItem::services(None));
        let _ = app_sub.append(&PredefinedMenuItem::separator());
        let _ = app_sub.append(&PredefinedMenuItem::hide(None));
        let _ = app_sub.append(&PredefinedMenuItem::hide_others(None));
        let _ = app_sub.append(&PredefinedMenuItem::show_all(None));
        let _ = app_sub.append(&PredefinedMenuItem::separator());
        let quit = MenuItem::with_id(
            MudaId::new(miao_term_ui::menu::key(miao_term_ui::chrome::MenuId::Quit)),
            "Quit miaotty",
            true,
            Accelerator::from_str("CmdOrCtrl+Q").ok(),
        );
        let _ = app_sub.append(&quit);
        items.push(quit);
        if menu.append(&app_sub).is_err() {
            return None;
        }
        submenus.push(app_sub);
        for (title, entries) in miao_term_ui::menu::menus(lang) {
            let sub = Submenu::new(title, true);
            for entry in entries {
                match entry {
                    miao_term_ui::menu::Entry::Item {
                        label,
                        id,
                        shortcut,
                    } => {
                        let acc = shortcut.and_then(|s| Accelerator::from_str(s).ok());
                        let item = MenuItem::with_id(
                            MudaId::new(miao_term_ui::menu::key(id)),
                            label,
                            true,
                            acc,
                        );
                        if sub.append(&item).is_err() {
                            return None;
                        }
                        items.push(item);
                    }
                    miao_term_ui::menu::Entry::Separator => {
                        let _ = sub.append(&PredefinedMenuItem::separator());
                    }
                    miao_term_ui::menu::Entry::Link { label, .. } => {
                        let item =
                            MenuItem::with_id(MudaId::new("documentation"), label, true, None);
                        if sub.append(&item).is_err() {
                            return None;
                        }
                        items.push(item);
                    }
                }
            }
            if menu.append(&sub).is_err() {
                return None;
            }
            submenus.push(sub);
        }
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let key = e.id.0.as_str();
            match miao_term_ui::menu::from_key(key) {
                Some(id) => {
                    let _ = proxy.send_event(HostEvent::Menu(id));
                }
                None => {
                    if key == "documentation" {
                        crate::open_external("https://github.com/oxdingzg/miao-term#readme");
                    }
                }
            }
        }));
        menu.init_for_nsapp();
        Some(MenuHandle {
            _menu: menu,
            _items: items,
            _submenus: submenus,
        })
    }
}

/// Run a native terminal window until it is closed.
pub fn run(title: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Single instance (ADR 0019): a later launch — a deep link or a second
    // `miaotty-native <url>` — is handed to the running instance, which drains
    // its inbox, and this process exits without opening a window.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let intent = miao_term_ui::launch::Intent::from_args(&args);
    if miao_term_ui::launch::forward_to_running(&intent.encode()) {
        eprintln!("miaotty-native: forwarded to the running instance");
        return Ok(());
    }

    // MTP control plane (ADR 0005): the shell inherits `MIAOTTY_SOCKET`, so
    // `miaotty-cli`, plugins and agent hooks work exactly as with the eframe app.
    let socket = miao_term_mtp::default_socket();
    std::env::set_var("MIAOTTY_SOCKET", &socket);
    // MIAOTTY_MTP_TOKEN (if set) requires it on every request; MIAOTTY_MTP_ALLOW
    // (if set) restricts which capabilities are accepted.
    let mtp = miao_term_mtp::ServerState::with_config(
        std::env::var("MIAOTTY_MTP_TOKEN").ok(),
        miao_term_mtp::ServerState::parse_allow(std::env::var("MIAOTTY_MTP_ALLOW").ok()),
    );
    match miao_term_mtp::serve(&socket, mtp.clone()) {
        Ok(()) => eprintln!("miaotty-native: MTP host on {}", socket.display()),
        Err(e) => eprintln!("miaotty-native: MTP host failed: {e}"),
    }
    if let Some(addr) = miao_term_config::Config::load().remote_listen {
        match miao_term_mtp::serve_tcp(&addr, mtp.clone()) {
            Ok(()) => eprintln!("miaotty-native: remote MTP access on {addr}"),
            Err(e) => eprintln!("miaotty-native: remote access disabled: {e}"),
        }
    }

    let event_loop = EventLoop::<HostEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    // Let the control plane wake the loop, so `miaotty-cli` commands apply
    // immediately even while the window is idle or unfocused.
    {
        let proxy = proxy.clone();
        mtp.set_waker(Arc::new(move || {
            let _ = proxy.send_event(HostEvent::Wake);
        }));
    }
    let mut host = Host {
        title: title.to_string(),
        proxy,
        mtp,
        state: None,
        #[cfg(target_os = "macos")]
        menu: None,
    };
    event_loop.run_app(&mut host)?;
    Ok(())
}

struct Host {
    title: String,
    proxy: EventLoopProxy<HostEvent>,
    mtp: Arc<miao_term_mtp::ServerState>,
    state: Option<State>,
    /// The OS menu bar, kept alive for the whole run (ADR 0031).
    #[cfg(target_os = "macos")]
    menu: Option<appmenu::MenuHandle>,
}

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
    /// Opened as an ssh session (shows a server icon).
    ssh: bool,
    /// Optional short prefix shown before the tab title.
    prefix: Option<String>,
    /// A short user marker appended to the tab title (ADR 0011).
    mark: Option<String>,
    /// The session-list group this tab belongs to (ADR 0011).
    group: Option<String>,
}

/// Background-computed details (git status / directory listing / ports).
#[derive(Default, Clone)]
struct DetailsData {
    git: Vec<(String, String)>,
    files: Vec<FileEntry>,
    ports: Vec<(String, String)>,
}

/// One entry in the Files panel.
#[allow(dead_code)]
#[derive(Clone)]
struct FileEntry {
    name: String,
    is_dir: bool,
    size: u64,
}

/// A simple built-in text file editor (with a naive Markdown preview).
struct Editor {
    path: std::path::PathBuf,
    text: String,
    original: String,
    preview: bool,
    /// Opened via MTP `app.view`: shown without an editable text field.
    readonly: bool,
    /// `(destination, remote path)` when editing a file over ssh.
    remote: Option<(String, String)>,
}

/// The inline-image layer for the debug capture (quads + the pane scissor).
struct ImageLayer<'a> {
    quads: &'a [(u64, i32, u32, ImageInstance)],
    rects: &'a [(String, Rect)],
    scale: f32,
}

struct PaneDraw {
    id: String,
    rect: Rect,
    quads: Vec<Quad>,
    rows: Vec<Vec<Span>>,
}

struct State {
    window: Arc<Window>,
    proxy: EventLoopProxy<HostEvent>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    quads: QuadRenderer,
    images: ImageRenderer,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    pip: Option<Pip>,
    pip_request: bool,
    graphics_enabled: bool,
    renderers: HashMap<String, TermRenderer>,
    mtp: Arc<miao_term_mtp::ServerState>,
    tabs: Vec<Tab>,
    active_tab: usize,
    theme: Theme,
    rules: miao_term_config::view::RuleSet,
    cw: f32,
    ch: f32,
    font_size: f32,
    default_font_size: f32,
    line_ratio: f32,
    font_family: Option<String>,
    lang: miao_term_ui::i18n::Lang,
    mods: ModifiersState,
    selection: Option<(String, Selection)>,
    dragging: bool,
    divider_drag: Option<(Vec<bool>, SplitDir, Rect)>,
    /// Mouse button currently forwarded to the application (0/1/2), if any.
    mouse_captured: Option<u8>,
    cursor: (f64, f64),
    /// Inline IME composition text (not yet committed to the shell).
    preedit: String,
    show_sidebar: bool,
    show_details: bool,
    renaming: Option<usize>,
    rename_buf: String,
    theme_name: String,
    show_palette: bool,
    palette_query: String,
    palette_idx: usize,
    show_settings: bool,
    editor: Option<Editor>,
    open_path: String,
    show_open: bool,
    recipe_dialog: Option<bool>,
    recipe_name: String,
    recipe_list: Vec<String>,
    ssh_dialog: Option<String>,
    remote_dialog: Option<(String, String)>,
    editor_vim: bool,
    vim: Option<miao_term_ui::vim::VimRuntime>,
    vim_for: String,
    cmark: egui_commonmark::CommonMarkCache,
    mmd: Mmd,
    recent_files: Vec<String>,
    open_counts: HashMap<String, u32>,
    integration_msg: Option<String>,
    read_only: bool,
    hint_mode: bool,
    hints: Vec<miao_term_ui::hints::Hint>,
    tree_expanded: std::collections::HashSet<std::path::PathBuf>,
    tree_children: HashMap<std::path::PathBuf, Vec<FileEntry>>,
    files_filter: String,
    prefix_renaming: Option<usize>,
    prefix_buf: String,
    mark_renaming: Option<usize>,
    mark_buf: String,
    group_renaming: Option<usize>,
    group_buf: String,
    hotkeys: Option<miao_term_ui::hotkey::Hotkeys>,
    opacity: f32,
    notifications: bool,
    prevent_sleep: bool,
    sleep: miao_term_ui::agentloop::SleepGuard,
    agent_states: HashMap<String, String>,
    composer: Option<String>,
    quick: Option<String>,
    closed: Vec<Option<std::path::PathBuf>>,
    /// Scratch "Quick" tab (ADR 0019): its pane id and the tab to return to.
    quick_pane: Option<String>,
    quick_return: Option<usize>,
    hover_pointer: bool,
    search: Option<String>,
    search_idx: usize,
    search_hits: Vec<(usize, u16)>,
    search_key: String,
    update_url: Option<String>,
    update_rx: Option<std::sync::mpsc::Receiver<String>>,
    update_msg: Option<String>,
    details_tab: usize,
    details_cwd: Option<std::path::PathBuf>,
    details_data: Option<DetailsData>,
    details_rx: Option<std::sync::mpsc::Receiver<(std::path::PathBuf, DetailsData)>>,
    details_at: Instant,
    prompts: Vec<String>,
    prompt_input: String,
    last_title: Option<String>,
    focused: bool,
    cursor_on: bool,
    last_blink: Instant,
    start: Instant,
    shot_now: bool,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

/// An always-on-top window mirroring the active pane (read-only).
struct Pip {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    quads: QuadRenderer,
    renderer: TermRenderer,
}

struct Shortcut {
    new_tab: bool,
    close: bool,
    select: Option<usize>,
    font: f32,
    split_right: bool,
    split_down: bool,
    cycle: i32,
    tab: i32,
    find: i32,
    hint: bool,
    toggle_sidebar: bool,
    toggle_details: bool,
    palette: bool,
    settings: bool,
    composer: bool,
    quickly: bool,
    reopen: bool,
    search: bool,
    quick_terminal: bool,
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
        tab: 0,
        find: 0,
        hint: false,
        toggle_sidebar: false,
        toggle_details: false,
        palette: false,
        settings: false,
        composer: false,
        quickly: false,
        reopen: false,
        search: false,
        quick_terminal: false,
    };
    let mut matched = true;
    match &event.logical_key {
        Key::Character(c) => match c.as_str() {
            "t" if mods.shift_key() => s.quick_terminal = true,
            "t" => s.new_tab = true,
            "z" if mods.shift_key() => s.reopen = true,
            "e" => s.composer = true,
            "h" if mods.shift_key() => s.hint = true,
            "g" if mods.shift_key() => s.find = -1,
            "g" => s.find = 1,
            "o" if mods.shift_key() => s.quickly = true,
            "f" => s.search = true,
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
            "]" if mods.shift_key() => s.tab = 1,
            "[" if mods.shift_key() => s.tab = -1,
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

/// A hyperlink resolved under the pointer, with its screen geometry.
struct LinkHit {
    url: String,
    start: u16,
    end: u16,
    row: u16,
    inner: Rect,
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
            let _ = proxy.send_event(HostEvent::Wake);
        });
        let env = vec![("MIAOTTY_PANE_ID".to_string(), id.clone())];
        Terminal::new(None, cols, rows, 10_000, cwd, &env, waker)
            .ok()
            .map(|mut term| {
                term.set_graphics_enabled(self.graphics_enabled);
                let scale = self.window.scale_factor() as f32;
                term.set_cell_size((self.cw * scale) as u16, (self.ch * scale) as u16);
                Pane {
                    id,
                    term,
                    scroll: 0,
                }
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
        self.new_tab_in(None);
    }

    fn new_tab_in(&mut self, cwd: Option<std::path::PathBuf>) {
        let Some(pane) = self.spawn_pane(cwd) else {
            return;
        };
        let id = pane.id.clone();
        let n = self.tabs.len() + 1;
        self.tabs.push(Tab {
            layout: Layout::leaf(id.clone()),
            panes: vec![pane],
            active: id,
            title: format!("shell {n}"),
            ssh: false,
            prefix: None,
            mark: None,
            group: None,
        });
        self.active_tab = self.tabs.len() - 1;
        self.selection = None;
        self.publish_panes();
    }

    /// Reopen the most recently closed tab (Cmd+Shift+T) in its old cwd.
    /// Toggle the scratch "Quick" tab (ADR 0019): the first invocation opens a
    /// tab named *Quick* and remembers where to return; later ones flip between
    /// the Quick tab and that tab.
    fn toggle_quick_terminal(&mut self) {
        if let Some(id) = self.quick_pane.clone() {
            if let Some(ti) = self
                .tabs
                .iter()
                .position(|t| t.panes.iter().any(|p| p.id == id))
            {
                if self.active_tab == ti {
                    if let Some(prev) = self.quick_return.take() {
                        self.active_tab = prev.min(self.tabs.len().saturating_sub(1));
                        self.selection = None;
                    }
                } else {
                    self.quick_return = Some(self.active_tab);
                    self.active_tab = ti;
                    self.selection = None;
                }
                self.window.request_redraw();
                return;
            }
        }
        let prev = self.active_tab;
        self.new_tab();
        if let Some(tab) = self.tabs.last_mut() {
            tab.title = miao_term_ui::i18n::t(self.lang, "Quick", "快速").to_string();
            if let Some(pane) = tab.panes.first() {
                self.quick_pane = Some(pane.id.clone());
            }
        }
        self.quick_return = Some(prev);
        self.publish_panes();
    }

    fn reopen_tab(&mut self) {
        let Some(cwd) = self.closed.pop() else {
            return;
        };
        let Some(pane) = self.spawn_pane(cwd) else {
            return;
        };
        let id = pane.id.clone();
        let n = self.tabs.len() + 1;
        self.tabs.push(Tab {
            layout: Layout::leaf(id.clone()),
            panes: vec![pane],
            active: id,
            title: format!("shell {n}"),
            ssh: false,
            prefix: None,
            mark: None,
            group: None,
        });
        self.active_tab = self.tabs.len() - 1;
        self.selection = None;
        self.publish_panes();
    }

    fn close_pane(&mut self) {
        let cwd = self
            .active_pane()
            .and_then(|p| p.term.cwd().map(std::path::PathBuf::from));
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        let target = tab.active.clone();
        if tab.panes.len() <= 1 {
            // Close the tab.
            if self.tabs.len() > 1 {
                self.closed.push(cwd);
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

    /// The active pane's inner (terminal) rect in logical points.
    fn active_inner(&self) -> Option<Rect> {
        let id = self.active_pane_id()?;
        self.pane_rects()
            .into_iter()
            .find(|(pid, _)| *pid == id)
            .map(|(_, r)| card_inner(r))
    }

    /// The active pane's inner rect and cursor cell, for cursor-anchored
    /// overlays (IME preedit). `None` while the view is scrolled back.
    fn active_cursor(&self) -> Option<(Rect, (u16, u16))> {
        let id = self.active_pane_id()?;
        let pane = self
            .tabs
            .get(self.active_tab)?
            .panes
            .iter()
            .find(|p| p.id == id)?;
        if pane.scroll != 0 {
            return None;
        }
        let inner = self.active_inner()?;
        Some((inner, pane.term.screen().cursor_position()))
    }

    fn cancel_hints(&mut self) {
        self.hint_mode = false;
        self.hints.clear();
    }

    /// Collect Hint-Mode labels over visible URLs / absolute paths.
    fn build_hints(&mut self) {
        self.hints.clear();
        let Some(id) = self.active_pane_id() else {
            return;
        };
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        let Some(pane) = tab.panes.iter().find(|p| p.id == id) else {
            return;
        };
        let screen = pane.term.screen();
        let (rows, _) = screen.size();
        let lines: Vec<String> = (0..rows).map(|r| screen.line_text(r)).collect();
        self.hints = miao_term_ui::hints::scan(&lines);
        self.hint_mode = !self.hints.is_empty();
    }

    /// Lazily load a directory and (transitively) its expanded subdirectories.
    fn load_tree(&mut self, dir: &std::path::Path) {
        if self.tree_children.contains_key(dir) {
            return;
        }
        let entries = files_rows(dir);
        let subdirs: Vec<std::path::PathBuf> = entries
            .iter()
            .filter(|f| f.is_dir)
            .map(|f| dir.join(&f.name))
            .collect();
        self.tree_children.insert(dir.to_path_buf(), entries);
        if self.tree_expanded.contains(dir) {
            for s in subdirs {
                self.load_tree(&s);
            }
        }
    }

    fn files_body(&mut self, ui: &mut egui::Ui, lang: miao_term_ui::i18n::Lang) {
        use miao_term_ui::i18n::t;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.files_filter)
                    .hint_text(t(lang, "Filter…", "过滤…"))
                    .desired_width(150.0),
            );
            if ui
                .small_button("\u{21bb}")
                .on_hover_text(t(lang, "Refresh", "刷新"))
                .clicked()
            {
                self.tree_children.clear();
                self.ensure_details();
            }
        });
        let Some(root) = self.cwd() else {
            ui.label("\u{2014}");
            return;
        };
        self.load_tree(&root);
        let filter = self.files_filter.to_lowercase();
        let mut open_file = None;
        let mut toggle = None;
        render_dir_tree(
            ui,
            &self.tree_children,
            &self.tree_expanded,
            &root,
            0,
            &filter,
            &mut open_file,
            &mut toggle,
        );
        if let Some(d) = toggle {
            if !self.tree_expanded.insert(d.clone()) {
                self.tree_expanded.remove(&d);
            }
            self.load_tree(&d);
        }
        if let Some(f) = open_file {
            self.open_editor(f);
        }
    }

    /// Create the PiP window (called from the event loop when requested).
    fn create_pip(&mut self, event_loop: &ActiveEventLoop) {
        if self.pip.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("miaotty \u{00b7} picture-in-picture")
            .with_inner_size(LogicalSize::new(720.0, 400.0))
            .with_window_level(winit::window::WindowLevel::AlwaysOnTop);
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("miaotty-native: picture-in-picture window failed: {e}");
                return;
            }
        };
        let surface = match self.instance.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("miaotty-native: pip surface failed: {e}");
                return;
            }
        };
        let caps = surface.get_capabilities(&self.adapter);
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
        surface.configure(&self.device, &config);
        let quads = QuadRenderer::new(&self.device, format);
        let renderer = TermRenderer::new(&self.device, &self.queue, format);
        self.pip = Some(Pip {
            window,
            surface,
            config,
            quads,
            renderer,
        });
        self.window.request_redraw();
    }

    fn resize_pip(&mut self) {
        let (device, w, h) = (
            &self.device,
            self.pip.as_ref().map(|p| p.window.inner_size().width),
            self.pip.as_ref().map(|p| p.window.inner_size().height),
        );
        if let (Some(w), Some(h)) = (w, h) {
            if w > 0 && h > 0 {
                if let Some(pip) = self.pip.as_mut() {
                    pip.config.width = w;
                    pip.config.height = h;
                    pip.surface.configure(device, &pip.config);
                }
            }
        }
    }

    /// Render the active pane into the PiP window.
    fn render_pip(&mut self) {
        let Some(pip) = self.pip.as_mut() else {
            return;
        };
        let theme = self.theme.clone();
        let scale = pip.window.scale_factor() as f32;
        let win_size = (pip.config.width, pip.config.height);
        let rows = match self.tabs.get(self.active_tab) {
            Some(tab) => match tab.panes.iter().find(|p| p.id == tab.active) {
                Some(pane) => build_rows(pane.term.screen(), &theme, None),
                None => return,
            },
            None => return,
        };
        let Ok(frame) = pip.surface.get_current_texture() else {
            return;
        };
        let view = frame.texture.create_view(&Default::default());
        let full = Quad::new(
            (0.0, 0.0),
            (win_size.0 as f32, win_size.1 as f32),
            (theme.bg.0, theme.bg.1, theme.bg.2, 255),
        );
        pip.quads
            .prepare(&self.device, &self.queue, win_size, &[full]);
        pip.renderer.prepare(
            &self.device,
            &self.queue,
            win_size,
            scale,
            self.font_size,
            (self.font_size * self.line_ratio).round(),
            self.cw,
            0.0,
            0.0,
            (theme.fg.0, theme.fg.1, theme.fg.2),
            self.font_family.as_deref(),
            &rows,
        );
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pip"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                })
                .forget_lifetime();
            pip.quads.render(&mut pass);
            pip.renderer.render(&mut pass);
        }
        self.queue.submit(Some(enc.finish()));
        frame.present();
    }

    /// Close one pane by id (drops its tab when it was the last one).
    fn close_pane_id(&mut self, id: &str) {
        let Some(ti) = self
            .tabs
            .iter()
            .position(|t| t.panes.iter().any(|p| p.id == id))
        else {
            return;
        };
        if self.quick_pane.as_deref() == Some(id) {
            self.quick_pane = None;
            self.quick_return = None;
        }
        let tab = &mut self.tabs[ti];
        tab.panes.retain(|p| p.id != id);
        let _ = tab.layout.remove(id);
        if tab.panes.is_empty() {
            self.tabs.remove(ti);
        } else if tab.active == id {
            tab.active = tab
                .layout
                .ids()
                .first()
                .cloned()
                .unwrap_or_else(|| tab.panes[0].id.clone());
        }
        if self.tabs.is_empty() {
            self.new_tab();
        }
        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        self.selection = None;
        self.publish_panes();
    }

    /// Close panes whose shell has exited (so `exit` actually closes).
    fn reap_exited(&mut self) {
        let ids: Vec<String> = self
            .tabs
            .iter()
            .flat_map(|t| t.panes.iter())
            .filter(|p| p.term.exited())
            .map(|p| p.id.clone())
            .collect();
        for id in ids {
            self.close_pane_id(&id);
        }
    }

    fn duplicate_tab(&mut self) {
        let cwd = self.cwd();
        let (title, ssh, prefix) = self
            .tabs
            .get(self.active_tab)
            .map(|t| (t.title.clone(), t.ssh, t.prefix.clone()))
            .unwrap_or_default();
        self.new_tab_in(cwd);
        if let Some(t) = self.tabs.last_mut() {
            t.title = title;
            t.ssh = ssh;
            t.prefix = prefix;
        }
        self.publish_panes();
    }

    fn cycle_tab(&mut self, forward: bool) {
        let n = self.tabs.len();
        if n > 1 {
            let i = self.active_tab;
            self.active_tab = if forward {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            };
            self.selection = None;
        }
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
                    pane.term.set_cell_size(cw as u16, ch as u16);
                    pane.term.resize(rows, cols);
                }
            }
        }
    }

    fn write_input(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.read_only {
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

    /// The hyperlink under the pointer in the active pane, if any.
    fn link_at_pointer(&self) -> Option<LinkHit> {
        let scale = self.window.scale_factor() as f32;
        let (px, py) = (self.cursor.0 as f32, self.cursor.1 as f32);
        let (id, r) = self.pane_rects().into_iter().find(|(_, r)| {
            px >= r.x * scale
                && px < (r.x + r.w) * scale
                && py >= r.y * scale
                && py < (r.y + r.h) * scale
        })?;
        let inner = card_inner(r);
        let col = ((px - inner.x * scale) / (self.cw * scale))
            .floor()
            .max(0.0) as u16;
        let row = ((py - inner.y * scale) / (self.ch * scale))
            .floor()
            .max(0.0) as u16;
        let tab = self.tabs.get(self.active_tab)?;
        let pane = tab.panes.iter().find(|p| p.id == id)?;
        let (url, start, end) = link_at(&pane.term.screen().line_text(row), col)?;
        Some(LinkHit {
            url,
            start,
            end,
            row,
            inner,
        })
    }

    /// The currently selected text, if any.
    fn selection_text(&self) -> Option<String> {
        let (pane_id, sel) = self.selection.as_ref()?;
        let tab = self.tabs.get(self.active_tab)?;
        let pane = tab.panes.iter().find(|p| &p.id == pane_id)?;
        let (r1, c1, r2, c2) = sel.ordered();
        Some(pane.term.screen().contents_between(r1, c1, r2, c2))
    }

    /// The selection with SGR colour codes.
    fn selection_ansi(&self) -> Option<String> {
        let (pane_id, sel) = self.selection.as_ref()?;
        let tab = self.tabs.get(self.active_tab)?;
        let pane = tab.panes.iter().find(|p| &p.id == pane_id)?;
        let (r1, c1, r2, c2) = sel.ordered();
        Some(pane.term.screen().contents_ansi_between(r1, c1, r2, c2))
    }

    fn copy_selection(&self, ctx: &egui::Context) {
        if let Some(text) = self.selection_text() {
            if !text.is_empty() {
                ctx.copy_text(text);
            }
        }
    }

    fn find_in_all_tabs(&mut self) {
        let q = self
            .search
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| self.selection_text().map(|s| s.trim().to_string()))
            .unwrap_or_default();
        if q.is_empty() {
            return;
        }
        let ql = q.to_lowercase();
        for (ti, tab) in self.tabs.iter().enumerate() {
            let hit = tab.panes.iter().any(|pane| {
                let screen = pane.term.screen();
                (0..screen.total_lines())
                    .any(|b| screen.line_text_abs(b).to_lowercase().contains(&ql))
            });
            if hit {
                self.active_tab = ti;
                self.selection = None;
                self.search = Some(q);
                self.search_idx = 0;
                self.search_key.clear();
                self.refresh_search();
                self.scroll_to_search_hit();
                return;
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

    /// Forward a mouse event to the pane under the pointer when the running
    /// application enabled mouse reporting. Returns true when it was consumed.
    ///
    /// `button`: 0 left, 1 middle, 2 right, 64 wheel-up, 65 wheel-down.
    /// `motion` marks a drag/hover report (bit 5 set, no press/release).
    fn forward_mouse(&mut self, px: f32, py: f32, button: u8, pressed: bool, motion: bool) -> bool {
        let scale = self.window.scale_factor() as f32;
        let Some((id, r)) = self.pane_rects().into_iter().find(|(_, r)| {
            px >= r.x * scale
                && px < (r.x + r.w) * scale
                && py >= r.y * scale
                && py < (r.y + r.h) * scale
        }) else {
            return false;
        };
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let Some(pane) = tab.panes.iter_mut().find(|p| p.id == id) else {
            return false;
        };
        let Some((clicks, motion_mode, drag_mode, sgr)) = pane.term.screen().mouse_reporting()
        else {
            return false;
        };
        let wheel = button >= 64;
        if motion && !(motion_mode || drag_mode) {
            return false;
        }
        if !motion && !clicks && !wheel {
            return false;
        }
        let cw = self.cw * scale;
        let ch = self.ch * scale;
        let inner = card_inner(r);
        let col = ((px - inner.x * scale) / cw).floor().max(0.0) as u16 + 1;
        let row = ((py - inner.y * scale) / ch).floor().max(0.0) as u16 + 1;
        let Some(seq) = mouse_report(sgr, button, pressed, motion, col, row) else {
            return false;
        };
        pane.term.write(seq.as_bytes());
        self.window.request_redraw();
        true
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
        self.agent_loop();
        self.refresh_search();
        self.poll_details();
        self.ensure_details();
        // MTP `pane.send` / `pane.run` — inject bytes into the target pane.
        for (pane_id, data) in self.mtp.take_writes() {
            for tab in &mut self.tabs {
                if let Some(p) = tab.panes.iter_mut().find(|p| p.id == pane_id) {
                    p.term.write(&data);
                    p.scroll = 0;
                }
            }
        }
        // MTP `pane.focus` / `pane.close` / `app.view` / `app.edit`.
        for command in self.mtp.take_commands() {
            match command {
                miao_term_mtp::Command::Focus(id) => {
                    let found = self
                        .tabs
                        .iter()
                        .position(|t| t.panes.iter().any(|p| p.id == id));
                    if let Some(ti) = found {
                        self.tabs[ti].active = id.clone();
                        self.active_tab = ti;
                        self.selection = None;
                        for p in self.tabs[ti].panes.iter_mut() {
                            p.scroll = 0;
                        }
                    }
                }
                miao_term_mtp::Command::Close(id) => self.close_pane_id(&id),
                miao_term_mtp::Command::View(path) => {
                    self.open_editor_ro(std::path::PathBuf::from(path), true);
                }
                miao_term_mtp::Command::Edit(path) => {
                    self.open_editor_ro(std::path::PathBuf::from(path), false);
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
        self.reap_exited();

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
        let panel_bg = theme.chrome().bg;
        let window_bg = panel_bg;
        let selection = self.selection.clone();
        let rects = self.pane_rects();
        let active_id = self.active_pane_id().unwrap_or_default();
        let search_on = self.search.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
        let search_hits = self.search_hits.clone();
        let search_idx = self.search_idx;
        let search_len = self
            .search
            .as_ref()
            .map(|s| s.chars().count() as u16)
            .unwrap_or(0);
        let mut draws: Vec<PaneDraw> = Vec::new();
        let mut image_quads: Vec<(u64, i32, u32, ImageInstance)> = Vec::new();
        let mut image_uploads: Vec<(u64, Arc<miao_term_core::graphics::PlacedImage>, usize)> =
            Vec::new();
        let mut image_keep: std::collections::HashSet<u64> = std::collections::HashSet::new();
        let mut animating = false;
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            for (pane_idx, (id, r)) in rects.iter().enumerate() {
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
                let bg = panel_bg;
                let radius = CARD_RADIUS * scale;
                quads.push(Quad::rounded(
                    (card.x * scale, card.y * scale),
                    ((card.x + card.w) * scale, (card.y + card.h) * scale),
                    (bg.0, bg.1, bg.2, 255),
                    radius,
                ));
                for row in 0..sr {
                    for col in 0..sc {
                        let Some(cell) = pane.term.screen().cell(row, col) else {
                            continue;
                        };
                        let bg = theme.color(cell.bg, false);
                        if bg != theme.bg && bg != panel_bg {
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
                if search_on && id == &active_id {
                    let hist = pane.term.screen().history_size() as i32;
                    let off = pane.term.screen().scroll_offset() as i32;
                    for (k, (b, col)) in search_hits.iter().enumerate() {
                        let row = *b as i32 - hist + off;
                        if row < 0 || row >= sr as i32 {
                            continue;
                        }
                        let color = if k == search_idx {
                            (0x2e, 0x5b, 0x8f)
                        } else {
                            (0x33, 0x3d, 0x4d)
                        };
                        for dc in 0..search_len {
                            quads.push(quad(ox, oy, row as u16, col + dc, cw, ch, color));
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
                // Inline images (engine layer), drawn under the glyphs.
                if self.graphics_enabled {
                    let off = pane.term.screen().scroll_offset() as i32;
                    let px1 = ox + inner.w * scale;
                    let py1 = oy + inner.h * scale;
                    for im in pane.term.graphics().images.iter() {
                        let fi = if im.animating && im.frames.len() > 1 {
                            animating = true;
                            (im.anim_start.elapsed().as_millis() / 100) as usize % im.frames.len()
                        } else {
                            0
                        };
                        let frame = &im.frames[fi.min(im.frames.len().saturating_sub(1))];
                        let key = miao_term_core::graphics::image_key(id, im.id, fi);
                        image_keep.insert(key);
                        image_uploads.push((key, Arc::new(im.clone()), fi));
                        let w = frame.width as f32;
                        let h = frame.height as f32;
                        let bx = ox + im.col as f32 * cw;
                        let by = oy + (im.anchor + off) as f32 * ch;
                        let (x0, y0, x1, y1) = if let (Some(c), Some(r)) = (im.cols, im.rows) {
                            // Explicit cell footprint: fit inside it, centred.
                            let (tw, th) = (c as f32 * cw, r as f32 * ch);
                            let s = (tw / w).min(th / h);
                            let (dw, dh) = (w * s, h * s);
                            let (x, y) = (bx + (tw - dw) / 2.0, by + (th - dh) / 2.0);
                            (x, y, x + dw, y + dh)
                        } else {
                            let (x, y) = (bx + im.x_off as f32, by + im.y_off as f32);
                            (x, y, x + w, y + h)
                        };
                        if x1 < ox || y1 < oy || x0 > px1 || y0 > py1 {
                            continue;
                        }
                        image_quads.push((
                            key,
                            im.z,
                            pane_idx as u32,
                            ImageInstance {
                                min: [x0, y0],
                                max: [x1, y1],
                                uv_min: [0.0, 0.0],
                                uv_max: [1.0, 1.0],
                            },
                        ));
                    }
                }
                draws.push(PaneDraw {
                    id: id.clone(),
                    rect: inner,
                    quads,
                    rows: rows_data,
                });
            }
        }

        // Split dividers between panes (Otty's 1px `[divider]` token).
        if let Some(tab) = self.tabs.get(self.active_tab) {
            let border = theme.chrome().hover;
            let mut divider_quads = Vec::new();
            for h in tab.layout.handles(self.grid_area()) {
                let (x0, y0, x1, y1) = match h.dir {
                    SplitDir::Right => {
                        let cx = h.rect.x + h.rect.w / 2.0;
                        (cx - 0.5, h.rect.y, cx + 0.5, h.rect.y + h.rect.h)
                    }
                    SplitDir::Down => {
                        let cy = h.rect.y + h.rect.h / 2.0;
                        (h.rect.x, cy - 0.5, h.rect.x + h.rect.w, cy + 0.5)
                    }
                };
                divider_quads.push(Quad::new(
                    (x0 * scale, y0 * scale),
                    (x1 * scale, y1 * scale),
                    (border.0, border.1, border.2, 255),
                ));
            }
            if let Some(last) = draws.last_mut() {
                last.quads.extend(divider_quads);
            }
        }

        // Upload any new inline images, drop textures for images that are gone.
        for (key, p, fi) in &image_uploads {
            if !self.images.has(*key) {
                if let Some(frame) = p.frames.get(*fi) {
                    self.images.upload(
                        &self.device,
                        &self.queue,
                        *key,
                        frame.width,
                        frame.height,
                        &frame.rgba,
                    );
                }
            }
        }
        if animating {
            self.window.request_redraw();
        }
        self.images.retain(&image_keep);

        // GPU: quads (all panes) then per-pane glyphs.
        let all_quads: Vec<Quad> = draws.iter().flat_map(|d| d.quads.iter().copied()).collect();
        self.quads
            .prepare(&self.device, &self.queue, self.window_size(), &all_quads);
        self.images
            .prepare(&self.device, &self.queue, self.window_size(), &image_quads);
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
        // egui asks for an immediate repaint when a widget changed (e.g. a
        // clicked tab). Honour it, otherwise the new state only shows on the
        // next OS event or the cursor-blink tick — which reads as lag.
        let repaint_now = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay.is_zero())
            .unwrap_or(false);
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
        if self.shot_now || std::env::var_os("MIAOTTY_NATIVE_SHOT").is_some() {
            self.capture(
                &draws,
                ImageLayer {
                    quads: &image_quads,
                    rects: &rects,
                    scale,
                },
                window_bg,
                &paint_jobs,
                &screen,
            );
        }

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
                            load: wgpu::LoadOp::Clear(linear_color(window_bg, self.opacity)),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                })
                .forget_lifetime();
            self.quads.render(&mut pass);
            if !image_quads.is_empty() {
                // Clip each pane's images to that pane (they are grouped by pane).
                for (pane_idx, d) in draws.iter().enumerate() {
                    let sx = (d.rect.x * scale).max(0.0) as u32;
                    let sy = (d.rect.y * scale).max(0.0) as u32;
                    let sw = ((d.rect.w * scale) as u32).min(self.config.width.saturating_sub(sx));
                    let sh = ((d.rect.h * scale) as u32).min(self.config.height.saturating_sub(sy));
                    if sw == 0 || sh == 0 {
                        continue;
                    }
                    pass.set_scissor_rect(sx, sy, sw, sh);
                    self.images.render(&mut pass, pane_idx as u32);
                }
                pass.set_scissor_rect(0, 0, self.config.width, self.config.height);
            }
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
        if repaint_now {
            self.window.request_redraw();
        }
        self.render_pip();
    }

    fn chrome(&mut self, ctx: &egui::Context) {
        // Shared, host-agnostic chrome (menu/tabs/sidebar/details/status).
        chrome::render(ctx, self);
        // Hyperlink hover cue: hand cursor + underline while Cmd/Ctrl is held.
        let link = if self.mods.super_key() || self.mods.control_key() {
            self.link_at_pointer()
        } else {
            None
        };
        let want = link.is_some();
        if want != self.hover_pointer {
            self.hover_pointer = want;
            self.window.set_cursor(if want {
                winit::window::CursorIcon::Pointer
            } else {
                winit::window::CursorIcon::Default
            });
        }
        if self.hint_mode && !self.hints.is_empty() {
            if let Some(inner) = self.active_inner() {
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("hints"),
                ));
                for h in &self.hints {
                    let x = inner.x + h.col as f32 * self.cw;
                    let y = inner.y + h.row as f32 * self.ch;
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(x, y),
                        egui::vec2(self.cw * 1.6, self.ch),
                    );
                    painter.rect_filled(
                        rect,
                        egui::Rounding::same(3.0),
                        egui::Color32::from_rgb(0xeb, 0xcb, 0x8b),
                    );
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        &h.label,
                        egui::FontId::monospace(11.0),
                        egui::Color32::BLACK,
                    );
                }
            }
        }
        if !self.preedit.is_empty() {
            if let Some((inner, (row, col))) = self.active_cursor() {
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("ime_preedit"),
                ));
                let ch = self.theme.chrome();
                let col_of = |c: miao_term_ui::theme::Rgb| egui::Color32::from_rgb(c.0, c.1, c.2);
                let galley = painter.layout_no_wrap(
                    self.preedit.clone(),
                    egui::FontId::proportional(14.0),
                    col_of(ch.text),
                );
                let pos = egui::pos2(
                    inner.x + col as f32 * self.cw,
                    inner.y + row as f32 * self.ch,
                );
                let rect = egui::Rect::from_min_size(
                    pos,
                    egui::vec2(galley.size().x.max(self.cw), self.ch.max(galley.size().y)),
                );
                painter.rect_filled(rect, egui::Rounding::same(2.0), col_of(ch.active));
                painter.galley(pos, galley, col_of(ch.text));
                painter.line_segment(
                    [
                        egui::pos2(rect.min.x, rect.max.y),
                        egui::pos2(rect.max.x, rect.max.y),
                    ],
                    egui::Stroke::new(1.0_f32, col_of(ch.accent)),
                );
            }
        }
        if let Some(h) = link {
            let y = h.inner.y + (h.row as f32 + 1.0) * self.ch - 1.5;
            let x0 = h.inner.x + h.start as f32 * self.cw;
            let x1 = h.inner.x + (h.end as f32 + 1.0) * self.cw;
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("link_underline"),
            ));
            painter.line_segment(
                [egui::pos2(x0, y), egui::pos2(x1, y)],
                egui::Stroke::new(1.0_f32, chrome::fg_color(&self.theme)),
            );
        }
        // Host-specific overlay windows.
        self.palette_window(ctx);
        self.settings_window(ctx);
        self.open_dialog_window(ctx);
        self.editor_window(ctx);
        self.recipe_dialog_window(ctx);
        self.ssh_dialog_window(ctx);
        self.remote_dialog_window(ctx);
        self.composer_window(ctx);
        self.quick_window(ctx);
        self.search_window(ctx);
        if let Some(i) = self.renaming {
            self.rename_window(ctx, i);
        }
        if let Some(i) = self.mark_renaming {
            let mut buf = std::mem::take(&mut self.mark_buf);
            let mut slot = self.mark_renaming;
            self.tab_text_window(ctx, i, "Tab Mark", "标签标记", &mut buf, &mut slot, false);
            self.mark_buf = buf;
            self.mark_renaming = slot;
        }
        if let Some(i) = self.group_renaming {
            let mut buf = std::mem::take(&mut self.group_buf);
            let mut slot = self.group_renaming;
            self.tab_text_window(ctx, i, "Tab Group", "标签分组", &mut buf, &mut slot, true);
            self.group_buf = buf;
            self.group_renaming = slot;
        }
        if let Some(i) = self.prefix_renaming {
            self.prefix_window(ctx, i);
        }
    }
    fn details_content(&self, tab: usize) -> (&'static str, Vec<(String, String)>) {
        match tab {
            1 => ("Agent", self.agent_rows()),
            2 => ("Outline", self.outline_rows()),
            3 => (
                "Git",
                self.details_data
                    .as_ref()
                    .map(|d| d.git.clone())
                    .unwrap_or_default(),
            ),
            4 => ("Files", Vec::new()),
            5 => (
                "Ports",
                self.details_data
                    .as_ref()
                    .map(|d| d.ports.clone())
                    .unwrap_or_default(),
            ),
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
                "prefix": tab.prefix,
                "mark": tab.mark,
                "group": tab.group,
                "ssh": tab.ssh,
            }));
        }
        serde_json::json!({
            "active_tab": self.active_tab,
            "tabs": tabs,
            "recent": self.recent_files,
            "counts": self.open_counts,
        })
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

    /// Persist the prompt queue so it survives a restart.
    fn save_queue(&self) {
        if let Some(path) = queue_file() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let body = serde_json::json!({ "prompts": self.prompts });
            let _ = std::fs::write(path, serde_json::to_vec(&body).unwrap_or_default());
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
            let prefix = t.get("prefix").and_then(|x| x.as_str()).map(str::to_string);
            let ssh = t.get("ssh").and_then(|x| x.as_bool()).unwrap_or(false);
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
                ssh,
                prefix,
                mark: t.get("mark").and_then(|m| m.as_str()).map(str::to_string),
                group: t.get("group").and_then(|g| g.as_str()).map(str::to_string),
            });
        }
        if self.tabs.is_empty() {
            return false;
        }
        self.recent_files = v
            .get("recent")
            .and_then(|r| r.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        self.open_counts =
            serde_json::from_value(v.get("counts").cloned().unwrap_or(serde_json::Value::Null))
                .unwrap_or_default();
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

    /// Kick off a background refresh of git/files/ports for the active cwd.
    fn ensure_details(&mut self) {
        let Some(cwd) = self.cwd() else {
            return;
        };
        let fresh = self.details_cwd.as_deref() == Some(cwd.as_path())
            && self.details_at.elapsed() < Duration::from_secs(2);
        if fresh || self.details_rx.is_some() {
            return;
        }
        let pid = self.active_pane().and_then(|p| p.term.pid());
        let proxy = self.proxy.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let cwd2 = cwd.clone();
        std::thread::spawn(move || {
            let data = DetailsData {
                git: git_rows(&cwd2),
                files: files_rows(&cwd2),
                ports: pid.map(ports_rows).unwrap_or_default(),
            };
            let _ = tx.send((cwd2, data));
            let _ = proxy.send_event(HostEvent::Wake);
        });
        self.details_rx = Some(rx);
    }

    fn poll_details(&mut self) {
        let Some(rx) = self.details_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok((cwd, data)) => {
                self.details_cwd = Some(cwd);
                self.details_data = Some(data);
                self.details_at = Instant::now();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => self.details_rx = Some(rx),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
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
        use miao_term_ui::i18n::t;
        let l = self.lang;
        let mut s = self
            .cwd()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "miaotty".to_string());
        // Git branch (when the details worker has it).
        if let Some(branch) = self.details_data.as_ref().and_then(|d| {
            d.git
                .iter()
                .find(|(k, _)| k == "branch")
                .map(|(_, v)| v.clone())
        }) {
            let b = branch.split_whitespace().next().unwrap_or(&branch);
            if !b.is_empty() {
                s.push_str("   ");
                s.push_str(t(l, "branch", "分支"));
                s.push(' ');
                s.push_str(b);
            }
        }
        let panes = self
            .tabs
            .get(self.active_tab)
            .map(|t| t.panes.len())
            .unwrap_or(0);
        if panes > 1 {
            s.push_str(&format!("   {} {panes}", t(l, "panes", "分屏")));
        }
        if self.read_only {
            s.push_str("   RO");
        }
        if let Some(m) = &self.update_msg {
            s.push_str("   \u{00b7}   ");
            s.push_str(m);
        }
        s
    }
}

/// A palette command.
#[derive(Clone, Copy)]
enum Cmd {
    NewTab,
    Composer,
    OpenQuickly,
    CheckUpdates,
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
    FontReset,
    Palette,
    CopyAnsi,
    PasteEscaped,
    Find,
    FindNext,
    FindPrev,
    UseSelForFind,
    JumpToSel,
    FindInAllTabs,
    Fullscreen,
    ReadOnly,
    HintMode,
    Pip,
    ClearScreen,
    ClearScrollback,
    DuplicateTab,
    ReopenClosed,
    QuickTerminal,
    SelectAll,
    CopyPath,
    RevealCwd,
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
        use miao_term_ui::i18n::t;
        let l = self.lang;
        vec![
            (Cmd::NewTab, t(l, "New Tab", "新建标签")),
            (Cmd::Composer, "Composer"),
            (Cmd::OpenQuickly, t(l, "Open Quickly", "快速打开")),
            (Cmd::QuickTerminal, t(l, "Quick Terminal", "快速终端")),
            (Cmd::CheckUpdates, t(l, "Check for Updates", "检查更新")),
            (Cmd::NewSsh, t(l, "New SSH Session…", "新建 SSH 会话…")),
            (Cmd::OpenRemote, t(l, "Open Remote File…", "打开远端文件…")),
            (Cmd::SaveRecipe, t(l, "Save Recipe…", "保存配方…")),
            (Cmd::OpenRecipe, t(l, "Open Recipe…", "打开配方…")),
            (Cmd::OpenFile, t(l, "Open File…", "打开文件…")),
            (Cmd::Save, t(l, "Save", "保存")),
            (Cmd::Copy, t(l, "Copy", "复制")),
            (Cmd::Paste, t(l, "Paste", "粘贴")),
            (Cmd::SplitRight, t(l, "Split Right", "向右分屏")),
            (Cmd::SplitDown, t(l, "Split Down", "向下分屏")),
            (Cmd::ClosePane, t(l, "Close Pane / Tab", "关闭 Pane/标签")),
            (Cmd::ToggleSidebar, t(l, "Toggle Sidebar", "开关侧栏")),
            (Cmd::ToggleDetails, t(l, "Toggle Details", "开关详情")),
            (Cmd::FontUp, t(l, "Increase Font Size", "增大字号")),
            (Cmd::FontDown, t(l, "Decrease Font Size", "减小字号")),
            (Cmd::FontReset, t(l, "Reset Font Size", "重置字号")),
            (Cmd::Palette, t(l, "Command Palette", "命令面板")),
            (
                Cmd::CopyAnsi,
                t(l, "Copy as ANSI Sequence", "复制为 ANSI 序列"),
            ),
            (
                Cmd::PasteEscaped,
                t(l, "Paste Escaping Special Characters", "转义粘贴"),
            ),
            (Cmd::Find, t(l, "Find…", "查找…")),
            (Cmd::FindNext, t(l, "Find Next", "查找下一个")),
            (Cmd::FindPrev, t(l, "Find Previous", "查找上一个")),
            (
                Cmd::UseSelForFind,
                t(l, "Use Selection for Find", "用所选内容查找"),
            ),
            (Cmd::JumpToSel, t(l, "Jump to Selection", "跳到所选")),
            (
                Cmd::FindInAllTabs,
                t(l, "Find in All Tabs", "在所有标签中查找"),
            ),
            (Cmd::Fullscreen, t(l, "Toggle Full Screen", "全屏切换")),
            (Cmd::ReadOnly, t(l, "Read Only", "只读")),
            (
                Cmd::HintMode,
                t(l, "Open Link (Hint Mode)", "打开链接（提示模式）"),
            ),
            (Cmd::ClearScreen, t(l, "Clear Screen", "清屏")),
            (Cmd::ClearScrollback, t(l, "Clear Scrollback", "清除回滚")),
            (Cmd::DuplicateTab, t(l, "Duplicate Tab", "复制标签")),
            (
                Cmd::ReopenClosed,
                t(l, "Reopen Last Closed", "重开最近关闭"),
            ),
            (Cmd::SelectAll, t(l, "Select All", "全选")),
            (Cmd::CopyPath, t(l, "Copy Path", "复制路径")),
            (
                Cmd::RevealCwd,
                t(l, "Reveal in File Manager", "在文件管理器中显示"),
            ),
            (Cmd::Settings, t(l, "Settings", "设置")),
            (Cmd::Quit, t(l, "Quit", "退出")),
        ]
    }

    fn run_command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::NewTab => self.new_tab(),
            Cmd::QuickTerminal => self.toggle_quick_terminal(),
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
            Cmd::FontReset => {
                self.font_size = self.default_font_size;
                let (cw, ch) =
                    State::cell_size(self.font_size, self.line_ratio, self.font_family.as_deref());
                self.cw = cw;
                self.ch = ch;
                self.resize();
            }
            Cmd::Palette => {
                self.show_palette = true;
                self.palette_query.clear();
                self.palette_idx = 0;
            }
            Cmd::Find => {
                self.search = Some(String::new());
                self.search_idx = 0;
                self.search_key.clear();
            }
            Cmd::CopyAnsi => {
                if let Some(t) = self.selection_ansi() {
                    if !t.is_empty() {
                        self.egui_ctx.copy_text(t);
                    }
                }
            }
            Cmd::PasteEscaped => {
                let text = self.egui_state.clipboard_text().unwrap_or_default();
                if !text.is_empty() {
                    let escaped = shell_escape_text(&text);
                    self.paste(&escaped);
                }
            }
            Cmd::FindNext | Cmd::FindPrev => {
                let step = if matches!(cmd, Cmd::FindNext) { 1 } else { -1 };
                let n = self.search_hits.len();
                if self.search.is_some() && n > 0 {
                    self.search_idx =
                        ((self.search_idx as i32 + step).rem_euclid(n as i32)) as usize;
                    self.scroll_to_search_hit();
                }
            }
            Cmd::UseSelForFind => {
                if let Some(s) = self.selection_text() {
                    let s = s.trim().to_string();
                    if !s.is_empty() {
                        self.search = Some(s);
                        self.search_idx = 0;
                        self.search_key.clear();
                    }
                }
            }
            Cmd::JumpToSel => {
                if self.search.is_some() {
                    self.scroll_to_search_hit();
                }
            }
            Cmd::FindInAllTabs => self.find_in_all_tabs(),
            Cmd::ReadOnly => self.read_only = !self.read_only,
            Cmd::HintMode => self.build_hints(),
            Cmd::Pip => {
                if self.pip.is_some() {
                    self.pip = None;
                } else {
                    self.pip_request = true;
                }
            }
            Cmd::Fullscreen => {
                let full = self.window.fullscreen().is_some();
                self.window.set_fullscreen(if full {
                    None
                } else {
                    Some(winit::window::Fullscreen::Borderless(None))
                });
            }
            Cmd::ClearScrollback => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    let active = tab.active.clone();
                    if let Some(p) = tab.panes.iter_mut().find(|p| p.id == active) {
                        p.term.screen_mut().process(b"\x1b[3J");
                    }
                }
            }
            Cmd::DuplicateTab => self.duplicate_tab(),
            Cmd::ReopenClosed => self.reopen_tab(),
            Cmd::SelectAll => {
                if let Some(id) = self.active_pane_id() {
                    if let Some(tab) = self.tabs.get(self.active_tab) {
                        if let Some(p) = tab.panes.iter().find(|p| p.id == id) {
                            let (r, c) = p.term.size();
                            self.selection = Some((
                                id,
                                Selection {
                                    start: (0, 0),
                                    end: (r.saturating_sub(1), c.saturating_sub(1)),
                                },
                            ));
                        }
                    }
                }
            }
            Cmd::ClearScreen => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    let active = tab.active.clone();
                    if let Some(p) = tab.panes.iter_mut().find(|p| p.id == active) {
                        p.term.screen_mut().process(b"\x1b[2J\x1b[H");
                        p.scroll = 0;
                    }
                }
            }
            Cmd::CopyPath => {
                if let Some(cwd) = self.cwd() {
                    self.egui_ctx.copy_text(cwd.display().to_string());
                }
            }
            Cmd::RevealCwd => {
                if let Some(cwd) = self.cwd() {
                    open_external(&cwd.display().to_string());
                }
            }
            Cmd::Composer => self.composer = Some(String::new()),
            Cmd::OpenQuickly => self.quick = Some(String::new()),
            Cmd::CheckUpdates => self.check_updates(),
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
        let mut chosen: Option<Cmd> = None;
        let mut open = true;
        let mut close = false;
        let ch = self.theme.chrome();
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
            ui.visuals_mut().selection.bg_fill = miao_term_ui::chrome::bg_color(ch.active);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .hint_text(miao_term_ui::i18n::t(
                        self.lang,
                        "Type a command…",
                        "输入命令…",
                    ))
                    .desired_width(420.0),
            );
            resp.request_focus();
            let q = query.to_lowercase();
            let mut rows: Vec<(usize, Cmd, &'static str)> = cmds
                .iter()
                .filter_map(|(c, l)| {
                    miao_term_ui::palette::score(l, "command", &q).map(|s| (s, *c, *l))
                })
                .collect();
            rows.sort_by_key(|(s, _, _)| *s);
            let down = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
            let up = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
            let esc = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            if down {
                self.palette_idx = (self.palette_idx + 1).min(rows.len().saturating_sub(1));
            }
            if up {
                self.palette_idx = self.palette_idx.saturating_sub(1);
            }
            if rows.is_empty() {
                self.palette_idx = 0;
            } else {
                self.palette_idx = self.palette_idx.min(rows.len() - 1);
            }
            for (i, (_, _, label)) in rows.iter().enumerate() {
                if ui.selectable_label(i == self.palette_idx, *label).clicked() {
                    chosen = Some(rows[i].1);
                }
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                chosen = rows.get(self.palette_idx).map(|(_, c, _)| *c);
            }
            if esc {
                close = true;
            }
        });
        self.palette_query = query;
        if let Some(cmd) = chosen {
            self.run_command(cmd);
            self.show_palette = false;
            self.palette_query.clear();
        }
        if !open || close {
            self.show_palette = false;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let mut font = self.font_size;
        let mut family = self.font_family.clone().unwrap_or_default();
        let mut line_ratio = self.line_ratio;
        let mut opacity = self.opacity;
        let mut cursor = self.theme.cursor;
        let mut graphics = self.graphics_enabled;
        let mut notifications = self.notifications;
        let mut prevent_sleep = self.prevent_sleep;
        let mut install_agent: Option<&'static str> = None;
        let current_theme = self.theme_name.clone();
        let mut chosen_theme: Option<&'static str> = None;
        egui::Window::new(miao_term_ui::i18n::t(self.lang, "Settings", "设置"))
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(560.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label(miao_term_ui::i18n::t(self.lang, "Font size", "字号"));
                        ui.add(egui::Slider::new(&mut font, 6.0..=40.0));
                        ui.horizontal(|ui| {
                            ui.label(miao_term_ui::i18n::t(self.lang, "Font family", "字体"));
                            ui.add(
                                egui::TextEdit::singleline(&mut family)
                                    .hint_text(miao_term_ui::i18n::t(
                                        self.lang,
                                        "system default",
                                        "系统默认",
                                    ))
                                    .desired_width(170.0),
                            );
                        });
                        ui.label(miao_term_ui::i18n::t(self.lang, "Line height", "行高"));
                        ui.add(egui::Slider::new(&mut line_ratio, 1.0..=2.0));
                        ui.label(miao_term_ui::i18n::t(self.lang, "Opacity", "不透明度"));
                        ui.add(egui::Slider::new(&mut opacity, 0.1..=1.0));
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
                        ui.checkbox(
                            &mut graphics,
                            miao_term_ui::i18n::t(self.lang, "Inline graphics", "终端内联图片"),
                        );
                        ui.checkbox(
                            &mut notifications,
                            miao_term_ui::i18n::t(self.lang, "Notifications", "通知"),
                        );
                        ui.checkbox(
                            &mut prevent_sleep,
                            miao_term_ui::i18n::t(self.lang, "Prevent sleep", "防休眠"),
                        );
                        ui.separator();
                        ui.label(miao_term_ui::i18n::t(
                            self.lang,
                            "Agent integrations",
                            "Agent 集成",
                        ));
                        for a in miao_term_ui::integration::AGENTS {
                            ui.horizontal(|ui| {
                                ui.label(if miao_term_ui::integration::detected(a.bin) {
                                    "\u{25cf}"
                                } else {
                                    "\u{25cb}"
                                });
                                ui.label(a.name);
                                if ui
                                    .button(miao_term_ui::i18n::t(
                                        self.lang,
                                        "Install hook",
                                        "安装钩子",
                                    ))
                                    .clicked()
                                {
                                    install_agent = Some(a.name);
                                }
                            });
                        }
                        if let Some(msg) = &self.integration_msg {
                            ui.label(
                                egui::RichText::new(msg)
                                    .size(11.0)
                                    .color(egui::Color32::from_gray(150)),
                            );
                        }
                        ui.separator();
                        ui.label(miao_term_ui::i18n::t(self.lang, "Theme", "主题"));
                        for name in Theme::NAMES {
                            if ui.selectable_label(current_theme == name, name).clicked() {
                                chosen_theme = Some(name);
                            }
                        }
                    });
            });
        let family_opt = if family.trim().is_empty() {
            None
        } else {
            Some(family.trim().to_string())
        };
        if (font - self.font_size).abs() > 0.01
            || (line_ratio - self.line_ratio).abs() > 0.001
            || family_opt != self.font_family
        {
            self.font_size = font;
            self.line_ratio = line_ratio;
            self.font_family = family_opt;
            let (cw, ch) =
                State::cell_size(self.font_size, self.line_ratio, self.font_family.as_deref());
            self.cw = cw;
            self.ch = ch;
            self.resize();
        }
        self.opacity = opacity;
        self.theme.cursor = cursor;
        if let Some(name) = install_agent {
            let msg = match miao_term_ui::integration::install(name) {
                Ok(path) => miao_term_ui::integration::AGENTS
                    .iter()
                    .find(|a| a.name == name)
                    .map(|a| miao_term_ui::integration::snippet(a, &path))
                    .unwrap_or_default(),
                Err(e) => format!("install failed: {e}"),
            };
            self.integration_msg = Some(msg);
        }
        self.notifications = notifications;
        if self.prevent_sleep != prevent_sleep {
            self.prevent_sleep = prevent_sleep;
            if !prevent_sleep {
                self.sleep.set_awake(false);
            }
        }
        if graphics != self.graphics_enabled {
            self.graphics_enabled = graphics;
            for tab in &mut self.tabs {
                for pane in &mut tab.panes {
                    pane.term.set_graphics_enabled(graphics);
                }
            }
            self.window.request_redraw();
        }
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

    /// Debug: render the terminal grid offscreen and dump a PPM, then exit.
    fn capture(
        &self,
        draws: &[PaneDraw],
        images: ImageLayer<'_>,
        window_bg: miao_term_ui::theme::Rgb,
        paint_jobs: &[egui::ClippedPrimitive],
        screen: &egui_wgpu::ScreenDescriptor,
    ) {
        let (w, h) = (self.config.width, self.config.height);
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shot"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(linear_color(window_bg, self.opacity)),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                })
                .forget_lifetime();
            self.quads.render(&mut pass);
            if !images.quads.is_empty() {
                let (sx, sy, sw, sh) = grid_scissor(images.rects, images.scale, w, h);
                pass.set_scissor_rect(sx, sy, sw, sh);
                self.images.render_all(&mut pass);
                pass.set_scissor_rect(0, 0, w, h);
            }
            for d in draws {
                if let Some(r) = self.renderers.get(&d.id) {
                    r.render(&mut pass);
                }
            }
            self.egui_renderer.render(&mut pass, paint_jobs, screen);
        }
        let bpr = (w * 4) as usize;
        let padded = (bpr + 255) & !255;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (padded * h as usize) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &buf,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(enc.finish()));
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::Maintain::Wait);
        let data = slice.get_mapped_range();
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        for y in 0..h as usize {
            let row = &data[y * padded..y * padded + bpr];
            for x in 0..w as usize {
                ppm.push(row[x * 4 + 2]);
                ppm.push(row[x * 4 + 1]);
                ppm.push(row[x * 4]);
            }
        }
        drop(data);
        let _ = std::fs::write("/tmp/native_shot.ppm", ppm);
        std::process::exit(0);
    }

    /// Apply a URL-scheme / argv launch intent (ADR 0013).
    fn apply_launch(&mut self, intent: &miao_term_ui::launch::Intent) {
        use miao_term_ui::launch::Intent;
        match intent {
            Intent::Activate => {}
            Intent::Quick => self.toggle_quick_terminal(),
            Intent::Focus(id) => {
                for (i, tab) in self.tabs.iter().enumerate() {
                    if tab.panes.iter().any(|p| &p.id == id) {
                        self.active_tab = i;
                        self.selection = None;
                        break;
                    }
                }
            }
            Intent::Run(cmd) => {
                self.new_tab();
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    let active = tab.active.clone();
                    if let Some(pane) = tab.panes.iter_mut().find(|p| p.id == active) {
                        pane.term.write(format!("{cmd}\r").as_bytes());
                    }
                }
                self.publish_panes();
            }
        }
    }

    /// Notifications + sleep guard (ADR 0010), mirroring the eframe host.
    fn agent_loop(&mut self) {
        if !self.notifications && !self.prevent_sleep {
            return;
        }
        let focused_id = self.active_pane_id();
        let mut states: Vec<(String, String, String)> = Vec::new();
        let mut any_processing = false;
        for tab in &self.tabs {
            for pane in &tab.panes {
                let Some(a) = self.mtp.agent_for(&pane.id) else {
                    continue;
                };
                let state = a
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let agent = a
                    .get("agent")
                    .and_then(|v| v.as_str())
                    .unwrap_or("agent")
                    .to_string();
                if state == "processing" {
                    any_processing = true;
                }
                states.push((pane.id.clone(), agent, state));
            }
        }
        let mut alert: Option<(String, String)> = None;
        for (id, agent, state) in states {
            let prev = self.agent_states.insert(id.clone(), state.clone());
            let changed = prev.as_deref() != Some(state.as_str());
            let wants = matches!(state.as_str(), "awaiting" | "error");
            let focused = Some(&id) == focused_id.as_ref();
            if changed && wants && self.notifications && !self.focused && !focused {
                let body = self
                    .tabs
                    .iter()
                    .find(|t| t.panes.iter().any(|p| p.id == id))
                    .map(|t| self.title_of(t))
                    .unwrap_or_default();
                alert = Some((format!("{agent} \u{00b7} {state}"), body));
            }
        }
        if let Some((title, body)) = alert {
            miao_term_ui::agentloop::notify(&title, &body);
        }
        if self.prevent_sleep {
            self.sleep.set_awake(any_processing);
        }
    }

    fn quick_window(&mut self, ctx: &egui::Context) {
        enum Pick {
            Tab(usize),
            File(String),
            Dir(String),
            Path(std::path::PathBuf),
        }
        let cwd = self.cwd();
        let tabs: Vec<(usize, String)> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| (i, self.title_of(t)))
            .collect();
        let files: Vec<(String, bool)> = self
            .details_data
            .as_ref()
            .map(|d| {
                d.files
                    .iter()
                    .filter(|f| f.is_dir)
                    .map(|f| (f.name.clone(), true))
                    .collect()
            })
            .unwrap_or_default();
        let recents = self.recent_files.clone();
        let counts = self.open_counts.clone();
        let Some(query) = self.quick.as_mut() else {
            return;
        };
        let mut chosen: Option<Pick> = None;
        let mut open = true;
        egui::Window::new(miao_term_ui::i18n::t(self.lang, "Open Quickly", "快速打开"))
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(query)
                        .hint_text(miao_term_ui::i18n::t(
                            self.lang,
                            "tab / file",
                            "标签 / 文件",
                        ))
                        .desired_width(420.0),
                );
                r.request_focus();
                let q = query.to_lowercase();
                let freq = |p: &str| std::cmp::Reverse(*counts.get(p).unwrap_or(&0));
                let mut rows: Vec<(usize, std::cmp::Reverse<u32>, String, Pick)> = Vec::new();
                for (i, title) in &tabs {
                    if let Some(s) = miao_term_ui::palette::score(title, "tab", &q) {
                        rows.push((s, freq(title), format!("\u{21e5} {title}"), Pick::Tab(*i)));
                    }
                }
                for (name, is_dir) in &files {
                    let kind = if *is_dir { "dir" } else { "file" };
                    if let Some(s) = miao_term_ui::palette::score(name, kind, &q) {
                        let icon = if *is_dir { "\u{1f4c1}" } else { " " };
                        let pick = if *is_dir {
                            Pick::Dir(name.clone())
                        } else {
                            Pick::File(name.clone())
                        };
                        let f = cwd
                            .as_ref()
                            .map(|c| c.join(name).to_string_lossy().to_string())
                            .unwrap_or_default();
                        rows.push((s, freq(&f), format!("{icon}  {name}"), pick));
                    }
                }
                for path in &recents {
                    if let Some(s) = miao_term_ui::palette::score(path, "recent", &q) {
                        rows.push((
                            s,
                            freq(path),
                            format!("\u{21ba} {path}"),
                            Pick::Path(std::path::PathBuf::from(path)),
                        ));
                    }
                }
                rows.sort_by_key(|r| (r.0, r.1));
                let clone_pick = |p: &Pick| match p {
                    Pick::Tab(i) => Pick::Tab(*i),
                    Pick::File(n) => Pick::File(n.clone()),
                    Pick::Dir(n) => Pick::Dir(n.clone()),
                    Pick::Path(p) => Pick::Path(p.clone()),
                };
                for (_, _, label, pick) in rows.iter().take(50) {
                    if ui.selectable_label(false, label).clicked() {
                        chosen = Some(clone_pick(pick));
                    }
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if let Some((_, _, _, p)) = rows.first() {
                        chosen = Some(clone_pick(p));
                    }
                }
            });
        if let Some(p) = chosen {
            self.quick = None;
            match p {
                Pick::Tab(i) => {
                    if i < self.tabs.len() {
                        self.active_tab = i;
                        self.selection = None;
                    }
                }
                Pick::File(name) => {
                    if let Some(path) = cwd.map(|c| c.join(&name)) {
                        self.open_editor(path);
                    }
                }
                Pick::Dir(name) => {
                    if let Some(path) = cwd.map(|c| c.join(&name)) {
                        self.new_tab_in(Some(path));
                    }
                }
                Pick::Path(path) => {
                    self.open_editor(path);
                }
            }
        } else if !open {
            self.quick = None;
        }
    }

    fn compute_search_hits(&self) -> Vec<(usize, u16)> {
        let Some(q) = self.search.as_deref() else {
            return Vec::new();
        };
        if q.is_empty() {
            return Vec::new();
        }
        let Some(pane) = self.active_pane() else {
            return Vec::new();
        };
        let screen = pane.term.screen();
        let qb = q.as_bytes();
        let mut out = Vec::new();
        for b in 0..screen.total_lines() {
            let text = screen.line_text_abs(b);
            let tb = text.as_bytes();
            let mut i = 0usize;
            while i + qb.len() <= tb.len() {
                if tb[i..i + qb.len()].eq_ignore_ascii_case(qb) {
                    out.push((b, i as u16));
                    i += qb.len().max(1);
                    if out.len() >= 2000 {
                        return out;
                    }
                } else {
                    i += 1;
                }
            }
        }
        out
    }

    fn refresh_search(&mut self) {
        if self.search.is_none() {
            self.search_hits.clear();
            self.search_key.clear();
            return;
        }
        let key = format!(
            "{}\u{0}{}",
            self.search.as_deref().unwrap_or(""),
            self.active_pane_id().unwrap_or_default()
        );
        if key != self.search_key {
            self.search_key = key;
            self.search_hits = self.compute_search_hits();
            self.search_idx = self
                .search_idx
                .min(self.search_hits.len().saturating_sub(1));
        }
    }

    fn scroll_to_search_hit(&mut self) {
        let Some((b, _)) = self.search_hits.get(self.search_idx).copied() else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        let Some(pane) = tab.panes.iter_mut().find(|p| p.id == tab.active) else {
            return;
        };
        let hist = pane.term.screen().history_size();
        pane.scroll = hist.saturating_sub(b);
    }

    fn search_window(&mut self, ctx: &egui::Context) {
        if self.search.is_none() {
            return;
        }
        let lang = self.lang;
        let n = self.search_hits.len();
        let idx = self.search_idx;
        let Some(query) = self.search.as_mut() else {
            return;
        };
        let mut step = 0i32;
        let mut close = false;
        egui::Window::new(miao_term_ui::i18n::t(lang, "Find", "查找"))
            .collapsible(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 40.0])
            .show(ctx, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(query)
                        .hint_text(miao_term_ui::i18n::t(self.lang, "search…", "搜索…"))
                        .desired_width(260.0),
                );
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    step = if ui.input(|i| i.modifiers.shift) {
                        -1
                    } else {
                        1
                    };
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
                ui.label(format!("{} / {}", if n == 0 { 0 } else { idx + 1 }, n));
            });
        if close {
            self.search = None;
            self.search_hits.clear();
            self.search_key.clear();
            self.window.request_redraw();
            return;
        }
        if step != 0 && n > 0 {
            self.search_idx = ((idx as i32 + step).rem_euclid(n as i32)) as usize;
            self.scroll_to_search_hit();
            self.window.request_redraw();
        }
    }

    fn composer_window(&mut self, ctx: &egui::Context) {
        let target = self.active_pane().map(|p| p.id.clone()).unwrap_or_default();
        let Some(text) = self.composer.as_mut() else {
            return;
        };
        use miao_term_ui::i18n::t;
        let lang = self.lang;
        let mut open = true;
        let mut send = false;
        let mut queue = false;
        egui::Window::new(t(lang, "Composer", "Composer"))
            .open(&mut open)
            .default_size([520.0, 260.0])
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(format!("to {target}"))
                        .small()
                        .color(egui::Color32::from_gray(140)),
                );
                ui.add(
                    egui::TextEdit::multiline(text)
                        .hint_text(t(lang, "Prompt…", "提示词…"))
                        .desired_width(f32::INFINITY)
                        .desired_rows(8),
                );
                ui.horizontal(|ui| {
                    let ready = !text.trim().is_empty();
                    if ui
                        .add_enabled(ready, egui::Button::new(t(lang, "Send", "发送")))
                        .clicked()
                    {
                        send = true;
                    }
                    if ui
                        .add_enabled(ready, egui::Button::new(t(lang, "Queue it", "加入队列")))
                        .clicked()
                    {
                        queue = true;
                    }
                });
            });
        let draft = text.clone();
        if send {
            self.composer = None;
            self.write_input(format!("{draft}\r").as_bytes());
        } else if queue {
            self.prompts.push(draft);
            self.save_queue();
            self.composer = None;
        } else if !open {
            self.composer = None;
        }
    }

    fn check_updates(&mut self) {
        let Some(url) = self.update_url.clone() else {
            self.update_msg = Some("no update URL configured".to_string());
            return;
        };
        self.update_msg = Some("Checking for updates…".to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let out = std::process::Command::new("curl")
                .args(["-fsSL", "--max-time", "8", &url])
                .output();
            let msg = match out {
                Ok(o) if o.status.success() => {
                    let m = miao_term_ui::update::parse(&String::from_utf8_lossy(&o.stdout));
                    let local = env!("CARGO_PKG_VERSION");
                    if miao_term_ui::update::is_newer(&m.version, local) {
                        format!("Update available: v{}", m.version)
                    } else {
                        format!("Up to date (v{local})")
                    }
                }
                _ => "Update check failed".to_string(),
            };
            let _ = tx.send(msg);
        });
        self.update_rx = Some(rx);
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
            tab.ssh = true;
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
                        .hint_text(miao_term_ui::i18n::t(self.lang, "host", "主机"))
                        .desired_width(140.0),
                );
            });
            ui.horizontal(|ui| {
                ui.label(miao_term_ui::i18n::t(self.lang, "Path", "路径"));
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
                        readonly: false,
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
                            .hint_text(miao_term_ui::i18n::t(self.lang, "name", "名称"))
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

    /// Open a local file in the built-in editor. Returns false if it can't be
    /// read. Resets the vim runtime so a new file starts in Normal mode.
    fn open_editor(&mut self, path: std::path::PathBuf) -> bool {
        self.open_editor_ro(path, false)
    }

    /// Open a file in the built-in editor; `readonly` is used by MTP `app.view`.
    fn open_editor_ro(&mut self, path: std::path::PathBuf, readonly: bool) -> bool {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let preview = name.ends_with(".md") || name.ends_with(".markdown");
                let key = path.to_string_lossy().to_string();
                self.editor = Some(Editor {
                    path,
                    original: text.clone(),
                    text,
                    preview,
                    readonly,
                    remote: None,
                });
                self.vim_for.clear();
                self.recent_files.retain(|p| p != &key);
                *self.open_counts.entry(key.clone()).or_insert(0) += 1;
                self.recent_files.insert(0, key);
                self.recent_files.truncate(50);
                true
            }
            Err(e) => {
                eprintln!("miaotty-native: open failed: {e}");
                false
            }
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
            if self.open_editor(p) {
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
        if self.vim_for != title {
            self.vim_for = title.clone();
            self.vim = self.editor_vim.then(miao_term_ui::vim::VimRuntime::default);
        }
        let modified = ed.text != ed.original;
        let lang = miao_term_ui::syntax::detect(&match &ed.remote {
            Some((_, p)) => p.clone(),
            None => ed.path.to_string_lossy().to_string(),
        });
        let mut layouter =
            miao_term_ui::syntax::layouter(lang, egui::Color32::from_rgb(0xe5, 0xe5, 0xe5), 13.0);
        let mut open = true;
        let mut save = false;
        let mut quit = false;
        egui::Window::new(title)
            .open(&mut open)
            .default_size([640.0, 480.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ed.readonly {
                        ui.label(
                            egui::RichText::new(miao_term_ui::i18n::t(
                                self.lang,
                                "read-only",
                                "只读",
                            ))
                            .color(egui::Color32::from_gray(140)),
                        );
                    } else if ui
                        .button(miao_term_ui::i18n::t(self.lang, "Save", "保存"))
                        .clicked()
                    {
                        save = true;
                    }
                    ui.checkbox(
                        &mut ed.preview,
                        miao_term_ui::i18n::t(self.lang, "Markdown preview", "Markdown 预览"),
                    );
                    if modified && !ed.readonly {
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
                    let ch = self.theme.chrome();
                    let fg = miao_term_ui::chrome::bg_color(ch.text);
                    let panel = miao_term_ui::chrome::bg_color(ch.card);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            render_markdown(
                                ui,
                                &ed.text,
                                &mut self.cmark,
                                &mut self.mmd,
                                fg,
                                panel,
                            );
                        });
                } else {
                    let mut vim_effect = miao_term_ui::vim::VimEffect::Nothing;
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
                                            .size(13.0)
                                            .color(egui::Color32::from_gray(110)),
                                    )
                                    .selectable(false),
                                );
                                let text_id = egui::Id::new("miaotty-native-editor-text");
                                let mut edit = egui::TextEdit::multiline(&mut ed.text)
                                    .id(text_id)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .layouter(&mut layouter);
                                if ed.readonly {
                                    edit = edit.interactive(false);
                                }
                                ui.add(edit);
                                if let Some(v) = self.vim.as_mut().filter(|_| !ed.readonly) {
                                    vim_effect = miao_term_ui::vim::vim_handle(
                                        &mut ed.text,
                                        v,
                                        ui.ctx(),
                                        text_id,
                                    );
                                }
                            });
                        });
                    if vim_effect == miao_term_ui::vim::VimEffect::Save {
                        save = true;
                    }
                    if vim_effect == miao_term_ui::vim::VimEffect::Quit {
                        quit = true;
                    }
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
        if quit {
            self.editor = None;
            self.vim = None;
            return;
        }
        if !open {
            self.editor = None;
        }
    }

    fn prefix_window(&mut self, ctx: &egui::Context, i: usize) {
        let mut open = true;
        let mut buf = std::mem::take(&mut self.prefix_buf);
        let mut commit = false;
        egui::Window::new(miao_term_ui::i18n::t(self.lang, "Tab Prefix", "标签前缀"))
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let r = ui.text_edit_singleline(&mut buf);
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit = true;
                }
                if ui
                    .button(miao_term_ui::i18n::t(self.lang, "Set", "设置"))
                    .clicked()
                {
                    commit = true;
                }
            });
        self.prefix_buf = buf;
        if commit {
            if let Some(t) = self.tabs.get_mut(i) {
                let p = self.prefix_buf.trim().to_string();
                t.prefix = if p.is_empty() { None } else { Some(p) };
            }
            self.prefix_renaming = None;
            self.publish_panes();
        }
        if !open {
            self.prefix_renaming = None;
        }
    }

    /// A one-line editor for a tab's mark or group (ADR 0011). Empty clears it.
    #[allow(clippy::too_many_arguments)]
    fn tab_text_window(
        &mut self,
        ctx: &egui::Context,
        i: usize,
        en: &'static str,
        zh: &'static str,
        buf: &mut String,
        slot: &mut Option<usize>,
        is_group: bool,
    ) {
        let mut open = true;
        let mut text = std::mem::take(buf);
        let mut commit = false;
        egui::Window::new(miao_term_ui::i18n::t(self.lang, en, zh))
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let r = ui.text_edit_singleline(&mut text);
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit = true;
                }
                if ui
                    .button(miao_term_ui::i18n::t(self.lang, "Set", "设置"))
                    .clicked()
                {
                    commit = true;
                }
            });
        *buf = text;
        if commit {
            let value = buf.trim().to_string();
            if let Some(t) = self.tabs.get_mut(i) {
                let value = if value.is_empty() { None } else { Some(value) };
                if is_group {
                    t.group = value;
                } else {
                    t.mark = value;
                }
            }
            *slot = None;
            self.publish_panes();
        }
        if !open {
            *slot = None;
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

/// Draw a small chevron triangle (avoids font-glyph tofu).
fn chevron(p: &egui::Painter, rect: egui::Rect, open: bool, color: egui::Color32) {
    let c = rect.center();
    let (dx, dy) = (3.0, 4.0);
    let pts = if open {
        vec![
            egui::pos2(c.x - dx, c.y - dy * 0.5),
            egui::pos2(c.x + dx, c.y - dy * 0.5),
            egui::pos2(c.x, c.y + dy * 0.7),
        ]
    } else {
        vec![
            egui::pos2(c.x - dy * 0.5, c.y - dx),
            egui::pos2(c.x - dy * 0.5, c.y + dx),
            egui::pos2(c.x + dy * 0.7, c.y),
        ]
    };
    p.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
}

/// Render a lazily-loaded directory tree.
#[allow(clippy::too_many_arguments)]
fn render_dir_tree(
    ui: &mut egui::Ui,
    children: &HashMap<std::path::PathBuf, Vec<FileEntry>>,
    expanded: &std::collections::HashSet<std::path::PathBuf>,
    dir: &std::path::Path,
    depth: usize,
    filter: &str,
    open_file: &mut Option<std::path::PathBuf>,
    toggle: &mut Option<std::path::PathBuf>,
) {
    use miao_term_ui::icons::Icon;
    let Some(entries) = children.get(dir) else {
        return;
    };
    let muted = egui::Color32::from_gray(132);
    for e in entries {
        let is_dir = e.is_dir;
        if !is_dir && !filter.is_empty() && !e.name.to_lowercase().contains(filter) {
            continue;
        }
        let path = dir.join(&e.name);
        let is_open = expanded.contains(&path);
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 12.0);
            // The disclosure triangle is itself clickable.
            let (ir, chev_resp) =
                ui.allocate_exact_size(egui::Vec2::splat(14.0), egui::Sense::click());
            if is_dir {
                chevron(ui.painter(), ir, is_open, muted);
                if chev_resp.clicked() {
                    *toggle = Some(path.clone());
                }
            }
            let (ird, icon_resp) =
                ui.allocate_exact_size(egui::Vec2::splat(14.0), egui::Sense::click());
            miao_term_ui::icons::draw(
                ui.painter(),
                ird,
                if is_dir { Icon::Folder } else { Icon::File },
                muted,
            );
            if icon_resp.clicked() {
                if is_dir {
                    *toggle = Some(path.clone());
                } else {
                    *open_file = Some(path.clone());
                }
            }
            let resp = ui.selectable_label(false, egui::RichText::new(&e.name).size(12.0));
            if resp.clicked() {
                if is_dir {
                    *toggle = Some(path.clone());
                } else {
                    *open_file = Some(path.clone());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !is_dir {
                    ui.label(
                        egui::RichText::new(human_size(e.size))
                            .size(10.5)
                            .color(muted),
                    );
                }
            });
        });
        if is_dir && is_open {
            render_dir_tree(
                ui,
                children,
                expanded,
                &path,
                depth + 1,
                filter,
                open_file,
                toggle,
            );
        }
    }
}

/// Human-readable byte size for the Files panel.
fn human_size(n: u64) -> String {
    const UNIT: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i + 1 < UNIT.len() {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNIT[i])
    }
}

/// A stable texture key for an image: unique across panes.
/// sRGB-encoded `Rgb` → a linear `wgpu::Color` (the surface is `*Srgb`, so clear
/// and quad colours must be linear to avoid a washed-out look).
fn linear_color(c: miao_term_ui::theme::Rgb, alpha: f32) -> wgpu::Color {
    fn lin(v: u8) -> f64 {
        let s = v as f64 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    wgpu::Color {
        r: lin(c.0),
        g: lin(c.1),
        b: lin(c.2),
        a: alpha as f64,
    }
}

/// Pick a transparency-capable surface alpha mode when `transparent`, else the
/// first (usually Opaque). Falls back to Opaque if none is suitable.
fn pick_alpha_mode(
    modes: &[wgpu::CompositeAlphaMode],
    transparent: bool,
) -> wgpu::CompositeAlphaMode {
    use wgpu::CompositeAlphaMode as M;
    if transparent {
        // Only straight (post-multiplied) alpha matches our renderer, which
        // outputs non-premultiplied colours. If unsupported, stay opaque.
        if let Some(m) = modes.iter().copied().find(|m| *m == M::PostMultiplied) {
            return m;
        }
    }
    modes.first().copied().unwrap_or(M::Opaque)
}

/// Scissor rectangle (physical px) covering all panes, clamped to the surface.
fn grid_scissor(
    rects: &[(String, Rect)],
    scale: f32,
    width: u32,
    height: u32,
) -> (u32, u32, u32, u32) {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (_, r) in rects {
        minx = minx.min(r.x);
        miny = miny.min(r.y);
        maxx = maxx.max(r.x + r.w);
        maxy = maxy.max(r.y + r.h);
    }
    if rects.is_empty() {
        return (0, 0, 0, 0);
    }
    let sx = (minx * scale).max(0.0).min(width as f32) as u32;
    let sy = (miny * scale).max(0.0).min(height as f32) as u32;
    let sw = ((maxx - minx) * scale).max(0.0) as u32;
    let sh = ((maxy - miny) * scale).max(0.0) as u32;
    (sx, sy, sw.min(width - sx), sh.min(height - sy))
}

/// The URL token under `col` in a line: `(url, start_col, end_col)`.
fn link_at(line: &str, col: u16) -> Option<(String, u16, u16)> {
    let chars: Vec<char> = line.chars().collect();
    let col = col as usize;
    if col >= chars.len() || chars[col].is_whitespace() {
        return None;
    }
    fn is_break(c: char) -> bool {
        c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | ')' | '<' | '>' | '[' | ']')
    }
    let mut start = col;
    while start > 0 && !is_break(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < chars.len() && !is_break(chars[end + 1]) {
        end += 1;
    }
    let raw: String = chars[start..=end].iter().collect();
    let token = raw.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']']);
    if token.starts_with("http://")
        || token.starts_with("https://")
        || token.starts_with("ftp://")
        || token.starts_with("file://")
    {
        let end = start + token.chars().count() - 1;
        Some((token.to_string(), start as u16, end as u16))
    } else {
        None
    }
}

/// Escape shell metacharacters in pasted text (single-quote problem tokens).
fn shell_escape_text(s: &str) -> String {
    s.split_whitespace()
        .map(|tok| {
            if tok
                .chars()
                .all(|c| c.is_alphanumeric() || "/._-@%+=:,~".contains(c))
            {
                tok.to_string()
            } else {
                format!("'{}'", tok.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quote a path for the shell (single quotes when it contains anything
/// unusual), so a dropped file can be pasted into the terminal.
fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars()
        .all(|c| c.is_alphanumeric() || "/._-@%+=:,~".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Open a URL in the OS default browser.
fn open_external(target: &str) {
    #[cfg(target_os = "macos")]
    let cmd = ("open", vec![target]);
    #[cfg(target_os = "windows")]
    let cmd = ("cmd", vec!["/C", "start", "", target]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = ("xdg-open", vec![target]);
    let _ = std::process::Command::new(cmd.0).args(cmd.1).spawn();
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

impl ApplicationHandler<HostEvent> for Host {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let (init_w, init_h) = load_window_size().unwrap_or((1100.0, 720.0));
        let opacity = miao_term_config::Config::load()
            .background_opacity
            .clamp(0.1, 1.0);
        let attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(init_w, init_h))
            // The chrome is dark; ask the OS for a dark title bar so the window
            // does not open with a light strip that clashes with the app (Otty
            // themes its whole frame the same way).
            .with_theme(Some(winit::window::Theme::Dark))
            .with_transparent(opacity < 1.0);
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
            alpha_mode: pick_alpha_mode(&caps.alpha_modes, opacity < 1.0),
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        let quads = QuadRenderer::new(&device, format);
        let images = ImageRenderer::new(&device, format);
        let egui_ctx = egui::Context::default();
        egui_extras::install_image_loaders(&egui_ctx);
        install_egui_fonts(&egui_ctx);
        configure_egui(&egui_ctx);
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
        let theme = Theme::from_config(&cfg.theme, cfg.cursor_style);
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
            images,
            instance,
            adapter,
            pip: None,
            pip_request: std::env::var_os("MIAOTTY_PIP").is_some(),
            graphics_enabled: cfg.graphics,
            renderers: HashMap::new(),
            mtp: self.mtp.clone(),
            tabs: Vec::new(),
            active_tab: 0,
            theme,
            rules: miao_term_config::view::RuleSet::load(),
            cw,
            ch,
            font_size,
            default_font_size: font_size,
            line_ratio,
            font_family,
            lang,
            mods: ModifiersState::empty(),
            selection: None,
            dragging: false,
            divider_drag: None,
            mouse_captured: None,
            cursor: (0.0, 0.0),
            // `MIAOTTY_PREEDIT` seeds the IME overlay for captures/QA.
            preedit: std::env::var("MIAOTTY_PREEDIT").unwrap_or_default(),
            show_sidebar: true,
            show_details: true,
            renaming: None,
            rename_buf: String::new(),
            theme_name,
            show_palette: false,
            palette_query: String::new(),
            palette_idx: 0,
            show_settings: false,
            editor: None,
            open_path: String::new(),
            show_open: false,
            recipe_dialog: None,
            recipe_name: String::new(),
            recipe_list: Vec::new(),
            ssh_dialog: None,
            remote_dialog: None,
            editor_vim: cfg.editor_vim,
            vim: cfg.editor_vim.then(miao_term_ui::vim::VimRuntime::default),
            vim_for: String::new(),
            cmark: egui_commonmark::CommonMarkCache::default(),
            mmd: Mmd {
                dir: window_file()
                    .map(|p| p.with_file_name("mermaid-cache"))
                    .unwrap_or_default(),
                cmd: cfg.mermaid_command.clone(),
                cache: std::collections::HashMap::new(),
            },
            recent_files: Vec::new(),
            open_counts: HashMap::new(),
            integration_msg: None,
            read_only: false,
            hint_mode: false,
            hints: Vec::new(),
            tree_expanded: std::collections::HashSet::new(),
            tree_children: HashMap::new(),
            files_filter: String::new(),
            prefix_renaming: None,
            prefix_buf: String::new(),
            mark_renaming: None,
            mark_buf: String::new(),
            group_renaming: None,
            group_buf: String::new(),
            hotkeys: None,
            opacity,
            notifications: cfg.notifications,
            prevent_sleep: cfg.prevent_sleep,
            sleep: miao_term_ui::agentloop::SleepGuard::new(),
            agent_states: HashMap::new(),
            composer: None,
            quick: None,
            closed: Vec::new(),
            quick_pane: None,
            quick_return: None,
            hover_pointer: false,
            search: None,
            search_idx: 0,
            search_hits: Vec::new(),
            search_key: String::new(),
            update_url: cfg.update_check_url.clone(),
            update_rx: None,
            update_msg: None,
            details_tab: std::env::var("MIAOTTY_DETAILS_TAB")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            details_cwd: None,
            details_data: None,
            details_rx: None,
            details_at: Instant::now(),
            prompts: load_queue(),
            prompt_input: String::new(),
            last_title: None,
            focused: false,
            cursor_on: true,
            last_blink: Instant::now(),
            start: Instant::now(),
            shot_now: false,
            egui_ctx,
            egui_state,
            egui_renderer,
        };
        if !state.restore_session() {
            state.new_tab();
        }
        #[cfg(target_os = "macos")]
        if menu_in_os() {
            let proxy = self.proxy.clone();
            match appmenu::install(state.lang, proxy) {
                Some(menu) => self.menu = Some(menu),
                None => eprintln!("miaotty-native: could not install the application menu"),
            }
        }
        let args: Vec<String> = std::env::args().skip(1).collect();
        let intent = miao_term_ui::launch::Intent::from_args(&args);
        state.apply_launch(&intent);
        if let Some(spec) = cfg.quick_terminal_hotkey.clone() {
            let proxy = self.proxy.clone();
            state.hotkeys = miao_term_ui::hotkey::Hotkeys::register(&spec, move || {
                let _ = proxy.send_event(HostEvent::Hotkey);
            });
            if state.hotkeys.is_none() {
                eprintln!("miaotty-native: could not register hotkey {spec}");
            }
        }
        state.window.request_redraw();
        self.state = Some(state);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: HostEvent) {
        let Some(state) = &mut self.state else {
            return;
        };
        match event {
            // PTY output / MTP work: just repaint.
            HostEvent::Wake => state.window.request_redraw(),
            // Global Quick Terminal hotkey.
            HostEvent::Hotkey => {
                state.toggle_quick_terminal();
                state.window.focus_window();
            }
            // OS menu bar command.
            #[cfg(target_os = "macos")]
            HostEvent::Menu(id) => {
                chrome::Chrome::on_menu(state, id);
                state.window.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(state) = &mut self.state {
            // Launches forwarded by later processes (ADR 0019).
            for line in miao_term_ui::launch::drain_inbox() {
                let intent = miao_term_ui::launch::Intent::decode(&line);
                state.apply_launch(&intent);
                state.window.request_redraw();
            }
            if let Some(rx) = state.update_rx.take() {
                match rx.try_recv() {
                    Ok(msg) => {
                        state.update_msg = Some(msg);
                        state.window.request_redraw();
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => state.update_rx = Some(rx),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
                }
            }
            state.poll_details();
            state.ensure_details();
            if state
                .hotkeys
                .as_ref()
                .is_some_and(miao_term_ui::hotkey::Hotkeys::take_pending)
            {
                let visible = state.window.is_visible().unwrap_or(true);
                state.window.set_visible(!visible);
                if !visible {
                    state.window.focus_window();
                }
                state.window.request_redraw();
            }
            if !state.shot_now {
                if let Ok(v) = std::env::var("MIAOTTY_NATIVE_SHOT_AFTER") {
                    let secs = v.parse::<f64>().unwrap_or(-1.0);
                    if secs >= 0.0 {
                        let at = state.start + Duration::from_secs_f64(secs);
                        if Instant::now() >= at {
                            state.shot_now = true;
                            state.window.request_redraw();
                        } else {
                            // Ensure we wake even without focus/blink events.
                            event_loop.set_control_flow(ControlFlow::WaitUntil(at));
                        }
                    }
                }
            }
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

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if state.pip_request {
            state.pip_request = false;
            state.create_pip(event_loop);
        }
        // Route events for the picture-in-picture window.
        if state
            .pip
            .as_ref()
            .map(|p| p.window.id() == id)
            .unwrap_or(false)
        {
            match event {
                WindowEvent::CloseRequested => state.pip = None,
                WindowEvent::Resized(_) => state.resize_pip(),
                WindowEvent::RedrawRequested => state.render_pip(),
                _ => {}
            }
            return;
        }
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
            WindowEvent::ModifiersChanged(m) => {
                state.mods = m.state();
                state.window.request_redraw();
            }
            WindowEvent::Ime(winit::event::Ime::Preedit(text, _)) => {
                // Inline composition: drawn at the cursor until it commits.
                if state.preedit != text {
                    state.preedit = text;
                    state.window.request_redraw();
                }
            }
            WindowEvent::Ime(winit::event::Ime::Disabled) => {
                if !state.preedit.is_empty() {
                    state.preedit.clear();
                    state.window.request_redraw();
                }
            }
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                // A commit can carry a control character (Enter/Tab/…); those are
                // already sent by the key handler, so only forward real text.
                let control_only = !text.is_empty() && text.chars().all(char::is_control);
                state.preedit.clear();
                if !control_only && !state.egui_ctx.wants_keyboard_input() {
                    state.write_input(text.as_bytes());
                }
            }
            WindowEvent::DroppedFile(path) => {
                let scale = state.window.scale_factor() as f32;
                let (px, py) = (state.cursor.0 as f32, state.cursor.1 as f32);
                let over_pane = state.pane_rects().iter().any(|(_, r)| {
                    px >= r.x * scale
                        && px < (r.x + r.w) * scale
                        && py >= r.y * scale
                        && py < (r.y + r.h) * scale
                });
                if over_pane {
                    // Drop onto the terminal: paste the shell-quoted path.
                    let quoted = shell_quote(&path.to_string_lossy());
                    let pasted = format!("{quoted} ");
                    state.paste(&pasted);
                } else {
                    state.open_editor(path);
                }
                state.window.request_redraw();
            }
            WindowEvent::HoveredFile(_) => {}
            WindowEvent::MouseInput {
                state: es, button, ..
            } => match button {
                MouseButton::Left => {
                    let (px, py) = (state.cursor.0 as f32, state.cursor.1 as f32);
                    if es == ElementState::Pressed {
                        // ⌘/Ctrl-click stays a terminal-level link gesture.
                        let linked = if state.mods.super_key() || state.mods.control_key() {
                            match state.link_at_pointer() {
                                Some(hit) => {
                                    open_external(&hit.url);
                                    true
                                }
                                None => false,
                            }
                        } else {
                            false
                        };
                        if !linked {
                            if state.forward_mouse(px, py, 0, true, false) {
                                state.mouse_captured = Some(0);
                            } else {
                                // Prefer a divider under the pointer, else a selection.
                                let scale = state.window.scale_factor() as f32;
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
                            }
                        }
                    } else {
                        if state.mouse_captured.take() == Some(0) {
                            state.forward_mouse(px, py, 0, false, false);
                        } else if state.divider_drag.take().is_none() {
                            let ctx = state.egui_ctx.clone();
                            state.copy_selection(&ctx);
                        }
                        state.dragging = false;
                    }
                }
                MouseButton::Right => {
                    let (px, py) = (state.cursor.0 as f32, state.cursor.1 as f32);
                    if es == ElementState::Pressed {
                        if state.forward_mouse(px, py, 2, true, false) {
                            state.mouse_captured = Some(2);
                        } else if state.selection.is_some() {
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
                    } else if state.mouse_captured.take() == Some(2) {
                        state.forward_mouse(px, py, 2, false, false);
                    }
                }
                MouseButton::Middle => {
                    let (px, py) = (state.cursor.0 as f32, state.cursor.1 as f32);
                    if es == ElementState::Pressed {
                        if state.forward_mouse(px, py, 1, true, false) {
                            state.mouse_captured = Some(1);
                        }
                    } else if state.mouse_captured.take() == Some(1) {
                        state.forward_mouse(px, py, 1, false, false);
                    }
                }
                _ => {}
            },
            WindowEvent::CursorMoved { position, .. } => {
                state.cursor = (position.x, position.y);
                if state.mods.super_key() || state.mods.control_key() {
                    state.window.request_redraw();
                }
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
                } else if let Some(btn) = state.mouse_captured {
                    // Drag report for the button the application captured.
                    state.forward_mouse(px, py, btn, true, true);
                } else {
                    // Hover report, only if the application asked for motion.
                    state.forward_mouse(px, py, 3, false, true);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                use winit::event::MouseScrollDelta;
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as i32,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 12.0) as i32,
                };
                if lines != 0 {
                    // Full-screen apps that grabbed the mouse (TUIs like miao)
                    // expect wheel events instead of scrollback navigation.
                    let (px, py) = (state.cursor.0 as f32, state.cursor.1 as f32);
                    let button = if lines > 0 { 64 } else { 65 };
                    let mut consumed = false;
                    for _ in 0..lines.unsigned_abs().min(16) {
                        consumed |= state.forward_mouse(px, py, button, true, false);
                    }
                    if !consumed {
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
                    }
                }
                state.window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if state.hint_mode && event.state == ElementState::Pressed {
                    {
                        let key = match &event.logical_key {
                            Key::Character(c) => c.chars().next(),
                            Key::Named(NamedKey::Escape) => Some('\u{1b}'),
                            _ => None,
                        };
                        if let Some(ch) = key {
                            if ch == '\u{1b}' {
                                state.cancel_hints();
                            } else if let Some(h) = state
                                .hints
                                .iter()
                                .find(|h| h.label.starts_with(ch))
                                .cloned()
                            {
                                state.cancel_hints();
                                if h.is_path {
                                    state.open_editor(std::path::PathBuf::from(&h.target));
                                } else {
                                    open_external(&h.target);
                                }
                            }
                            state.window.request_redraw();
                            return;
                        }
                    }
                }
                if state.egui_ctx.wants_keyboard_input() {
                    state.window.request_redraw();
                    return;
                }
                if let Some(s) = shortcut(&event, state.mods) {
                    if s.new_tab {
                        state.new_tab();
                    }
                    if s.reopen {
                        state.reopen_tab();
                    }
                    if s.quick_terminal {
                        state.toggle_quick_terminal();
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
                    if s.tab != 0 {
                        state.cycle_tab(s.tab > 0);
                    }
                    if s.hint {
                        state.build_hints();
                    }
                    if s.find != 0 {
                        let n = state.search_hits.len();
                        if state.search.is_some() && n > 0 {
                            state.search_idx =
                                ((state.search_idx as i32 + s.find).rem_euclid(n as i32)) as usize;
                            state.scroll_to_search_hit();
                        }
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
                        state.palette_idx = 0;
                    }
                    if s.settings {
                        state.show_settings = true;
                    }
                    if s.composer {
                        state.composer = Some(String::new());
                    }
                    if s.quickly {
                        state.quick = Some(String::new());
                    }
                    if s.search {
                        state.search = Some(String::new());
                        state.search_idx = 0;
                        state.search_key.clear();
                    }
                    state.window.request_redraw();
                } else {
                    // winit emits a `KeyboardInput` for key-up as well as key-down
                    // (macOS always does). `encode_key` returns an escape sequence
                    // for Enter/Tab/Backspace/arrows no matter the state, so acting
                    // on releases sent every special key twice — one Return became
                    // a blank line. Characters were unaffected because `Char`
                    // encodes to nothing there. Only presses carry input.
                    if event.state != ElementState::Pressed {
                        return;
                    }
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

/// Encode a mouse report for the terminal application.
///
/// `button`: 0 left, 1 middle, 2 right, 3 none (hover), 64 wheel-up, 65 wheel-down.
/// With SGR encoding (`?1006h`) a press ends in `M` and a release in `m`; wheel
/// and motion reports have no release, so they always use `M`. Otherwise the
/// legacy X10 encoding applies (coordinates are limited to 223).
fn mouse_report(
    sgr: bool,
    button: u8,
    pressed: bool,
    motion: bool,
    col: u16,
    row: u16,
) -> Option<String> {
    if col == 0 || row == 0 {
        return None;
    }
    let code = if motion && button < 64 {
        button + 32
    } else {
        button
    };
    if sgr {
        let end = if pressed || motion || button >= 64 {
            'M'
        } else {
            'm'
        };
        return Some(format!("\x1b[<{code};{col};{row}{end}"));
    }
    if col > 223 || row > 223 {
        return None;
    }
    let bytes = [
        0x1b,
        b'[',
        b'M',
        32u8.saturating_add(code),
        32u8.saturating_add(col as u8),
        32u8.saturating_add(row as u8),
    ];
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// External Mermaid rendering (opt-in via `mermaid-command`) with a cache.
#[derive(Default)]
struct Mmd {
    dir: std::path::PathBuf,
    cmd: Option<String>,
    cache: std::collections::HashMap<u64, Option<std::path::PathBuf>>,
}

impl Mmd {
    fn image(&mut self, source: &str) -> Option<std::path::PathBuf> {
        let cmd = self.cmd.clone()?;
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut h);
        let key = h.finish();
        if let Some(v) = self.cache.get(&key) {
            return v.clone();
        }
        let img = miao_term_ui::mermaid::render_external(source, &cmd, &self.dir);
        self.cache.insert(key, img.clone());
        img
    }
}

fn open_commonmark(ui: &mut egui::Ui, cache: &mut egui_commonmark::CommonMarkCache, text: &str) {
    if !text.trim().is_empty() {
        egui_commonmark::CommonMarkViewer::new().show(ui, cache, text);
    }
}

/// Render Markdown, drawing ```mermaid blocks (built-in subset or `mmdc`).
fn render_markdown(
    ui: &mut egui::Ui,
    text: &str,
    cache: &mut egui_commonmark::CommonMarkCache,
    mmd: &mut Mmd,
    fg: egui::Color32,
    panel: egui::Color32,
) {
    let mut rest = text;
    loop {
        let Some(i) = rest.find("```mermaid") else {
            open_commonmark(ui, cache, rest);
            break;
        };
        open_commonmark(ui, cache, &rest[..i]);
        let after = &rest[i + "```mermaid".len()..];
        let Some(j) = after.find("```") else {
            open_commonmark(ui, cache, after);
            break;
        };
        let body = &after[..j];
        if let Some(path) = mmd.image(body) {
            if let Some(uri) = miao_term_ui::markdown::image_uri(&path.display().to_string(), None)
            {
                ui.add(
                    egui::Image::new(uri)
                        .max_width(ui.available_width())
                        .max_height(400.0),
                );
            }
        } else if let Some(d) = miao_term_ui::mermaid::parse_diagram(body) {
            miao_term_ui::mermaid::show_diagram(ui, &d, fg, panel);
        } else {
            ui.label(
                egui::RichText::new(miao_term_ui::i18n::t(
                    miao_term_ui::i18n::Lang::En,
                    "Mermaid diagram (not rendered)",
                    "Mermaid 图（未渲染）",
                ))
                .size(12.0)
                .color(egui::Color32::from_gray(150)),
            );
        }
        rest = &after[j + 3..];
        if rest.trim().is_empty() {
            break;
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

fn files_rows(cwd: &std::path::Path) -> Vec<FileEntry> {
    let Ok(read) = std::fs::read_dir(cwd) else {
        return Vec::new();
    };
    let mut rows: Vec<FileEntry> = read
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .take(500)
        .map(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            FileEntry {
                name: e.file_name().to_string_lossy().to_string(),
                is_dir,
                size,
            }
        })
        .collect();
    // Directories first, then name.
    rows.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    rows
}

fn configure_egui(ctx: &egui::Context) {
    let ch = miao_term_ui::theme::Chrome::dark();
    let col = |c: miao_term_ui::theme::Rgb| egui::Color32::from_rgb(c.0, c.1, c.2);
    let mut style = (*ctx.style()).clone();
    {
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.window_fill = col(ch.bg);
        v.panel_fill = col(ch.card);
        v.extreme_bg_color = col(ch.bg);
        v.faint_bg_color = col(ch.hover);
        v.override_text_color = Some(col(ch.text));
        v.hyperlink_color = col(ch.accent);
        v.selection.bg_fill = col(ch.active);
        v.widgets.inactive.weak_bg_fill = col(ch.hover);
        v.widgets.hovered.weak_bg_fill = col(ch.active);
        v.widgets.active.weak_bg_fill = col(ch.active);
        v.widgets.inactive.bg_fill = col(ch.hover);
        v.widgets.hovered.bg_fill = col(ch.active);
        v.widgets.active.bg_fill = col(ch.active);
        // Panel separators: egui defaults to a flat grey; use the sidebar edge
        // colour so the split reads like Otty's `[sidebar] border-right`.
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, col(ch.border));
    }
    let r = egui::Rounding::same(6.0);
    style.visuals.widgets.inactive.rounding = r;
    style.visuals.widgets.hovered.rounding = r;
    style.visuals.widgets.active.rounding = r;
    style.visuals.widgets.open.rounding = r;
    style.visuals.window_rounding = egui::Rounding::same(10.0);
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(6.0, 2.0);
    ctx.set_style(style);
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
    // Tabler Icons (MIT), subset to the glyphs we use, as the UI icon family.
    fonts.font_data.insert(
        "tabler".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../../assets/fonts/tabler-icons-subset.ttf"
        ))),
    );
    fonts.families.insert(
        egui::FontFamily::Name("tabler".into()),
        vec!["tabler".to_owned()],
    );
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

fn queue_file() -> Option<std::path::PathBuf> {
    window_file().map(|p| p.with_file_name("native-queue.json"))
}

/// Load the persisted prompt queue (agent Composer drafts).
fn load_queue() -> Vec<String> {
    let Some(path) = queue_file() else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| {
            v.get("prompts").and_then(|p| p.as_array()).map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default()
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

impl chrome::Chrome for State {
    fn draws_menu_bar(&self) -> bool {
        // macOS inside an app bundle uses the system menu bar instead (ADR 0031).
        !menu_in_os()
    }

    fn pane_close_rects(&self) -> Vec<(String, egui::Rect)> {
        self.pane_rects()
            .into_iter()
            .map(|(id, r)| {
                (
                    id,
                    egui::Rect::from_min_size(egui::pos2(r.x, r.y), egui::vec2(r.w, r.h)),
                )
            })
            .collect()
    }

    fn on_close_pane(&mut self, id: &str) {
        self.close_pane_id(id);
        self.window.request_redraw();
    }

    fn lang(&self) -> miao_term_ui::i18n::Lang {
        self.lang
    }
    fn tabs(&self) -> Vec<chrome::ChromeTab> {
        self.tabs
            .iter()
            .map(|t| {
                let badge = self.agent_badge(&t.active);
                let icon = if badge.is_some() {
                    miao_term_ui::icons::Icon::Agent
                } else if t.ssh {
                    miao_term_ui::icons::Icon::Server
                } else {
                    miao_term_ui::icons::Icon::Terminal
                };
                let mut title = self.title_of(t);
                if let Some(p) = &t.prefix {
                    title = format!("[{p}] {title}");
                }
                if let Some(m) = &t.mark {
                    title = format!("{title}{m}");
                }
                chrome::ChromeTab { title, badge, icon }
            })
            .collect()
    }
    fn active_tab(&self) -> usize {
        self.active_tab
    }
    fn show_sidebar(&self) -> bool {
        self.show_sidebar
    }
    fn show_details(&self) -> bool {
        self.show_details
    }
    fn details_tab(&self) -> usize {
        self.details_tab.min(6)
    }
    fn details_title(&self) -> String {
        self.details_content(self.details_tab.min(6)).0.to_string()
    }
    fn details_rows(&self) -> Vec<(String, String)> {
        self.details_content(self.details_tab.min(6)).1
    }
    fn read_only(&self) -> bool {
        self.read_only
    }
    fn status_right(&self) -> String {
        if let Some(p) = self.active_pane() {
            if let Some(a) = self
                .mtp
                .agent_for(&p.id)
                .and_then(|v| v.get("agent").and_then(|x| x.as_str()).map(str::to_string))
            {
                return a;
            }
        }
        std::env::var("SHELL")
            .ok()
            .and_then(|s| s.rsplit('/').next().map(str::to_string))
            .unwrap_or_default()
    }
    fn theme(&self) -> Theme {
        self.theme.clone()
    }
    fn details_is_queue(&self) -> bool {
        self.details_tab == 6
    }
    fn details_list(&self) -> Option<Vec<chrome::ChromeItem>> {
        use chrome::ChromeItem;
        use miao_term_ui::icons::Icon;
        let item = |icon: Icon, label: String, meta: String| ChromeItem { icon, label, meta };
        match self.details_tab.min(6) {
            2 => Some(
                self.outline_rows()
                    .into_iter()
                    .map(|(cwd, cmd)| item(Icon::Terminal, cmd, cwd))
                    .collect(),
            ),
            3 => Some(
                self.details_data
                    .as_ref()
                    .map(|d| {
                        d.git
                            .iter()
                            .map(|(k, v)| item(Icon::Git, v.clone(), k.clone()))
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            4 => Some(
                self.details_data
                    .as_ref()
                    .map(|d| {
                        d.files
                            .iter()
                            .map(|f| {
                                let icon = if f.is_dir { Icon::Folder } else { Icon::File };
                                let meta = if f.is_dir {
                                    String::new()
                                } else {
                                    human_size(f.size)
                                };
                                item(icon, f.name.clone(), meta)
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            5 => Some(
                self.details_data
                    .as_ref()
                    .map(|d| {
                        d.ports
                            .iter()
                            .map(|(k, v)| item(Icon::Ports, v.clone(), k.clone()))
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            _ => None,
        }
    }
    fn details_body(&mut self, ui: &mut egui::Ui, lang: miao_term_ui::i18n::Lang) -> bool {
        if self.details_tab.min(6) == 4 {
            self.files_body(ui, lang);
            true
        } else {
            false
        }
    }
    fn status(&self) -> String {
        self.status_text()
    }
    fn queue(&self) -> Vec<String> {
        self.prompts.clone()
    }
    fn take_queue_input(&mut self) -> String {
        std::mem::take(&mut self.prompt_input)
    }
    fn set_queue_input(&mut self, input: String) {
        self.prompt_input = input;
    }
    fn on_new_tab(&mut self) {
        self.new_tab();
    }
    fn on_switch_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            self.active_tab = i;
            self.selection = None;
        }
    }
    fn on_close_tab(&mut self, i: usize) {
        self.active_tab = i;
        self.close_pane();
    }
    fn on_rename_tab(&mut self, i: usize) {
        let title = self
            .tabs
            .get(i)
            .map(|t| self.title_of(t))
            .unwrap_or_default();
        self.renaming = Some(i);
        self.rename_buf = title;
    }
    fn on_duplicate_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            self.active_tab = i;
            self.duplicate_tab();
        }
    }
    fn on_close_others(&mut self, i: usize) {
        if i < self.tabs.len() {
            let keep = self.tabs.remove(i);
            self.tabs = vec![keep];
            self.active_tab = 0;
            self.selection = None;
            self.publish_panes();
        }
    }
    fn on_close_below(&mut self, i: usize) {
        if i < self.tabs.len() {
            self.tabs.truncate(i + 1);
            self.active_tab = self.active_tab.min(self.tabs.len() - 1);
            self.selection = None;
            self.publish_panes();
        }
    }
    fn on_move_tab(&mut self, i: usize, delta: i32) {
        let j = i as i32 + delta;
        if i < self.tabs.len() && j >= 0 && (j as usize) < self.tabs.len() {
            self.tabs.swap(i, j as usize);
            if self.active_tab == i {
                self.active_tab = j as usize;
            } else if self.active_tab == j as usize {
                self.active_tab = i;
            }
            self.publish_panes();
        }
    }
    fn tab_groups(&self) -> Vec<Option<String>> {
        self.tabs.iter().map(|t| t.group.clone()).collect()
    }

    fn on_mark_tab(&mut self, i: usize) {
        if let Some(t) = self.tabs.get(i) {
            self.mark_buf = t.mark.clone().unwrap_or_default();
        }
        self.mark_renaming = Some(i);
    }

    fn on_group_tab(&mut self, i: usize) {
        if let Some(t) = self.tabs.get(i) {
            self.group_buf = t.group.clone().unwrap_or_default();
        }
        self.group_renaming = Some(i);
    }

    fn on_ungroup_tab(&mut self, i: usize) {
        if let Some(t) = self.tabs.get_mut(i) {
            t.group = None;
        }
        self.publish_panes();
    }

    fn on_set_prefix(&mut self, i: usize) {
        self.prefix_buf = self
            .tabs
            .get(i)
            .and_then(|t| t.prefix.clone())
            .unwrap_or_default();
        self.prefix_renaming = Some(i);
    }
    fn on_reorder_tab(&mut self, from: usize, to: usize) {
        if from < self.tabs.len() && to < self.tabs.len() && from != to {
            let was_active = self.active_tab == from;
            let tab = self.tabs.remove(from);
            self.tabs.insert(to, tab);
            if was_active {
                self.active_tab = to;
            }
            self.publish_panes();
        }
    }
    fn on_font_delta(&mut self, d: f32) {
        self.font_size = (self.font_size + d).clamp(6.0, 40.0);
        let (cw, ch) =
            State::cell_size(self.font_size, self.line_ratio, self.font_family.as_deref());
        self.cw = cw;
        self.ch = ch;
        self.resize();
    }
    fn on_toggle_sidebar(&mut self) {
        self.show_sidebar = !self.show_sidebar;
    }
    fn on_toggle_details(&mut self) {
        self.show_details = !self.show_details;
    }
    fn on_details_tab(&mut self, i: usize) {
        self.details_tab = i;
    }
    fn on_queue_add(&mut self) {
        if !self.prompt_input.is_empty() {
            let p = std::mem::take(&mut self.prompt_input);
            self.prompts.push(p);
            self.save_queue();
        }
    }
    fn on_queue_send(&mut self, i: usize) {
        if let Some(item) = self.prompts.get(i).cloned() {
            self.write_input(format!("{item}\r").as_bytes());
        }
    }
    fn on_queue_remove(&mut self, i: usize) {
        if i < self.prompts.len() {
            self.prompts.remove(i);
            self.save_queue();
        }
    }
    fn on_queue_send_all(&mut self) {
        let items = std::mem::take(&mut self.prompts);
        for item in items {
            self.write_input(format!("{item}\r").as_bytes());
        }
        self.save_queue();
    }
    fn on_queue_clear(&mut self) {
        self.prompts.clear();
        self.save_queue();
    }
    fn on_menu(&mut self, id: chrome::MenuId) {
        use chrome::MenuId::*;
        let cmd = match id {
            NewTab => Cmd::NewTab,
            QuickTerminal => Cmd::QuickTerminal,
            ClosePane => Cmd::ClosePane,
            OpenFile => Cmd::OpenFile,
            Save => Cmd::Save,
            SaveRecipe => Cmd::SaveRecipe,
            OpenRecipe => Cmd::OpenRecipe,
            NewSsh => Cmd::NewSsh,
            OpenRemote => Cmd::OpenRemote,
            Composer => Cmd::Composer,
            CheckUpdates => Cmd::CheckUpdates,
            Copy => Cmd::Copy,
            Paste => Cmd::Paste,
            SplitRight => Cmd::SplitRight,
            SplitDown => Cmd::SplitDown,
            ToggleSidebar => Cmd::ToggleSidebar,
            ToggleDetails => Cmd::ToggleDetails,
            FontUp => Cmd::FontUp,
            FontDown => Cmd::FontDown,
            FontReset => Cmd::FontReset,
            Palette => Cmd::Palette,
            CopyAnsi => Cmd::CopyAnsi,
            PasteEscaped => Cmd::PasteEscaped,
            Find => Cmd::Find,
            FindNext => Cmd::FindNext,
            FindPrev => Cmd::FindPrev,
            UseSelForFind => Cmd::UseSelForFind,
            JumpToSel => Cmd::JumpToSel,
            FindInAllTabs => Cmd::FindInAllTabs,
            Fullscreen => Cmd::Fullscreen,
            ReadOnly => Cmd::ReadOnly,
            HintMode => Cmd::HintMode,
            Pip => Cmd::Pip,
            ClearScreen => Cmd::ClearScreen,
            ClearScrollback => Cmd::ClearScrollback,
            DuplicateTab => Cmd::DuplicateTab,
            ReopenClosed => Cmd::ReopenClosed,
            SelectAll => Cmd::SelectAll,
            CopyPath => Cmd::CopyPath,
            RevealCwd => Cmd::RevealCwd,
            Settings => Cmd::Settings,
            Quit => Cmd::Quit,
        };
        self.run_command(cmd);
        self.window.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn link_detection() {
        let line = "see https://example.com/a?b=1 now";
        assert_eq!(
            super::link_at(line, 8).map(|(u, _, _)| u).as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(super::link_at(line, 0), None);
        assert_eq!(
            super::link_at("(https://x.io).", 2)
                .map(|(u, _, _)| u)
                .as_deref(),
            Some("https://x.io")
        );
    }

    use super::*;

    #[test]
    fn shell_quoting() {
        assert_eq!(super::shell_quote("/tmp/a.txt"), "/tmp/a.txt");
        assert_eq!(super::shell_quote("/a b/c"), "'/a b/c'");
        assert_eq!(super::shell_quote("it's"), "'it'\\''s'");
    }

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

    #[test]
    fn sgr_mouse_reports() {
        assert_eq!(
            super::mouse_report(true, 0, true, false, 10, 5).unwrap(),
            "\x1b[<0;10;5M"
        );
        assert_eq!(
            super::mouse_report(true, 0, false, false, 10, 5).unwrap(),
            "\x1b[<0;10;5m"
        );
        // Drag: left button plus the motion bit.
        assert_eq!(
            super::mouse_report(true, 0, true, true, 10, 5).unwrap(),
            "\x1b[<32;10;5M"
        );
        // Hover without a button.
        assert_eq!(
            super::mouse_report(true, 3, false, true, 10, 5).unwrap(),
            "\x1b[<35;10;5M"
        );
        // Wheel events have no release.
        assert_eq!(
            super::mouse_report(true, 64, true, false, 10, 5).unwrap(),
            "\x1b[<64;10;5M"
        );
        assert_eq!(
            super::mouse_report(true, 65, false, false, 10, 5).unwrap(),
            "\x1b[<65;10;5M"
        );
    }

    #[test]
    fn x10_mouse_reports_and_bounds() {
        assert_eq!(
            super::mouse_report(false, 0, true, false, 1, 1).unwrap(),
            "\x1b[M\x20\x21\x21"
        );
        assert_eq!(
            super::mouse_report(false, 2, true, false, 5, 9).unwrap(),
            "\x1b[M\x22\x25\x29"
        );
        assert!(super::mouse_report(false, 0, true, false, 224, 1).is_none());
        assert!(super::mouse_report(false, 0, true, false, 1, 224).is_none());
        assert!(super::mouse_report(true, 0, true, false, 0, 1).is_none());
    }
}
