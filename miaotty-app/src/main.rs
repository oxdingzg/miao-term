//! miaotty — a cross-platform terminal (engine: `miao-term-core`).
//!
//! R0/R1 bootstrap UI on eframe/egui: multi-tab, left sidebar, selection +
//! copy/paste, scrollback, wide-char (CJK) layout, cursor blink. The terminal
//! grid is drawn with egui for iteration speed; R1 replaces this with the
//! custom wgpu renderer (`term-render`) per the architecture.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use eframe::egui;
use eframe::egui_wgpu;
use unicode_width::UnicodeWidthChar;

mod agentloop;
mod hotkey;
mod i18n;
mod icons;
mod integration;
mod launch;
mod panels;
mod ssh;
mod update;

use miao_term_core::aterm::{ATerm, Color, NamedColor};
use miao_term_core::Terminal;

fn main() -> eframe::Result<()> {
    // Start the MTP control plane and export the socket so shells (and the
    // existing `miaotty-cli`) inherit it.
    let socket = miao_term_mtp::default_socket();
    std::env::set_var("MIAOTTY_SOCKET", &socket);
    let cfg = miao_term_config::Config::load();
    let intent = launch::Intent::from_args(&std::env::args().skip(1).collect::<Vec<_>>());

    // Single instance: a later launch (e.g. a second `ssh://` link) is handed
    // to the running instance and this process exits.
    if forward_to_running(&intent.encode()) {
        eprintln!("miaotty: forwarded to the running instance");
        return Ok(());
    }

    // MIAOTTY_MTP_TOKEN (if set) makes the host require it on every request.
    let state = miao_term_mtp::ServerState::with_token(std::env::var("MIAOTTY_MTP_TOKEN").ok());
    match miao_term_mtp::serve(&socket, state.clone()) {
        Ok(()) => eprintln!("miaotty: MTP host listening on {}", socket.display()),
        Err(e) => eprintln!("miaotty: failed to start MTP host: {e}"),
    }
    // Transparent windows go through a slower compositing path on macOS, which
    // adds input-to-photon latency. Only opt in when a translucent background
    // is actually configured.
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1180.0, 760.0])
        .with_title("miaotty");
    if cfg.background_opacity < 1.0 {
        viewport = viewport.with_transparent(true);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "miaotty",
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            egui_extras::install_image_loaders(&cc.egui_ctx);
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                rs.renderer
                    .write()
                    .callback_resources
                    .insert(PaneRenderers {
                        format: rs.target_format,
                        map: std::collections::HashMap::new(),
                    });
            }
            let mut app = MiaottyApp::new(state, cfg, cc.egui_ctx.clone());
            app.pending = intent;
            Ok(Box::new(app))
        }),
    )
}

/// Register the bundled Nerd Font (UI icons) and a system CJK fallback.
fn install_fonts(ctx: &egui::Context) {
    let cjk_candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
    ];

    let mut fonts = egui::FontDefinitions::default();
    // Bundled Nerd Font gives the UI access to icon glyphs (Otty-style).
    fonts.font_data.insert(
        "nerd".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/SymbolsNerdFontMono-Regular.ttf"
        ))),
    );
    let mut cjk_loaded = false;
    for path in cjk_candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "cjk".to_owned(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            cjk_loaded = true;
            break;
        }
    }
    for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
        let list = fonts.families.entry(family).or_default();
        if cjk_loaded {
            list.push("cjk".to_owned());
        }
        list.push("nerd".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Nerd Font glyphs (Font Awesome subset) used across the UI.
mod glyph {
    pub const INFO: &str = "\u{f129}";
    pub const AGENT: &str = "\u{f007}";
    pub const OUTLINE: &str = "\u{f03a}";
    pub const GIT: &str = "\u{f1d3}";
    pub const FILES: &str = "\u{f07b}";
    pub const PORTS: &str = "\u{f1e6}";
    pub const QUEUE: &str = "\u{f017}";
    pub const COG: &str = "\u{f013}";
    pub const PLUS: &str = "\u{f067}";
    pub const PANEL: &str = "\u{f0db}";
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
const FIND: egui::Color32 = egui::Color32::from_rgba_premultiplied(0xeb, 0xcb, 0x8b, 0x77);

/// Case-insensitive matches of `query_lower` per row, as `(row, start_col, end_col)`.
fn find_matches(screen: &ATerm, query_lower: &str) -> Vec<(u16, u16, u16)> {
    let mut out = Vec::new();
    if query_lower.is_empty() {
        return out;
    }
    let (rows, cols) = screen.size();
    let qlen = query_lower.chars().count().max(1);
    for row in 0..rows {
        // Lowercased row text plus a char-index → grid-column map, so matches on
        // rows containing wide (CJK) characters highlight the right columns.
        let mut line = String::new();
        let mut col_of: Vec<u16> = Vec::new();
        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                for lc in cell.ch.to_lowercase() {
                    line.push(lc);
                    col_of.push(col);
                }
            }
        }
        let mut from = 0;
        while let Some(pos) = line[from..].find(query_lower) {
            let start = from + pos;
            let end = start + qlen;
            let start_col = col_of.get(start).copied().unwrap_or(0);
            let end_col = col_of.get(end).map(|c| c + 1).unwrap_or(cols);
            out.push((row, start_col, end_col));
            from = end;
            if from >= line.len() {
                break;
            }
        }
    }
    out
}

fn indexed_palette(named: NamedColor) -> Option<usize> {
    use NamedColor::*;
    Some(match named {
        Black => 0,
        Red => 1,
        Green => 2,
        Yellow => 3,
        Blue => 4,
        Magenta => 5,
        Cyan => 6,
        White => 7,
        BrightBlack => 8,
        BrightRed => 9,
        BrightGreen => 10,
        BrightYellow => 11,
        BrightBlue => 12,
        BrightMagenta => 13,
        BrightCyan => 14,
        BrightWhite => 15,
        _ => return None,
    })
}

fn map_color(color: &Color, foreground: bool, theme: &Theme) -> egui::Color32 {
    match color {
        Color::Named(named) => {
            if let Some(i) = indexed_palette(*named) {
                theme.palette[i]
            } else if foreground {
                theme.fg
            } else {
                theme.bg
            }
        }
        Color::Indexed(i) => theme.palette[(*i as usize) & 0x0f],
        Color::Spec(rgb) => egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b),
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
                    layout_rects(
                        a,
                        egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + w, rect.max.y)),
                        out,
                    );
                    layout_rects(
                        b,
                        egui::Rect::from_min_max(egui::pos2(rect.left() + w, rect.top()), rect.max),
                        out,
                    );
                }
                SplitDir::Down => {
                    let h = rect.height() * r;
                    layout_rects(
                        a,
                        egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.top() + h)),
                        out,
                    );
                    layout_rects(
                        b,
                        egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + h), rect.max),
                        out,
                    );
                }
            }
        }
    }
}

/// A draggable divider between two panes.
struct SplitHandle {
    path: Vec<bool>,
    rect: egui::Rect,
    divider: egui::Rect,
    dir: SplitDir,
}

fn collect_splits(
    layout: &Layout,
    rect: egui::Rect,
    path: &mut Vec<bool>,
    out: &mut Vec<SplitHandle>,
) {
    if let Layout::Split { dir, ratio, a, b } = layout {
        let r = ratio.clamp(0.1, 0.9);
        match dir {
            SplitDir::Right => {
                let x = rect.left() + rect.width() * r;
                out.push(SplitHandle {
                    path: path.clone(),
                    rect,
                    divider: egui::Rect::from_min_max(
                        egui::pos2(x - 3.0, rect.top()),
                        egui::pos2(x + 3.0, rect.bottom()),
                    ),
                    dir: SplitDir::Right,
                });
                path.push(false);
                collect_splits(
                    a,
                    egui::Rect::from_min_max(rect.min, egui::pos2(x, rect.max.y)),
                    path,
                    out,
                );
                path.pop();
                path.push(true);
                collect_splits(
                    b,
                    egui::Rect::from_min_max(egui::pos2(x, rect.top()), rect.max),
                    path,
                    out,
                );
                path.pop();
            }
            SplitDir::Down => {
                let y = rect.top() + rect.height() * r;
                out.push(SplitHandle {
                    path: path.clone(),
                    rect,
                    divider: egui::Rect::from_min_max(
                        egui::pos2(rect.left(), y - 3.0),
                        egui::pos2(rect.right(), y + 3.0),
                    ),
                    dir: SplitDir::Down,
                });
                path.push(false);
                collect_splits(
                    a,
                    egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, y)),
                    path,
                    out,
                );
                path.pop();
                path.push(true);
                collect_splits(
                    b,
                    egui::Rect::from_min_max(egui::pos2(rect.left(), y), rect.max),
                    path,
                    out,
                );
                path.pop();
            }
        }
    }
}

fn set_ratio(layout: &mut Layout, path: &[bool], ratio: f32) {
    let Some((&first, rest)) = path.split_first() else {
        if let Layout::Split { ratio: r, .. } = layout {
            *r = ratio;
        }
        return;
    };
    if let Layout::Split { a, b, .. } = layout {
        if first {
            set_ratio(b, rest, ratio);
        } else {
            set_ratio(a, rest, ratio);
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
    /// Short label shown before the title.
    prefix: Option<String>,
    /// Mark shown after the title.
    mark: Option<String>,
    /// Tabs sharing a group are drawn together, separated from other groups.
    group: Option<String>,
}

/// Which editable text field of a tab the rename dialog is for (U1).
#[derive(Clone, Copy)]
enum TabField {
    Title,
    Prefix,
    Mark,
    Group,
}

// ---- session persistence ----

#[derive(Serialize, Deserialize)]
struct Session {
    tabs: Vec<TabSession>,
    active: usize,
    #[serde(default)]
    recent_files: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct TabSession {
    title: String,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    mark: Option<String>,
    #[serde(default)]
    group: Option<String>,
    panes: Vec<PaneSession>,
    layout: LayoutNode,
    active: usize,
}

#[derive(Serialize, Deserialize, Default)]
struct PaneSession {
    cwd: Option<String>,
}

#[derive(Serialize, Deserialize)]
enum LayoutNode {
    Leaf(usize),
    Split {
        down: bool,
        ratio: f32,
        a: Box<LayoutNode>,
        b: Box<LayoutNode>,
    },
}

/// The inbox directory for cross-instance launches.
fn inbox_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("miaotty").join("inbox"))
}

/// Hand a launch to an already-running instance (single-instance deep link).
/// Returns true when one was reached.
fn forward_to_running(request: &str) -> bool {
    let socket = miao_term_mtp::default_socket();
    if miao_term_mtp::client::connect(&socket).is_err() {
        return false;
    }
    if let Some(dir) = inbox_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let _ = std::fs::write(dir.join(format!("{stamp}.request")), request);
    }
    true
}

/// Consume forwarding requests written by later launches. `Some(cmd)` opens a
/// tab; `None` is a bare activation.
fn drain_inbox() -> Vec<String> {
    let mut out = Vec::new();
    let Some(dir) = inbox_dir() else {
        return out;
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return out;
    };
    let mut paths: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("request"))
        .collect();
    paths.sort();
    for path in paths {
        if let Ok(text) = std::fs::read_to_string(&path) {
            out.push(text.trim().to_string());
        }
        let _ = std::fs::remove_file(&path);
    }
    out
}

fn session_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("miaotty").join("session.json"))
}

impl Session {
    fn load() -> Option<Self> {
        let path = session_path()?;
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    fn save(&self) {
        let Some(path) = session_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(data) = serde_json::to_string(self) {
            let _ = std::fs::write(path, data);
        }
    }
}

fn layout_to_node(layout: &Layout, index_of: &dyn Fn(&str) -> usize) -> LayoutNode {
    match layout {
        Layout::Leaf(id) => LayoutNode::Leaf(index_of(id)),
        Layout::Split { dir, ratio, a, b } => LayoutNode::Split {
            down: *dir == SplitDir::Down,
            ratio: *ratio,
            a: Box::new(layout_to_node(a, index_of)),
            b: Box::new(layout_to_node(b, index_of)),
        },
    }
}

fn node_to_layout(node: &LayoutNode, ids: &[String]) -> Layout {
    match node {
        LayoutNode::Leaf(i) => Layout::Leaf(ids.get(*i).cloned().unwrap_or_default()),
        LayoutNode::Split { down, ratio, a, b } => Layout::Split {
            dir: if *down {
                SplitDir::Down
            } else {
                SplitDir::Right
            },
            ratio: *ratio,
            a: Box::new(node_to_layout(a, ids)),
            b: Box::new(node_to_layout(b, ids)),
        },
    }
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
    format!(
        "{:016x}{:04x}",
        nanos,
        COUNTER.fetch_add(1, Ordering::SeqCst)
    )
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
    show_sidebar: bool,
    state: Arc<miao_term_mtp::ServerState>,
    /// Wakes the UI from the PTY reader threads so output is drawn promptly.
    waker: Arc<dyn Fn() + Send + Sync>,
    theme: Theme,
    metrics: miao_term_render::MetricsProbe,
    renaming: Option<(usize, TabField)>,
    rename_buf: String,
    last_title: Option<String>,
    find_open: bool,
    find_query: String,
    split_drag: Option<Vec<bool>>,
    font_family: Option<String>,
    show_settings: bool,
    settings_family: String,
    line_height_ratio: f32,
    cursor_style: miao_term_config::CursorStyle,
    tab_drag: Option<usize>,
    opacity: f32,
    alpha: u8,
    rules: miao_term_config::view::RuleSet,
    view_edit: Option<usize>,
    palette: Option<Palette>,
    palette_focus: bool,
    panels: panels::PanelsWorker,
    panels_last: panels::Request,
    panels_at: Instant,
    search_at: Instant,
    recent_files: Vec<PathBuf>,
    details_tab: DetailsTab,
    file_sel: Option<usize>,
    tree_expanded: std::collections::HashSet<PathBuf>,
    editor: Option<Editor>,
    notifications: bool,
    prevent_sleep: bool,
    badges: miao_term_config::Badges,
    attention: std::collections::HashSet<String>,
    sleep: agentloop::SleepGuard,
    agent_states: std::collections::HashMap<String, String>,
    queue: Vec<QueuedPrompt>,
    composer: Option<Composer>,
    recipe_ui: Option<RecipeDialog>,
    quick_pane: Option<String>,
    quick_return: Option<usize>,
    ssh_ui: Option<SshDialog>,
    integration_msg: Option<String>,
    external_editor: Option<String>,
    pending: launch::Intent,
    pending_copy: Option<String>,
    quick_hotkey: Option<String>,
    remote_ui: Option<RemoteDialog>,
    remote_rx: Option<(bool, std::sync::mpsc::Receiver<RemoteRead>)>,
    update_manifest: Option<update::Manifest>,
    update_ready: Option<PathBuf>,
    update_pubkey: Option<String>,
    hotkeys: Option<hotkey::Hotkeys>,
    hotkey_tried: bool,
    lang: i18n::Lang,
    update_config: Option<String>,
    update_rx: Option<std::sync::mpsc::Receiver<UpdateEvent>>,
    update_msg: Option<String>,
    /// In-progress IME composition (e.g. Pinyin), shown inline near the cursor.
    ime_preedit: String,
    /// Whether to enable the OS input method at all. Can be turned off with
    /// `MIAOTTY_IME=0` to isolate IME-related input latency.
    ime_enabled: bool,
    /// Whether IME has been enabled on the window (avoid re-sending every frame,
    /// which resets the input method and makes it flicker).
    ime_allowed: bool,
    /// Last IME candidate-window rect sent, so we only send it when it moves.
    ime_rect: Option<egui::Rect>,
    /// Sidebar session search filter.
    tab_filter: String,
}

/// The save/open recipe dialog (U7).
struct RecipeDialog {
    save: bool,
    name: String,
    list: Vec<String>,
}

/// The "New SSH Session" dialog (M4/ADR 0014).
struct SshDialog {
    target: String,
}

/// A prompt waiting to be sent to an agent pane once it is idle.
struct QueuedPrompt {
    pane_id: String,
    text: String,
}

/// The multi-line prompt composer for one pane.
struct Composer {
    pane_id: String,
    text: String,
}

/// Details panel tab (ADR 0009).
#[derive(Clone, Copy, PartialEq)]
enum DetailsTab {
    Info,
    Agent,
    Outline,
    Git,
    Files,
    Ports,
    Queue,
}

/// An open file in the preview/editor window.
struct Editor {
    path: PathBuf,
    text: String,
    original: String,
    readonly: bool,
    /// Show raw text even for Markdown (preview is the default when read-only).
    raw: bool,
    /// A line to highlight and scroll to on open (1-based).
    target: Option<usize>,
    /// Whether the initial scroll to `target` has happened.
    jumped: bool,
    /// When set, the file lives on `dest` and is read/written over ssh.
    remote: Option<RemoteRef>,
}

/// A file reached over ssh (ADR 0021).
#[derive(Clone)]
struct RemoteRef {
    dest: String,
    path: String,
}

/// Result of a threaded remote read: the reference plus its text.
type RemoteRead = Result<(RemoteRef, String), String>;

/// The "view/edit a remote file" dialog.
struct RemoteDialog {
    dest: String,
    path: String,
    edit: bool,
}

/// One rendered tab in the top tab bar: label, agent color, view icon,
/// attention flag and group.
type TabBarRow = (
    String,
    Option<egui::Color32>,
    Option<miao_term_config::view::Icon>,
    bool,
    Option<String>,
);

/// A command-palette action (Open Quickly, ADR 0008).
#[derive(Clone)]
enum PaletteAction {
    SwitchTab(usize),
    Run(Verb),
    OpenFile(PathBuf, Option<usize>),
}

#[derive(Clone, Copy, PartialEq)]
enum Verb {
    Composer,
    QuickTerminal,
    ViewRemote,
    EditRemote,
    DownloadUpdate,
    CopyPaneId,
    NewSsh,
    CheckUpdates,
    SaveRecipe,
    OpenRecipe,
    NewTab,
    SplitRight,
    SplitDown,
    CloseTab,
    ToggleDetails,
    ToggleSidebar,
    IncreaseFont,
    DecreaseFont,
    ResetFont,
    ClearScreen,
    CopyPath,
    RevealInFinder,
    Find,
    Settings,
    NextTab,
    PrevTab,
}

struct PaletteEntry {
    kind: String,
    label: String,
    icon: Option<miao_term_config::view::Icon>,
    action: PaletteAction,
}

struct Palette {
    query: String,
    selected: usize,
}

impl MiaottyApp {
    fn new(
        state: Arc<miao_term_mtp::ServerState>,
        cfg: miao_term_config::Config,
        ctx: egui::Context,
    ) -> Self {
        let opacity = cfg.background_opacity.clamp(0.1, 1.0);
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
        let mut app = Self {
            tabs: Vec::new(),
            active: 0,
            font_size: cfg.font_size,
            selection: None,
            scroll: 0,
            cursor_on: true,
            last_blink: Instant::now(),
            show_details: true,
            show_sidebar: true,
            state,
            waker,
            theme: Theme::from_config(&cfg.theme),
            metrics: miao_term_render::MetricsProbe::new(),
            renaming: None,
            rename_buf: String::new(),
            last_title: None,
            find_open: false,
            find_query: String::new(),
            split_drag: None,
            font_family: cfg.font_family.clone(),
            show_settings: false,
            settings_family: cfg.font_family.clone().unwrap_or_default(),
            line_height_ratio: cfg.line_height,
            cursor_style: cfg.cursor_style,
            tab_drag: None,
            opacity,
            alpha: (opacity * 255.0).round() as u8,
            rules: miao_term_config::view::RuleSet::load(),
            view_edit: None,
            palette: None,
            palette_focus: false,
            panels: panels::PanelsWorker::spawn(),
            panels_last: panels::Request {
                cwd: None,
                pid: None,
                search: None,
            },
            panels_at: Instant::now(),
            search_at: Instant::now(),
            recent_files: Vec::new(),
            details_tab: DetailsTab::Info,
            file_sel: None,
            tree_expanded: std::collections::HashSet::new(),
            editor: None,
            notifications: cfg.notifications,
            prevent_sleep: cfg.prevent_sleep,
            badges: cfg.badges,
            attention: std::collections::HashSet::new(),
            sleep: agentloop::SleepGuard::new(),
            agent_states: std::collections::HashMap::new(),
            queue: Vec::new(),
            composer: None,
            recipe_ui: None,
            quick_pane: None,
            quick_return: None,
            ssh_ui: None,
            integration_msg: None,
            external_editor: cfg.editor.clone(),
            pending: launch::Intent::Activate,
            pending_copy: None,
            quick_hotkey: cfg.quick_terminal_hotkey.clone(),
            remote_ui: None,
            remote_rx: None,
            update_manifest: None,
            update_ready: None,
            update_pubkey: cfg.update_pubkey.clone(),
            hotkeys: None,
            hotkey_tried: false,
            lang: i18n::Lang::resolve(cfg.language.as_deref()),
            update_config: cfg.update_check_url.clone(),
            update_rx: None,
            update_msg: None,
            ime_preedit: String::new(),
            ime_enabled: std::env::var("MIAOTTY_IME")
                .map(|v| v != "0")
                .unwrap_or(true),
            ime_allowed: false,
            ime_rect: None,
            tab_filter: String::new(),
        };
        if let Some(session) = Session::load() {
            app.restore(session);
        }
        if app.tabs.is_empty() {
            app.push_tab("shell".to_owned(), 100, 30, None);
        }
        app
    }

    fn snapshot(&self) -> Session {
        let tabs = self
            .tabs
            .iter()
            .map(|tab| {
                let index_of =
                    |id: &str| tab.panes.iter().position(|p| p.pane_id == id).unwrap_or(0);
                TabSession {
                    title: tab.title.clone(),
                    prefix: tab.prefix.clone(),
                    mark: tab.mark.clone(),
                    group: tab.group.clone(),
                    panes: tab
                        .panes
                        .iter()
                        .map(|p| PaneSession {
                            cwd: p.term.cwd().map(str::to_string),
                        })
                        .collect(),
                    layout: layout_to_node(&tab.layout, &index_of),
                    active: index_of(&tab.active),
                }
            })
            .collect();
        Session {
            tabs,
            active: self.active,
            recent_files: self
                .recent_files
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        }
    }

    fn restore(&mut self, session: Session) {
        self.tabs.clear();
        self.recent_files = session.recent_files.iter().map(PathBuf::from).collect();
        for ts in session.tabs {
            let mut panes = Vec::new();
            let mut ids = Vec::new();
            let mut ok = true;
            for ps in &ts.panes {
                let pane_id = gen_pane_id();
                let cwd = ps.cwd.clone().map(PathBuf::from);
                match Self::spawn_terminal(100, 30, &pane_id, cwd, self.waker.clone()) {
                    Some(term) => {
                        panes.push(Pane {
                            term,
                            rows: Arc::new(Vec::new()),
                            dirty: true,
                            pane_id: pane_id.clone(),
                        });
                        ids.push(pane_id);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok || panes.is_empty() {
                continue;
            }
            let layout = node_to_layout(&ts.layout, &ids);
            let active = ids
                .get(ts.active)
                .cloned()
                .unwrap_or_else(|| ids[0].clone());
            self.tabs.push(Tab {
                panes,
                layout,
                active,
                title: ts.title,
                prefix: ts.prefix,
                mark: ts.mark,
                group: ts.group,
            });
        }
        if !self.tabs.is_empty() {
            self.active = session.active.min(self.tabs.len() - 1);
            self.publish_panes();
        }
    }

    fn spawn_terminal(
        cols: u16,
        rows: u16,
        pane_id: &str,
        cwd: Option<PathBuf>,
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> Option<Terminal> {
        let env = vec![("MIAOTTY_PANE_ID".to_owned(), pane_id.to_owned())];
        match Terminal::new(None, cols, rows, 10_000, cwd, &env, waker) {
            Ok(term) => Some(term),
            Err(e) => {
                eprintln!("failed to spawn shell: {e}");
                None
            }
        }
    }

    fn push_tab(&mut self, title: String, cols: u16, rows: u16, cwd: Option<PathBuf>) {
        let pane_id = gen_pane_id();
        if let Some(term) = Self::spawn_terminal(cols, rows, &pane_id, cwd, self.waker.clone()) {
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
                prefix: None,
                mark: None,
                group: None,
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
        let (rows, cols) = self.tabs[i]
            .focused()
            .map(|p| p.term.size())
            .unwrap_or((30, 100));
        let cwd = self.tabs[i]
            .focused()
            .and_then(|p| p.term.cwd().map(PathBuf::from));
        let title = self.tabs[i].title.clone();
        self.push_tab(title, cols, rows, cwd);
    }

    /// Move focus to the next/previous pane in the active tab.
    fn focus_cycle(&mut self, forward: bool) {
        let tab = &mut self.tabs[self.active];
        if tab.panes.len() < 2 {
            return;
        }
        let mut order = Vec::new();
        layout_rects(
            &tab.layout,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, 1.0)),
            &mut order,
        );
        let ids: Vec<String> = order.into_iter().map(|(id, _)| id).collect();
        if ids.len() < 2 {
            return;
        }
        let idx = ids.iter().position(|id| id == &tab.active).unwrap_or(0);
        let next = if forward {
            (idx + 1) % ids.len()
        } else {
            (idx + ids.len() - 1) % ids.len()
        };
        tab.active = ids[next].clone();
        self.selection = None;
        self.scroll = 0;
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
        if let Some(term) = Self::spawn_terminal(cols, rows, &pane_id, cwd, self.waker.clone()) {
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

    fn rename_window(&mut self, ctx: &egui::Context, i: usize, field: TabField) {
        let mut open = true;
        let title = match field {
            TabField::Title => "Rename Tab",
            TabField::Prefix => "Tab Prefix",
            TabField::Mark => "Tab Mark",
            TabField::Group => "Tab Group",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let resp = ui.text_edit_singleline(&mut self.rename_buf);
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter || ui.button(self.t("OK")).clicked() {
                    let value = self.rename_buf.trim().to_string();
                    let value = (!value.is_empty()).then_some(value);
                    if let Some(tab) = self.tabs.get_mut(i) {
                        match field {
                            TabField::Title => {
                                if let Some(v) = value {
                                    tab.title = v;
                                }
                            }
                            TabField::Prefix => tab.prefix = value,
                            TabField::Mark => tab.mark = value,
                            TabField::Group => tab.group = value,
                        }
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

        // MTP `pane.focus` / `pane.close`.
        let commands = self.state.take_commands();
        if !commands.is_empty() {
            for command in commands {
                match command {
                    miao_term_mtp::Command::Focus(id) => {
                        for (ti, tab) in self.tabs.iter_mut().enumerate() {
                            if tab.panes.iter().any(|p| p.pane_id == id) {
                                tab.active = id.clone();
                                self.active = ti;
                                self.selection = None;
                                self.scroll = 0;
                                break;
                            }
                        }
                    }
                    miao_term_mtp::Command::View(path) => {
                        self.open_editor(PathBuf::from(path));
                    }
                    miao_term_mtp::Command::Edit(path) => {
                        self.open_editor(PathBuf::from(path));
                        if let Some(editor) = self.editor.as_mut() {
                            editor.readonly = false;
                        }
                    }
                    miao_term_mtp::Command::Close(id) => {
                        if let Some(ti) = self
                            .tabs
                            .iter()
                            .position(|t| t.panes.iter().any(|p| p.pane_id == id))
                        {
                            self.active = ti;
                            self.tabs[ti].active = id.clone();
                            self.close_active();
                        }
                    }
                }
            }
            ctx.request_repaint();
        }

        // Poll a pending update check / download.
        if let Some(rx) = &self.update_rx {
            let local = env!("CARGO_PKG_VERSION");
            match rx.try_recv() {
                Ok(UpdateEvent::Checked(Ok(manifest))) => {
                    self.update_msg = Some(if manifest.version.is_empty() {
                        "Update check: empty response".to_string()
                    } else if update::is_newer(&manifest.version, local) {
                        format!("Update available: {} (you have {local})", manifest.version)
                    } else {
                        format!("Up to date ({local})")
                    });
                    self.update_manifest = Some(manifest);
                    self.update_rx = None;
                }
                Ok(UpdateEvent::Checked(Err(e))) => {
                    self.update_msg = Some(format!("Update check failed: {e}"));
                    self.update_rx = None;
                }
                Ok(UpdateEvent::Downloaded(Ok(path))) => {
                    self.update_ready = PathBuf::from(&path).into();
                    self.update_msg = Some(format!("Saved {path}"));
                    self.update_rx = None;
                }
                Ok(UpdateEvent::Downloaded(Err(e))) => {
                    self.update_msg = Some(format!("Download failed: {e}"));
                    self.update_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.update_rx = None,
            }
        }

        // Register the system-wide Quick Terminal hotkey once (ADR 0019).
        if !self.hotkey_tried {
            self.hotkey_tried = true;
            if let Some(spec) = self.quick_hotkey.clone() {
                let ctx = ctx.clone();
                match hotkey::Hotkeys::register(&spec, move || ctx.request_repaint()) {
                    Some(hk) => {
                        eprintln!("miaotty: quick-terminal hotkey {spec} registered");
                        self.hotkeys = Some(hk);
                    }
                    None => eprintln!("miaotty: could not register hotkey {spec}"),
                }
            }
        }
        if self.hotkeys.as_ref().is_some_and(|h| h.take_pending()) {
            self.toggle_quick_terminal();
        }
        if let Some(text) = self.pending_copy.take() {
            ctx.copy_text(text);
        }

        // Launch intents: this process's argv, then forwarded ones (ADR 0019).
        let intent = std::mem::take(&mut self.pending);
        self.apply_intent(intent);
        for line in drain_inbox() {
            self.apply_intent(launch::Intent::decode(&line));
        }

        // Notifications, sleep guard and the prompt queue (ADR 0010).
        self.agent_loop(ctx);

        // Refresh details-panel data for the focused pane (ADR 0009).
        let (cwd, pid) = self
            .tabs
            .get(self.active)
            .and_then(|t| t.focused())
            .map(|p| (p.term.cwd().map(str::to_string), p.term.pid()))
            .unwrap_or((None, None));
        let search = self
            .palette
            .as_ref()
            .map(|p| p.query.trim().to_string())
            .filter(|q| q.starts_with('#') && q.len() >= 2)
            .map(|q| q[1..].to_string());
        let req = panels::Request { cwd, pid, search };
        let search_changed = req.search != self.panels_last.search;
        let debounce_ok = !search_changed || self.search_at.elapsed() >= Duration::from_millis(200);
        if (req != self.panels_last || self.panels_at.elapsed() >= Duration::from_millis(2500))
            && debounce_ok
        {
            if search_changed {
                self.search_at = Instant::now();
            }
            self.panels.request(req.clone());
            self.panels_last = req;
            self.panels_at = Instant::now();
        }

        // Window title from the focused pane's OSC 0/2 title.
        let pane = self.tabs.get(self.active).and_then(|t| t.focused());
        let title = pane
            .and_then(|p| self.view_for(p))
            .map(|v| v.title)
            .filter(|s| !s.is_empty())
            .or_else(|| pane.and_then(|p| p.term.title().map(str::to_string)));
        if title != self.last_title {
            self.last_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                title.unwrap_or_else(|| "miaotty".to_string()),
            ));
        }

        // Cursor blink (only when focused).
        if ctx.input(|i| i.focused) && self.last_blink.elapsed() >= Duration::from_millis(530) {
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
        // Keep a steady frame cadence so PTY output and input echo are drawn
        // promptly. Gating this on focus was unreliable and let frames fall back
        // to the blink interval (~530 ms), which felt like heavy input lag.
        ctx.request_repaint_after(Duration::from_millis(16));

        self.menu_bar(ctx);
        self.tab_bar(ctx);
        if self.find_open {
            self.find_bar(ctx);
        }
        if self.show_sidebar {
            self.sidebar(ctx);
        }
        if self.show_details {
            self.details_panel(ctx);
        }
        self.terminal_panel(ctx);
        if let Some((i, field)) = self.renaming {
            self.rename_window(ctx, i, field);
        }
        if self.show_settings {
            self.settings_window(ctx);
        }
        if self.ssh_ui.is_some() {
            self.ssh_window(ctx);
        }
        if let Some((edit, rx)) = &self.remote_rx {
            match rx.try_recv() {
                Ok(Ok((rref, text))) => {
                    let edit = *edit;
                    self.remote_rx = None;
                    self.editor = Some(Editor {
                        path: PathBuf::from(&rref.path),
                        original: text.clone(),
                        text,
                        readonly: !edit,
                        raw: false,
                        target: None,
                        jumped: false,
                        remote: Some(rref),
                    });
                }
                Ok(Err(e)) => {
                    self.integration_msg = Some(format!("remote read failed: {e}"));
                    self.remote_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.remote_rx = None,
            }
        }
        if self.remote_ui.is_some() {
            self.remote_window(ctx);
        }
        if self.recipe_ui.is_some() {
            self.recipe_window(ctx);
        }
        if self.composer.is_some() {
            self.composer_window(ctx);
        }
        if self.editor.is_some() {
            self.editor_window(ctx);
        }
        if self.palette.is_some() {
            self.palette_ui(ctx);
        }
    }
}

impl MiaottyApp {
    /// Entries for Open Quickly: tabs, agents and commands. Ranked by
    /// [`palette_score`].
    fn palette_entries(&self) -> Vec<PaletteEntry> {
        let mut out = Vec::new();
        for (i, tab) in self.tabs.iter().enumerate() {
            let view = tab.focused().and_then(|p| self.view_for(p));
            out.push(PaletteEntry {
                kind: "tab".to_string(),
                label: self.tab_title(tab),
                icon: view.as_ref().and_then(|v| v.icon.clone()),
                action: PaletteAction::SwitchTab(i),
            });
            if tab
                .focused()
                .is_some_and(|p| self.attention.contains(&p.pane_id))
            {
                out.push(PaletteEntry {
                    kind: "attention".to_string(),
                    label: format!("{} needs attention", self.tab_title(tab)),
                    icon: view.as_ref().and_then(|v| v.icon.clone()),
                    action: PaletteAction::SwitchTab(i),
                });
            }
            if let Some(agent) = tab
                .focused()
                .and_then(|p| self.state.agent_for(&p.pane_id))
                .and_then(|a| a.get("agent").and_then(|v| v.as_str()).map(str::to_string))
            {
                out.push(PaletteEntry {
                    kind: "agent".to_string(),
                    label: agent,
                    icon: view.and_then(|v| v.icon),
                    action: PaletteAction::SwitchTab(i),
                });
            }
        }
        for file in &self.panels.snapshot().files {
            if !file.is_dir {
                out.push(PaletteEntry {
                    kind: "file".to_string(),
                    label: file.name.clone(),
                    icon: None,
                    action: PaletteAction::OpenFile(file.path.clone(), None),
                });
            }
        }
        for hit in &self.panels.snapshot().hits {
            let name = hit
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push(PaletteEntry {
                kind: format!("{name}:{}", hit.line),
                label: hit.text.clone(),
                icon: None,
                action: PaletteAction::OpenFile(hit.path.clone(), Some(hit.line as usize)),
            });
        }
        for path in &self.recent_files {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            out.push(PaletteEntry {
                kind: "recent".to_string(),
                label: name,
                icon: None,
                action: PaletteAction::OpenFile(path.clone(), None),
            });
        }
        for (label, verb) in [
            ("Composer", Verb::Composer),
            ("Quick Terminal", Verb::QuickTerminal),
            ("View Remote File\u{2026}", Verb::ViewRemote),
            ("Edit Remote File\u{2026}", Verb::EditRemote),
            ("Download Update", Verb::DownloadUpdate),
            ("Copy Pane ID", Verb::CopyPaneId),
            ("New SSH Session\u{2026}", Verb::NewSsh),
            ("Check for Updates", Verb::CheckUpdates),
            ("Save Recipe\u{2026}", Verb::SaveRecipe),
            ("Open Recipe\u{2026}", Verb::OpenRecipe),
            ("New Tab", Verb::NewTab),
            ("Split Right", Verb::SplitRight),
            ("Split Down", Verb::SplitDown),
            ("Close Tab", Verb::CloseTab),
            ("Toggle Details Panel", Verb::ToggleDetails),
            ("Toggle Tabs Panel", Verb::ToggleSidebar),
            ("Increase Font Size", Verb::IncreaseFont),
            ("Decrease Font Size", Verb::DecreaseFont),
            ("Reset Font Size", Verb::ResetFont),
            ("Clear Screen", Verb::ClearScreen),
            ("Copy Path", Verb::CopyPath),
            ("Reveal in Finder", Verb::RevealInFinder),
            ("Find", Verb::Find),
            ("Settings", Verb::Settings),
            ("Next Tab", Verb::NextTab),
            ("Previous Tab", Verb::PrevTab),
        ] {
            out.push(PaletteEntry {
                kind: "command".to_string(),
                label: self.t(label).to_string(),
                icon: None,
                action: PaletteAction::Run(verb),
            });
        }
        out
    }

    fn run_palette(&mut self, action: PaletteAction) {
        match action {
            PaletteAction::SwitchTab(i) => {
                if i < self.tabs.len() {
                    self.active = i;
                    self.selection = None;
                    self.scroll = 0;
                    if let Some(p) = self.tabs[i].focused() {
                        self.attention.remove(&p.pane_id);
                    }
                }
            }
            PaletteAction::Run(verb) => match verb {
                Verb::Composer => self.open_composer(),
                Verb::QuickTerminal => self.toggle_quick_terminal(),
                Verb::DownloadUpdate => self.download_update(),
                Verb::ViewRemote | Verb::EditRemote => {
                    self.remote_ui = Some(RemoteDialog {
                        dest: String::new(),
                        path: String::new(),
                        edit: verb == Verb::EditRemote,
                    })
                }
                Verb::CopyPaneId => {
                    self.pending_copy = self
                        .tabs
                        .get(self.active)
                        .and_then(|t| t.focused())
                        .map(|p| p.pane_id.clone());
                }
                Verb::NewSsh => {
                    self.ssh_ui = Some(SshDialog {
                        target: String::new(),
                    })
                }
                Verb::CheckUpdates => self.check_updates(),
                Verb::SaveRecipe => {
                    self.recipe_ui = Some(RecipeDialog {
                        save: true,
                        name: String::new(),
                        list: Vec::new(),
                    })
                }
                Verb::OpenRecipe => {
                    self.recipe_ui = Some(RecipeDialog {
                        save: false,
                        name: String::new(),
                        list: list_recipes(),
                    })
                }
                Verb::NewTab => self.new_tab(),
                Verb::SplitRight => self.split_active(SplitDir::Right),
                Verb::SplitDown => self.split_active(SplitDir::Down),
                Verb::CloseTab => self.close_active(),
                Verb::ToggleDetails => self.show_details = !self.show_details,
                Verb::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
                Verb::IncreaseFont => self.font_size = (self.font_size + 1.0).min(40.0),
                Verb::DecreaseFont => self.font_size = (self.font_size - 1.0).max(6.0),
                Verb::ResetFont => self.font_size = 13.0,
                Verb::ClearScreen => {
                    let id = self
                        .tabs
                        .get(self.active)
                        .and_then(|t| t.focused())
                        .map(|p| p.pane_id.clone());
                    if let (Some(id), Some(tab)) = (id, self.tabs.get_mut(self.active)) {
                        if let Some(pane) = tab.panes.iter_mut().find(|p| p.pane_id == id) {
                            pane.term.write(b"\x0c");
                        }
                    }
                }
                Verb::CopyPath => {
                    let cwd = self
                        .tabs
                        .get(self.active)
                        .and_then(|t| t.focused())
                        .and_then(|p| p.term.cwd())
                        .map(str::to_string);
                    if let Some(cwd) = cwd {
                        self.pending_copy = Some(cwd);
                    }
                }
                Verb::RevealInFinder => {
                    let cwd = self
                        .tabs
                        .get(self.active)
                        .and_then(|t| t.focused())
                        .and_then(|p| p.term.cwd())
                        .map(str::to_string);
                    if let Some(cwd) = cwd {
                        reveal_in_finder(&cwd);
                    }
                }
                Verb::Find => self.find_open = true,
                Verb::Settings => {
                    self.show_settings = true;
                    self.settings_family = self.font_family.clone().unwrap_or_default();
                }
                Verb::NextTab => self.focus_cycle(true),
                Verb::PrevTab => self.focus_cycle(false),
            },
            PaletteAction::OpenFile(path, line) => self.open_editor_at(path, line),
        }
    }

    fn palette_ui(&mut self, ctx: &egui::Context) {
        let Some(mut pal) = self.palette.take() else {
            return;
        };
        let entries = self.palette_entries();
        let fg = self.theme.fg;
        let muted = egui::Color32::from_gray(120);

        let filtered = rank_entries(&entries, &pal.query);
        if pal.selected >= filtered.len() {
            pal.selected = filtered.len().saturating_sub(1);
        }

        let (mut nav, mut accept, mut cancel) = (0i32, false, false);
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key {
                    key, pressed: true, ..
                } = ev
                {
                    match key {
                        egui::Key::ArrowDown => nav += 1,
                        egui::Key::ArrowUp => nav -= 1,
                        egui::Key::Enter => accept = true,
                        egui::Key::Escape => cancel = true,
                        _ => {}
                    }
                }
            }
        });
        if cancel {
            self.palette = None;
            return;
        }
        if nav != 0 && !filtered.is_empty() {
            pal.selected = (pal.selected as i32 + nav).rem_euclid(filtered.len() as i32) as usize;
        }
        let accept_index = accept.then_some(pal.selected);

        let mut clicked: Option<usize> = None;
        egui::Window::new("quickopen")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, egui::Vec2::new(0.0, 90.0))
            .default_width(560.0)
            .show(ctx, |ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut pal.query)
                        .hint_text(self.t("Open quickly\u{2026}"))
                        .desired_width(f32::INFINITY),
                );
                if self.palette_focus {
                    edit.request_focus();
                    self.palette_focus = false;
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (rank, (_, idx)) in filtered.iter().enumerate() {
                            let entry = &entries[*idx];
                            ui.horizontal(|ui| {
                                if let Some(icon) = &entry.icon {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::Vec2::splat(14.0),
                                        egui::Sense::hover(),
                                    );
                                    icons::draw_view(ui.painter(), rect, icon, fg);
                                }
                                let text = egui::RichText::new(&entry.label).color(fg);
                                let resp = ui.selectable_label(rank == pal.selected, text);
                                if resp.hovered() {
                                    pal.selected = rank;
                                }
                                if resp.clicked() {
                                    clicked = Some(rank);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new(entry.kind.clone())
                                                .small()
                                                .color(muted),
                                        );
                                    },
                                );
                            });
                        }
                    });
            });

        if let Some(rank) = accept_index.or(clicked) {
            if let Some((_, idx)) = filtered.get(rank) {
                let action = entries[*idx].action.clone();
                self.run_palette(action);
                self.palette = None;
                return;
            }
        }
        self.palette = Some(pal);
    }
}

impl Drop for MiaottyApp {
    fn drop(&mut self) {
        self.snapshot().save();
    }
}

fn agent_color(state: &str) -> egui::Color32 {
    match state {
        "processing" => egui::Color32::from_rgb(0x81, 0xa1, 0xc1),
        "idle" => egui::Color32::from_rgb(0xa3, 0xbe, 0x8c),
        "awaiting" => egui::Color32::from_rgb(0xeb, 0xcb, 0x8b),
        "error" => egui::Color32::from_rgb(0xbf, 0x61, 0x6a),
        _ => egui::Color32::GRAY,
    }
}

fn section(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .size(11.0)
        .strong()
        .color(egui::Color32::from_gray(150))
}

/// Edit an optional string; an empty field clears it. Returns whether it
/// changed.
fn opt_text(ui: &mut egui::Ui, opt: &mut Option<String>) -> bool {
    let mut text = opt.clone().unwrap_or_default();
    let changed = ui
        .add(egui::TextEdit::singleline(&mut text).desired_width(150.0))
        .changed();
    if changed {
        let trimmed = text.trim();
        *opt = (!trimmed.is_empty()).then(|| trimmed.to_string());
    }
    changed
}

/// The command that resumes a reported agent session, if any. Prefers an
/// explicit `resume` field, else derives one from `session_id` and the agent.
fn resume_command(agent_state: &serde_json::Value, agent: &str) -> Option<String> {
    if let Some(resume) = agent_state.get("resume").and_then(|v| v.as_str()) {
        if !resume.trim().is_empty() {
            return Some(resume.to_string());
        }
    }
    let id = agent_state.get("session_id").and_then(|v| v.as_str())?;
    if id.trim().is_empty() {
        return None;
    }
    Some(match agent {
        "claude" => format!("claude --resume {id}"),
        "codex" => format!("codex resume {id}"),
        "miao" => format!("miao --resume {id}"),
        other => format!("{other} --resume {id}"),
    })
}

/// Rank palette entries for a query, returning `(rank, entry index)` sorted
/// best-first. Ties keep list order, so recent files stay ahead.
fn rank_entries(entries: &[PaletteEntry], query: &str) -> Vec<(usize, usize)> {
    let query = query.to_lowercase();
    let mut filtered: Vec<(usize, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| palette_score(&e.label, &e.kind, &query).map(|s| (s, i)))
        .collect();
    filtered.sort_by_key(|(s, i)| (*s, *i));
    filtered
}

/// Rank an entry against a lowercased query: substring hits rank by position,
/// subsequence hits after all substring hits, and an empty query matches all.
fn palette_score(label: &str, kind: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let hay = format!("{label} {kind}").to_lowercase();
    if let Some(pos) = hay.find(query) {
        return Some(pos);
    }
    let mut chars = hay.chars();
    for c in query.chars() {
        if !chars.any(|h| h == c) {
            return None;
        }
    }
    Some(1000)
}

fn match_summary(rule: &miao_term_config::view::Rule) -> String {
    let m = &rule.r#match;
    let mut parts = Vec::new();
    if let Some(p) = &m.path {
        parts.push(format!("path:{p}"));
    }
    if let Some(p) = &m.command {
        parts.push(format!("cmd:{p}"));
    }
    if let Some(p) = &m.agent {
        parts.push(format!("agent:{p}"));
    }
    if let Some(p) = &m.host {
        parts.push(format!("host:{p}"));
    }
    if let Some(p) = &m.file {
        parts.push(format!("file:{p}"));
    }
    if parts.is_empty() {
        parts.push("any".to_string());
    }
    parts.join(" ")
}

/// A sample context for the rule preview, seeded from the rule's match.
fn preview_context(m: &miao_term_config::view::Match) -> miao_term_config::view::Context {
    let cwd = m
        .path
        .as_deref()
        .map(sample_path)
        .unwrap_or_else(|| "/Users/me/project".to_string());
    miao_term_config::view::Context {
        cwd: Some(cwd),
        command: m
            .command
            .clone()
            .or_else(|| Some("cargo build".to_string())),
        agent: m.agent.clone().or_else(|| Some("claude".to_string())),
        host: m.host.clone(),
        file: m.file.clone(),
        user: Some("me".to_string()),
        shell: Some("/bin/zsh".to_string()),
        branch: Some("main".to_string()),
        osc_title: Some("zsh".to_string()),
        index: Some(0),
    }
}

fn sample_path(pattern: &str) -> String {
    let base = pattern
        .split('*')
        .next()
        .unwrap_or(pattern)
        .trim_end_matches('/');
    if base.is_empty() {
        "/tmp/demo".to_string()
    } else {
        format!("{base}/demo")
    }
}

/// Render a directory tree. Hidden entries are skipped, each directory shows at
/// most 200 entries, and recursion stops at depth 6 so a huge tree cannot stall
/// the UI. `.` is expanded via `expanded`; double-clicking a file opens it.
fn file_tree(
    ui: &mut egui::Ui,
    dir: &std::path::Path,
    depth: usize,
    expanded: &mut std::collections::HashSet<PathBuf>,
    open: &mut Option<PathBuf>,
    fg: egui::Color32,
) {
    const MAX_DEPTH: usize = 6;
    const MAX_ENTRIES: usize = 200;
    if depth > MAX_DEPTH {
        return;
    }
    let muted = egui::Color32::from_gray(150);
    for entry in panels::read_dir_entries(dir).iter().take(MAX_ENTRIES) {
        if entry.name.starts_with('.') {
            continue;
        }
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 10.0);
            if entry.is_dir {
                let is_open = expanded.contains(&entry.path);
                let glyph = if is_open { "\u{25be}" } else { "\u{25b8}" };
                let label = egui::RichText::new(format!("{glyph} {}", entry.name)).color(fg);
                if ui.selectable_label(false, label).clicked() {
                    if is_open {
                        expanded.remove(&entry.path);
                    } else {
                        expanded.insert(entry.path.clone());
                    }
                }
            } else {
                let label = egui::RichText::new(format!("  {}", entry.name)).color(muted);
                if ui.selectable_label(false, label).double_clicked() {
                    *open = Some(entry.path.clone());
                }
            }
        });
        if entry.is_dir && expanded.contains(&entry.path) {
            file_tree(ui, &entry.path, depth + 1, expanded, open, fg);
        }
    }
}

#[cfg(target_os = "macos")]
/// Render a small, dependency-free subset of Markdown: ATX headings, fenced
/// code, bullet lists, block quotes. Inline emphasis markers are stripped.
#[cfg(target_os = "macos")]
/// The `.app` bundle root containing `exe`, if it is inside one
/// (`…/X.app/Contents/MacOS/x` → `…/X.app`).
fn bundle_root(exe: &std::path::Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    if macos.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    (bundle.extension().and_then(|e| e.to_str()) == Some("app")).then(|| bundle.to_path_buf())
}

#[cfg(target_os = "macos")]
/// The shell script that swaps `bundle` for `new_app` once `pid` exits, then
/// relaunches. Restores the old bundle if the swap fails.
fn install_script(pid: u32, bundle: &std::path::Path, new_app: &std::path::Path) -> String {
    let bundle = bundle.display();
    let new_app = new_app.display();
    format!(
        "#!/bin/sh\n\
         # miaotty self-update helper\n\
         set -eu\n\
         while kill -0 {pid} 2>/dev/null; do sleep 0.3; done\n\
         old=\"{bundle}.old\"\n\
         rm -rf \"$old\"\n\
         mv \"{bundle}\" \"$old\"\n\
         if mv \"{new_app}\" \"{bundle}\"; then\n\
         \x20 open \"{bundle}\"\n\
         \x20 rm -rf \"$old\"\n\
         else\n\
         \x20 mv \"$old\" \"{bundle}\"\n\
         fi\n"
    )
}

#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
/// The helper `.cmd` Windows runs after we exit: wait for our PID, then either
/// run the MSI or unpack the zip over the current executable, and relaunch.
fn windows_install_script(pid: u32, artifact: &std::path::Path, exe: &std::path::Path) -> String {
    let artifact = artifact.display();
    let exe = exe.display();
    let dir = exe_parent(&exe.to_string());
    format!(
        "@echo off\r\n\
         :wait\r\n\
         tasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul && (timeout /t 1 /nobreak >nul & goto wait)\r\n\
         echo \"{artifact}\" | find /I \".msi\" >nul\r\n\
         if not errorlevel 1 (\r\n\
         \x20 msiexec /i \"{artifact}\" /passive /norestart\r\n\
         \x20 start \"\" \"%ProgramFiles%\\miaotty\\bin\\miaotty.exe\"\r\n\
         ) else (\r\n\
         \x20 rmdir /s /q \"%TEMP%\\miaotty-update\" 2>nul\r\n\
         \x20 mkdir \"%TEMP%\\miaotty-update\" 2>nul\r\n\
         \x20 tar -xf \"{artifact}\" -C \"%TEMP%\\miaotty-update\"\r\n\
         \x20 copy /y \"%TEMP%\\miaotty-update\\miaotty.exe\" \"{exe}\" >nul\r\n\
         \x20 if exist \"%TEMP%\\miaotty-update\\miaotty-cli.exe\" copy /y \"%TEMP%\\miaotty-update\\miaotty-cli.exe\" \"{dir}\\miaotty-cli.exe\" >nul\r\n\
         \x20 start \"\" \"{exe}\"\r\n\
         )\r\n"
    )
}

#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
/// The directory part of a Windows path (either separator).
fn exe_parent(path: &str) -> String {
    match path.rfind(['\\', '/']) {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}

#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
/// The helper `sh` Linux runs after we exit: wait for our PID, replace the
/// running AppImage and start it again.
fn linux_install_script(
    pid: u32,
    appimage: &std::path::Path,
    downloaded: &std::path::Path,
) -> String {
    format!(
        "#!/bin/sh\n\
         while kill -0 {pid} 2>/dev/null; do sleep 0.3; done\n\
         cp \"{downloaded}\" \"{appimage}\" && chmod +x \"{appimage}\" && exec \"{appimage}\"\n",
        pid = pid,
        appimage = appimage.display(),
        downloaded = downloaded.display()
    )
}

#[cfg(target_os = "macos")]
/// The first `.app` bundle anywhere under `dir`.
fn find_app(dir: &std::path::Path) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("app") {
                return Some(path);
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    None
}

/// Open a path with the OS default application.
fn open_path_externally(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .spawn();
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = path;
    }
}

/// A read-only, line-numbered source view that highlights `target`.
fn source_ui(
    ui: &mut egui::Ui,
    text: &str,
    target: Option<usize>,
    jumped: &mut bool,
    fg: egui::Color32,
) {
    let muted = egui::Color32::from_gray(120);
    let width = text.lines().count().to_string().len().max(2);
    let highlight = egui::Color32::from_rgb(0x3b, 0x42, 0x52);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, line) in text.lines().enumerate() {
                let n = i + 1;
                let is_target = target == Some(n);
                let mut frame = egui::Frame::none().inner_margin(egui::Margin::symmetric(2.0, 0.0));
                if is_target {
                    frame = frame.fill(highlight);
                }
                let row = frame
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{n:>width$}"))
                                    .monospace()
                                    .size(11.0)
                                    .color(muted),
                            );
                            let body = if line.is_empty() { " " } else { line };
                            ui.label(egui::RichText::new(body).monospace().size(12.0).color(fg));
                        });
                    })
                    .response;
                if is_target && !*jumped {
                    ui.scroll_to_rect(row.rect, Some(egui::Align::Center));
                    *jumped = true;
                }
            }
        });
}

/// Whether a line is a Markdown horizontal rule (`---`, `***`, `___`).
fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3
        && t.chars().all(|c| c == t.chars().next().unwrap())
        && matches!(t.chars().next(), Some('-' | '*' | '_'))
}

/// If a line is exactly `![alt](url)`, return `(alt, url)`.
fn parse_image(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("![")?;
    let (alt, rest) = rest.split_once("](")?;
    let url = rest.strip_suffix(')')?;
    Some((alt, url))
}

/// Resolve a local image reference to a `file://` URI, or `None` for remote
/// URLs / unknown extensions. Relative paths resolve against `base` (the
/// document's directory).
fn image_uri(url: &str, base: Option<&std::path::Path>) -> Option<String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return None;
    }
    let path = std::path::Path::new(url);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base?.join(path)
    };
    let ext = abs.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
    ) {
        return None;
    }
    // Normalize to a file URL: forward slashes, and a leading slash before a
    // Windows drive so `C:\dir\x.png` becomes `file:///C:/dir/x.png`.
    let mut path = abs.display().to_string().replace('\\', "/");
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    Some(format!("file://{path}"))
}

/// Pull `[^id]: text` footnote definitions out of the flow.
fn take_footnotes<'a>(lines: &[&'a str]) -> (Vec<&'a str>, Vec<(String, String)>) {
    let mut body = Vec::new();
    let mut defs = Vec::new();
    for line in lines {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("[^") {
            if let Some((id, text)) = rest.split_once("]: ") {
                defs.push((id.trim().to_string(), text.trim().to_string()));
                continue;
            }
        }
        body.push(*line);
    }
    (body, defs)
}

/// Rewrite inline footnote references `[^id]` to a plain `[id]` marker.
fn footnote_refs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("[^") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find(']') {
            Some(j) => {
                out.push('[');
                out.push_str(&after[..j]);
                out.push(']');
                rest = &after[j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Line numbers for a sequence of layout rows (increments after a newline row).
fn gutter_numbers(ends_with_newline: &[bool]) -> Vec<usize> {
    let mut numbers = Vec::with_capacity(ends_with_newline.len());
    let mut line = 1;
    for ends in ends_with_newline {
        numbers.push(line);
        if *ends {
            line += 1;
        }
    }
    numbers
}

/// Paint the editing gutter numbers beside a `TextEdit`.
fn paint_gutter(
    ui: &egui::Ui,
    gutter: egui::Rect,
    output: &egui::text_edit::TextEditOutput,
    color: egui::Color32,
) {
    let clip = output.text_clip_rect;
    let painter = ui.painter();
    let font = egui::FontId::monospace(11.0);
    let flags: Vec<bool> = output
        .galley
        .rows
        .iter()
        .map(|r| r.ends_with_newline)
        .collect();
    let numbers = gutter_numbers(&flags);
    for (row, number) in output.galley.rows.iter().zip(numbers) {
        let y = output.galley_pos.y + row.rect.top();
        if y + row.rect.height() >= clip.top() && y <= clip.bottom() {
            painter.text(
                egui::pos2(gutter.right() - 3.0, y),
                egui::Align2::RIGHT_TOP,
                number.to_string(),
                font.clone(),
                color,
            );
        }
    }
}

/// Split a line into text and `[text](url)` link segments.
fn link_segments(line: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        if open > 0 {
            out.push((rest[..open].to_string(), None));
        }
        let Some(close) = rest[open..].find(']') else {
            out.push((rest[open..].to_string(), None));
            return out;
        };
        let label_end = open + close;
        let after = &rest[label_end + 1..];
        if let Some(inner) = after.strip_prefix('(') {
            if let Some(end) = inner.find(')') {
                out.push((
                    rest[open + 1..label_end].to_string(),
                    Some(inner[..end].to_string()),
                ));
                rest = &inner[end + 1..];
                continue;
            }
        }
        out.push((rest[open..=label_end].to_string(), None));
        rest = &rest[label_end + 1..];
    }
    if !rest.is_empty() {
        out.push((rest.to_string(), None));
    }
    out
}

fn markdown_ui(
    ui: &mut egui::Ui,
    text: &str,
    fg: egui::Color32,
    base: Option<&std::path::Path>,
    lang: i18n::Lang,
) {
    let muted = egui::Color32::from_gray(150);
    let all: Vec<&str> = text.lines().collect();
    let (lines, footnotes) = take_footnotes(&all);
    let mut grid = 0usize;
    let mut i = 0;
    let mut in_code = false;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            let info = trimmed.trim_start_matches('`').trim().to_ascii_lowercase();
            in_code = !in_code;
            if in_code && info == "mermaid" {
                // Mermaid layout is out of scope; say so instead of pretending.
                ui.label(
                    egui::RichText::new(i18n::t(lang, "Mermaid diagram (not rendered)"))
                        .small()
                        .color(muted),
                );
            }
            ui.label(egui::RichText::new(line).monospace().color(muted));
            i += 1;
            continue;
        }
        if in_code {
            ui.label(egui::RichText::new(line).monospace().color(fg));
            i += 1;
            continue;
        }
        if let Some((alt, url)) = parse_image(line.trim()) {
            match image_uri(url, base) {
                Some(uri) => {
                    ui.add(
                        egui::Image::new(uri)
                            .max_width(ui.available_width())
                            .max_height(400.0),
                    );
                }
                None => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(format!("[{alt}]")).color(muted));
                        ui.hyperlink_to(url, url);
                    });
                }
            }
            i += 1;
            continue;
        }
        if line.contains('|') && i + 1 < lines.len() && is_table_sep(lines[i + 1]) {
            let header = split_row(line);
            let aligns = table_align(lines[i + 1]);
            i += 2;
            let mut rows = Vec::new();
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                rows.push(split_row(lines[i]));
                i += 1;
            }
            render_table(ui, grid, &header, &rows, &aligns, fg);
            grid += 1;
            continue;
        }
        if is_rule(line) {
            ui.add_space(3.0);
            ui.separator();
        } else if let Some((level, rest)) = heading(line) {
            let size = match level {
                1 => 22.0,
                2 => 18.0,
                3 => 16.0,
                _ => 14.0,
            };
            ui.add_space(if level <= 2 { 6.0 } else { 3.0 });
            ui.label(
                egui::RichText::new(strip_inline(&footnote_refs(rest)))
                    .size(size)
                    .strong()
                    .color(fg),
            );
        } else if let Some(rest) = bullet(line) {
            ui.label(
                egui::RichText::new(format!("\u{2022} {}", strip_inline(&footnote_refs(rest))))
                    .color(fg),
            );
        } else if let Some(rest) = trimmed.strip_prefix('>') {
            ui.label(
                egui::RichText::new(strip_inline(rest.trim_start()))
                    .italics()
                    .color(muted),
            );
        } else if line.trim().is_empty() {
            ui.add_space(4.0);
        } else {
            let line = footnote_refs(line);
            let segments = link_segments(&line);
            if segments.iter().any(|(_, url)| url.is_some()) {
                ui.horizontal_wrapped(|ui| {
                    for (text, url) in &segments {
                        match url {
                            Some(url) => {
                                ui.hyperlink_to(strip_inline(text), url);
                            }
                            None => {
                                ui.label(egui::RichText::new(strip_inline(text)).color(fg));
                            }
                        }
                    }
                });
            } else {
                ui.label(egui::RichText::new(strip_inline(&line)).color(fg));
            }
        }
        i += 1;
    }
    if !footnotes.is_empty() {
        ui.add_space(6.0);
        ui.separator();
        ui.label(egui::RichText::new("Footnotes").strong().color(fg));
        for (id, text) in &footnotes {
            ui.label(
                egui::RichText::new(format!("[{id}] {text}"))
                    .small()
                    .color(muted),
            );
        }
    }
}

/// Whether a line is a Markdown table separator (`---|:--:|---`).
fn is_table_sep(line: &str) -> bool {
    let t = line.trim();
    t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Split a table row on `|`, trimming the outer pipes and whitespace.
fn split_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

/// Per-column alignment from a separator row.
fn table_align(sep: &str) -> Vec<egui::Align> {
    split_row(sep)
        .iter()
        .map(|c| {
            let left = c.starts_with(':');
            let right = c.ends_with(':');
            match (left, right) {
                (true, true) => egui::Align::Center,
                (false, true) => egui::Align::RIGHT,
                _ => egui::Align::LEFT,
            }
        })
        .collect()
}

fn render_table(
    ui: &mut egui::Ui,
    index: usize,
    header: &[String],
    rows: &[Vec<String>],
    aligns: &[egui::Align],
    fg: egui::Color32,
) {
    let align_of = |c: usize| aligns.get(c).copied().unwrap_or(egui::Align::LEFT);
    egui::Grid::new(format!("mdtable{index}"))
        .striped(true)
        .spacing([14.0, 3.0])
        .show(ui, |ui| {
            for (c, cell) in header.iter().enumerate() {
                align_cell(
                    ui,
                    egui::RichText::new(strip_inline(cell)).strong().color(fg),
                    align_of(c),
                );
            }
            ui.end_row();
            for row in rows {
                for (c, cell) in row.iter().enumerate() {
                    align_cell(
                        ui,
                        egui::RichText::new(strip_inline(cell)).color(fg),
                        align_of(c),
                    );
                }
                ui.end_row();
            }
        });
}

fn align_cell(ui: &mut egui::Ui, text: egui::RichText, align: egui::Align) {
    match align {
        egui::Align::RIGHT => {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(text);
            });
        }
        egui::Align::Center => {
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.label(text);
            });
        }
        _ => {
            ui.label(text);
        }
    }
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && line.chars().nth(hashes) == Some(' ') {
        Some((hashes, line[hashes..].trim_start()))
    } else {
        None
    }
}

fn bullet(line: &str) -> Option<&str> {
    let t = line.trim_start();
    t.strip_prefix("- ").or_else(|| t.strip_prefix("* "))
}

/// Strip inline emphasis/code markers, returning plain text.
fn strip_inline(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '`' | '*' | '_'))
        .collect()
}

/// Run an update check on a thread: fetch `url` with curl, compare the first
/// token to the running version, and report a human string.
/// An update-related result delivered from a worker thread.
enum UpdateEvent {
    Checked(Result<update::Manifest, String>),
    Downloaded(Result<String, String>),
}

/// Fetch the manifest at `url` (bounded) and parse it.
fn spawn_update_check(url: String) -> std::sync::mpsc::Receiver<UpdateEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = http_get(&url).map(|body| update::parse(&body));
        let _ = tx.send(UpdateEvent::Checked(result));
    });
    rx
}

/// Download `url` and verify its SHA-256 before moving it to `~/Downloads`.
fn spawn_update_download(
    url: String,
    sha256: String,
    signature: Option<String>,
    pubkey: Option<String>,
) -> std::sync::mpsc::Receiver<UpdateEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(UpdateEvent::Downloaded(download_and_verify(
            &url,
            &sha256,
            signature.as_deref(),
            pubkey.as_deref(),
        )));
    });
    rx
}

fn http_get(url: &str) -> Result<String, String> {
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "8", url])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("curl failed: {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn download_and_verify(
    url: &str,
    sha256: &str,
    signature: Option<&str>,
    pubkey: Option<&str>,
) -> Result<String, String> {
    let name = url
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("miaotty-download")
        .to_string();
    let tmp = std::env::temp_dir().join(format!("miaotty-dl-{name}"));
    let out = std::process::Command::new("curl")
        .args(["-fL", "--max-time", "600", "-o"])
        .arg(&tmp)
        .arg(url)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("download failed: {}", out.status));
    }
    if !sha256.is_empty() {
        match update::verify_file(&tmp, sha256) {
            Ok(true) => {}
            Ok(false) => {
                let _ = std::fs::remove_file(&tmp);
                return Err("checksum mismatch".to_string());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    let target = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join("Downloads"))
        .filter(|d| d.is_dir())
        .map(|d| d.join(&name))
        .unwrap_or_else(|| tmp.clone());
    if target != tmp && std::fs::rename(&tmp, &target).is_err() {
        std::fs::copy(&tmp, &target).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&tmp);
    }
    // Optional minisign signature check (requires the `minisign` tool).
    let mut signature_note = "";
    if let (Some(sig_url), Some(key)) = (signature, pubkey) {
        let sig_path = tmp.with_extension("sig");
        let out = std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "30", "-o"])
            .arg(&sig_path)
            .arg(sig_url)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            let _ = std::fs::remove_file(&target);
            return Err("could not fetch the signature".to_string());
        }
        let verify = std::process::Command::new("minisign")
            .args(["-V", "-P", key, "-x"])
            .arg(&sig_path)
            .arg("-m")
            .arg(&target)
            .output();
        let _ = std::fs::remove_file(&sig_path);
        match verify {
            Ok(out) if out.status.success() => signature_note = ", signature ok",
            Ok(out) => {
                let _ = std::fs::remove_file(&target);
                let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err(format!("signature invalid: {msg}"));
            }
            Err(_) => signature_note = ", signature not checked (minisign not found)",
        }
    }
    let verified = if sha256.is_empty() {
        "unverified"
    } else {
        "checksum ok"
    };
    Ok(format!("{}{verified}{signature_note}", target.display()))
}

/// `~/.config/miaotty/recipes` (respecting `XDG_CONFIG_HOME`).
fn recipes_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("miaotty").join("recipes"))
}

fn list_recipes() -> Vec<String> {
    let Some(dir) = recipes_dir() else {
        return Vec::new();
    };
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            (path.extension().and_then(|x| x.to_str()) == Some("json"))
                .then(|| path.file_stem().map(|s| s.to_string_lossy().to_string()))
                .flatten()
        })
        .collect();
    names.sort();
    names
}

fn save_recipe(name: &str, session: &Session) -> std::io::Result<PathBuf> {
    let dir = recipes_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no config directory"))?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", sanitize_name(name)));
    let json = serde_json::to_string_pretty(session).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;
    Ok(path)
}

fn load_recipe(name: &str) -> Option<Session> {
    let path = recipes_dir()?.join(format!("{}.json", sanitize_name(name)));
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Keep recipe names to a safe filename charset.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn reveal_in_finder(path: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn();
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
        let rows: Vec<(
            String,
            Option<egui::Color32>,
            Option<miao_term_config::view::Icon>,
            bool,
        )> = self
            .tabs
            .iter()
            .map(|tab| {
                let title = self.tab_title(tab);
                let view = tab.focused().and_then(|p| self.view_for(p));
                let att = tab
                    .focused()
                    .is_some_and(|p| self.attention.contains(&p.pane_id));
                let color = tab
                    .focused()
                    .and_then(|p| self.state.agent_for(&p.pane_id))
                    .and_then(|a| a.get("state").and_then(|v| v.as_str()).map(str::to_string))
                    .filter(|s| self.badges.enabled(s))
                    .map(|s| agent_color(&s));
                (title, color, view.and_then(|v| v.icon), att)
            })
            .collect();
        let active = self.active;
        let fg = self.theme.fg;
        let tree_root = self
            .tabs
            .get(self.active)
            .and_then(|t| t.focused())
            .and_then(|p| p.term.cwd().map(str::to_string));
        let can_close = self.tabs.len() > 1;
        let mut tree_open: Option<PathBuf> = None;
        let mut switch_to: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut close_others: Option<usize> = None;
        let mut duplicate: Option<usize> = None;
        let mut edit: Option<(usize, TabField, String)> = None;
        let mut ungroup: Option<usize> = None;
        let mut add = false;
        let mut toggle = false;
        let mut settings = false;

        egui::SidePanel::left("tabs")
            .resizable(true)
            .default_width(190.0)
            .frame(
                egui::Frame::default()
                    .fill(self.bg())
                    .inner_margin(egui::Margin::same(6.0)),
            )
            .show(ctx, |ui| {
                ui.visuals_mut().selection.bg_fill = self.theme.palette[4];
                ui.visuals_mut().selection.stroke = egui::Stroke::NONE;
                ui.horizontal(|ui| {
                    ui.label(section(&format!("{} ({})", self.t("TABS"), rows.len())));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(glyph::PLUS).on_hover_text("New Tab").clicked() {
                            add = true;
                        }
                        if ui
                            .button(glyph::PANEL)
                            .on_hover_text("Toggle Details Panel")
                            .clicked()
                        {
                            toggle = true;
                        }
                        if ui.button(glyph::COG).on_hover_text("Settings").clicked() {
                            settings = true;
                        }
                    });
                });
                let l_search = self.t("Search sessions…");
                ui.add(
                    egui::TextEdit::singleline(&mut self.tab_filter)
                        .hint_text(l_search)
                        .desired_width(f32::INFINITY)
                        .margin(egui::Margin::symmetric(6.0, 3.0)),
                );
                ui.add_space(4.0);
                let needle = self.tab_filter.to_lowercase();
                for (i, (title, color, icon, att)) in rows.iter().enumerate() {
                    if !needle.is_empty() && !title.to_lowercase().contains(&needle) {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        if *att {
                            ui.colored_label(egui::Color32::from_rgb(0xeb, 0xcb, 0x8b), "\u{0021}");
                        }
                        if let Some(icon) = icon {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::Vec2::splat(13.0), egui::Sense::hover());
                            icons::draw_view(ui.painter(), rect, icon, fg);
                        }
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
                        let fields = (
                            self.tabs[i].title.clone(),
                            self.tabs[i].prefix.clone(),
                            self.tabs[i].mark.clone(),
                            self.tabs[i].group.clone(),
                        );
                        resp.context_menu(|ui| {
                            if ui.button(self.t("Rename\u{2026}")).clicked() {
                                edit = Some((i, TabField::Title, fields.0.clone()));
                                ui.close_menu();
                            }
                            if ui.button(self.t("Prefix\u{2026}")).clicked() {
                                edit = Some((
                                    i,
                                    TabField::Prefix,
                                    fields.1.clone().unwrap_or_default(),
                                ));
                                ui.close_menu();
                            }
                            if ui.button(self.t("Mark\u{2026}")).clicked() {
                                edit =
                                    Some((i, TabField::Mark, fields.2.clone().unwrap_or_default()));
                                ui.close_menu();
                            }
                            if ui.button(self.t("Group\u{2026}")).clicked() {
                                edit = Some((
                                    i,
                                    TabField::Group,
                                    fields.3.clone().unwrap_or_default(),
                                ));
                                ui.close_menu();
                            }
                            if fields.3.is_some()
                                && ui.button(self.t("Remove from Group")).clicked()
                            {
                                ungroup = Some(i);
                                ui.close_menu();
                            }
                            if ui.button(self.t("Duplicate")).clicked() {
                                duplicate = Some(i);
                                ui.close_menu();
                            }
                            if ui.button(self.t("New Tab")).clicked() {
                                add = true;
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button(self.t("Close Tab")).clicked() {
                                close = Some(i);
                                ui.close_menu();
                            }
                            if ui.button(self.t("Close Other Tabs")).clicked() {
                                close_others = Some(i);
                                ui.close_menu();
                            }
                        });
                    });
                }

                ui.separator();
                ui.label(section(self.t("FILES")));
                match &tree_root {
                    Some(root) => {
                        let mut expanded = std::mem::take(&mut self.tree_expanded);
                        file_tree(
                            ui,
                            std::path::Path::new(root),
                            0,
                            &mut expanded,
                            &mut tree_open,
                            fg,
                        );
                        self.tree_expanded = expanded;
                    }
                    None => {
                        ui.label(
                            egui::RichText::new("\u{2014}").color(egui::Color32::from_gray(120)),
                        );
                    }
                }
            });

        if let Some(path) = tree_open {
            self.open_editor(path);
        }
        if add {
            self.new_tab();
        }
        if toggle {
            self.show_details = !self.show_details;
        }
        if settings {
            self.show_settings = true;
            self.settings_family = self.font_family.clone().unwrap_or_default();
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
        if let Some(i) = ungroup {
            if let Some(tab) = self.tabs.get_mut(i) {
                tab.group = None;
            }
        }
        if let Some((i, field, buf)) = edit {
            self.renaming = Some((i, field));
            self.rename_buf = buf;
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
        let snapshot = self.panels.snapshot();
        let muted = egui::Color32::from_gray(120);
        let fg = self.theme.fg;
        let mut open_file: Option<PathBuf> = None;
        let mut compose = false;
        let mut resume: Option<String> = None;
        egui::SidePanel::right("details")
            .resizable(true)
            .default_width(300.0)
            .frame(
                egui::Frame::default()
                    .fill(self.bg())
                    .inner_margin(egui::Margin::same(10.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (tab, glyph, label) in [
                        (DetailsTab::Info, glyph::INFO, self.t("Info")),
                        (DetailsTab::Agent, glyph::AGENT, self.t("Agent")),
                        (DetailsTab::Outline, glyph::OUTLINE, self.t("Outline")),
                        (DetailsTab::Git, glyph::GIT, self.t("Git")),
                        (DetailsTab::Files, glyph::FILES, self.t("Files")),
                        (DetailsTab::Ports, glyph::PORTS, self.t("Ports")),
                        (DetailsTab::Queue, glyph::QUEUE, self.t("Queue")),
                    ] {
                        let text = egui::RichText::new(format!("{glyph} {label}"));
                        if ui.selectable_label(self.details_tab == tab, text).clicked() {
                            self.details_tab = tab;
                        }
                    }
                });
                ui.separator();

                match self.details_tab {
                    DetailsTab::Info => {
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
                                if ui.button(self.t("Copy Path")).clicked() {
                                    ctx.copy_text(p.clone());
                                }
                                if ui.button(self.t("Reveal in Finder")).clicked() {
                                    reveal_in_finder(p);
                                }
                            });
                        }
                        if let Some(git) = &snapshot.git {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(format!("git \u{00b7} {}", git.branch))
                                    .color(fg),
                            );
                            ui.label(
                                egui::RichText::new(format!("{} changed", git.changes.len()))
                                    .color(muted),
                            );
                        }
                        if let Some(a) = &agent {
                            ui.add_space(8.0);
                            let st = a.get("state").and_then(|v| v.as_str()).unwrap_or("?");
                            let name = a.get("agent").and_then(|v| v.as_str()).unwrap_or("agent");
                            ui.label(
                                egui::RichText::new(format!("{name} \u{00b7} {st}")).color(fg),
                            );
                        }
                    }
                    DetailsTab::Agent => match &agent {
                        Some(a) => {
                            let st = a.get("state").and_then(|v| v.as_str()).unwrap_or("?");
                            let name = a.get("agent").and_then(|v| v.as_str()).unwrap_or("agent");
                            ui.label(
                                egui::RichText::new(format!("{name} \u{00b7} {st}")).color(fg),
                            );
                            if let Some(sid) = a.get("session_id").and_then(|v| v.as_str()) {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(sid)
                                            .monospace()
                                            .size(10.0)
                                            .color(muted),
                                    );
                                    if ui.small_button("Copy").clicked() {
                                        ctx.copy_text(sid.to_string());
                                    }
                                });
                            }
                            if let Some(usage) = a.get("usage") {
                                let text = usage
                                    .as_str()
                                    .map(str::to_string)
                                    .unwrap_or_else(|| usage.to_string());
                                ui.label(egui::RichText::new(text).small().color(muted));
                            }
                            if let Some(cmd) = resume_command(a, name) {
                                if ui.button(self.t("Resume")).on_hover_text(&cmd).clicked() {
                                    resume = Some(cmd);
                                }
                            }
                        }
                        None => {
                            ui.label(egui::RichText::new("No agent in this pane").color(muted));
                        }
                    },
                    DetailsTab::Outline => {
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
                                                egui::RichText::new(c)
                                                    .monospace()
                                                    .size(10.0)
                                                    .color(muted),
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
                    }
                    DetailsTab::Git => match &snapshot.git {
                        Some(git) => {
                            ui.label(
                                egui::RichText::new(format!("branch: {}", git.branch))
                                    .monospace()
                                    .color(fg),
                            );
                            ui.add_space(4.0);
                            if git.changes.is_empty() {
                                ui.label(egui::RichText::new("Clean").color(muted));
                            } else {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        for (code, path) in &git.changes {
                                            let text =
                                                egui::RichText::new(format!("{code:<2} {path}"))
                                                    .monospace()
                                                    .size(11.0)
                                                    .color(fg);
                                            if ui.selectable_label(false, text).double_clicked() {
                                                if let Some(cwd) = &cwd {
                                                    open_file = Some(PathBuf::from(cwd).join(path));
                                                }
                                            }
                                        }
                                    });
                            }
                        }
                        None => {
                            ui.label(egui::RichText::new("Not a git repository").color(muted));
                        }
                    },
                    DetailsTab::Files => {
                        if snapshot.files.is_empty() {
                            ui.label(egui::RichText::new("Empty").color(muted));
                        } else {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for (i, entry) in snapshot.files.iter().enumerate() {
                                        let glyph =
                                            if entry.is_dir { "\u{25b8}" } else { "\u{00b7}" };
                                        let text =
                                            egui::RichText::new(format!("{glyph} {}", entry.name))
                                                .monospace()
                                                .size(11.0)
                                                .color(fg);
                                        let resp =
                                            ui.selectable_label(self.file_sel == Some(i), text);
                                        if resp.clicked() {
                                            self.file_sel = Some(i);
                                        }
                                        if resp.double_clicked() && !entry.is_dir {
                                            open_file = Some(entry.path.clone());
                                        }
                                    }
                                });
                        }
                    }
                    DetailsTab::Queue => {
                        ui.horizontal(|ui| {
                            if ui.button(self.t("Compose")).clicked() {
                                compose = true;
                            }
                            ui.label(
                                egui::RichText::new(format!("{} queued", self.queue.len()))
                                    .small()
                                    .color(muted),
                            );
                        });
                        ui.add_space(4.0);
                        if self.queue.is_empty() {
                            ui.label(
                                egui::RichText::new("Prompts send when the agent is idle")
                                    .color(muted),
                            );
                        } else {
                            let mut send = None;
                            let mut remove = None;
                            for (i, q) in self.queue.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    let first = q.text.lines().next().unwrap_or("").to_string();
                                    ui.label(
                                        egui::RichText::new(first).monospace().size(11.0).color(fg),
                                    );
                                    if ui.small_button("Send").clicked() {
                                        send = Some(i);
                                    }
                                    if ui.small_button("\u{2715}").clicked() {
                                        remove = Some(i);
                                    }
                                });
                            }
                            if let Some(i) = send {
                                let q = self.queue.remove(i);
                                self.send_to_pane(&q.pane_id, &q.text);
                            }
                            if let Some(i) = remove {
                                self.queue.remove(i);
                            }
                        }
                    }
                    DetailsTab::Ports => {
                        if snapshot.ports.is_empty() {
                            ui.label(egui::RichText::new("No listening ports").color(muted));
                        } else {
                            for p in &snapshot.ports {
                                ui.label(
                                    egui::RichText::new(format!(":{}  {}", p.port, p.process))
                                        .monospace()
                                        .size(11.0)
                                        .color(fg),
                                );
                            }
                        }
                    }
                }
            });
        if let Some(path) = open_file {
            self.open_editor(path);
        }
        if compose {
            self.open_composer();
        }
        if let Some(cmd) = resume {
            self.send_to_pane(&pane_id, &cmd);
            self.attention.remove(&pane_id);
        }
    }

    /// The label of the tab that owns `pane_id`.
    fn pane_label(&self, pane_id: &str) -> String {
        for tab in &self.tabs {
            if tab.panes.iter().any(|p| p.pane_id == pane_id) {
                return self.tab_title(tab);
            }
        }
        pane_id.to_string()
    }

    /// Write `text` (then Enter) to a pane.
    fn send_to_pane(&mut self, pane_id: &str, text: &str) {
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                if pane.pane_id == pane_id {
                    let mut bytes = text.replace("\r\n", "\n").into_bytes();
                    bytes.push(b'\r');
                    pane.term.write(&bytes);
                    pane.dirty = true;
                    return;
                }
            }
        }
    }

    /// Apply a launch intent (argv, URL scheme or a forwarded request).
    fn apply_intent(&mut self, intent: launch::Intent) {
        match intent {
            launch::Intent::Activate => {}
            launch::Intent::Quick => self.toggle_quick_terminal(),
            launch::Intent::Focus(id) => self.focus_pane(&id),
            launch::Intent::Run(cmd) => self.open_command_tab(&cmd),
        }
    }

    /// Focus a pane by id, switching to its tab.
    fn focus_pane(&mut self, pane_id: &str) {
        for (ti, tab) in self.tabs.iter_mut().enumerate() {
            if tab.panes.iter().any(|p| p.pane_id == pane_id) {
                tab.active = pane_id.to_string();
                self.active = ti;
                self.selection = None;
                self.scroll = 0;
                return;
            }
        }
        eprintln!("miaotty: no pane {pane_id}");
    }

    /// Open a new tab and run `cmd` in it (URL-scheme launch).
    fn open_command_tab(&mut self, cmd: &str) {
        self.new_tab();
        let id = self
            .tabs
            .get(self.active)
            .and_then(|t| t.panes.first())
            .map(|p| p.pane_id.clone());
        if let Some(id) = id {
            self.send_to_pane(&id, cmd);
        }
    }

    /// The "New SSH Session" dialog: resolve the typed target through the ssh
    /// config, then open a tab running the reused/bootstrap ssh command.
    fn ssh_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.ssh_ui.take() else {
            return;
        };
        let mut open = true;
        let mut connect = false;
        egui::Window::new(self.t("New SSH Session"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(self.t("user@host:port"));
                    ui.text_edit_singleline(&mut dialog.target);
                });
                ui.horizontal(|ui| {
                    let ready = ssh::Target::parse(&dialog.target).is_some();
                    if ui
                        .add_enabled(ready, egui::Button::new(self.t("Connect")))
                        .clicked()
                    {
                        connect = true;
                    }
                });
            });
        if connect {
            if let Some(target) = ssh::Target::parse(&dialog.target) {
                let resolved = ssh::resolve(&target);
                let cmd = ssh::command(&resolved, &ssh::bootstrap("xterm-256color"));
                self.open_command_tab(&cmd);
            }
            open = false;
        }
        if open {
            self.ssh_ui = Some(dialog);
        }
    }

    /// Toggle a scratch "Quick Terminal" tab: create it on first use, then
    /// switch between it and the previously active tab.
    fn toggle_quick_terminal(&mut self) {
        if let Some(id) = self.quick_pane.clone() {
            if let Some(ti) = self
                .tabs
                .iter()
                .position(|t| t.panes.iter().any(|p| p.pane_id == id))
            {
                if self.active == ti {
                    if let Some(prev) = self.quick_return.take() {
                        self.active = prev.min(self.tabs.len() - 1);
                        self.selection = None;
                        self.scroll = 0;
                    }
                } else {
                    self.quick_return = Some(self.active);
                    self.active = ti;
                    self.selection = None;
                    self.scroll = 0;
                }
                return;
            }
        }
        let prev = self.active;
        self.new_tab();
        if let Some(tab) = self.tabs.last_mut() {
            tab.title = "Quick".to_string();
            if let Some(pane) = tab.panes.first() {
                self.quick_pane = Some(pane.pane_id.clone());
            }
        }
        self.quick_return = Some(prev);
    }

    fn open_composer(&mut self) {
        let pane_id = self
            .tabs
            .get(self.active)
            .and_then(|t| t.focused())
            .map(|p| p.pane_id.clone())
            .unwrap_or_default();
        self.composer = Some(Composer {
            pane_id,
            text: String::new(),
        });
    }

    fn composer_window(&mut self, ctx: &egui::Context) {
        let fg = self.theme.fg;
        let Some(mut composer) = self.composer.take() else {
            return;
        };
        let target = self.pane_label(&composer.pane_id);
        let mut open = true;
        let mut send = false;
        let mut queue = false;
        egui::Window::new(self.t("Composer"))
            .open(&mut open)
            .default_size([520.0, 280.0])
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(format!("to {target}"))
                        .small()
                        .color(fg),
                );
                ui.add(
                    egui::TextEdit::multiline(&mut composer.text)
                        .hint_text(self.t("Prompt\u{2026}"))
                        .desired_width(f32::INFINITY)
                        .desired_rows(8),
                );
                ui.horizontal(|ui| {
                    let ready = !composer.text.trim().is_empty();
                    if ui
                        .add_enabled(ready, egui::Button::new(self.t("Send")))
                        .clicked()
                    {
                        send = true;
                    }
                    if ui
                        .add_enabled(ready, egui::Button::new(self.t("Queue it")))
                        .clicked()
                    {
                        queue = true;
                    }
                });
            });
        if send {
            let text = std::mem::take(&mut composer.text);
            self.send_to_pane(&composer.pane_id, &text);
        }
        if queue {
            let text = std::mem::take(&mut composer.text);
            self.queue.push(QueuedPrompt {
                pane_id: composer.pane_id.clone(),
                text,
            });
        }
        if open {
            self.composer = Some(composer);
        }
    }

    fn recipe_window(&mut self, ctx: &egui::Context) {
        let l_save = self.t("Save");
        let l_name = self.t("Name");
        let l_none = self.t("No recipes yet");
        let title = if self.recipe_ui.as_ref().map(|d| d.save).unwrap_or(false) {
            self.t("Save Recipe")
        } else {
            self.t("Open Recipe")
        };
        let Some(mut dialog) = self.recipe_ui.take() else {
            return;
        };
        let fg = self.theme.fg;
        let mut open = true;
        let mut close = false;
        let mut saved = false;
        let mut chosen: Option<String> = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                if dialog.save {
                    ui.horizontal(|ui| {
                        ui.label(l_name);
                        ui.text_edit_singleline(&mut dialog.name);
                    });
                    let ready = !dialog.name.trim().is_empty();
                    if ui.add_enabled(ready, egui::Button::new(l_save)).clicked() {
                        saved = true;
                    }
                } else if dialog.list.is_empty() {
                    ui.label(egui::RichText::new(l_none).color(fg));
                } else {
                    for name in &dialog.list {
                        if ui.button(name).clicked() {
                            chosen = Some(name.clone());
                            close = true;
                        }
                    }
                }
            });
        if saved {
            let session = self.snapshot();
            match save_recipe(dialog.name.trim(), &session) {
                Ok(path) => eprintln!("miaotty: wrote {}", path.display()),
                Err(e) => eprintln!("miaotty: failed to save recipe: {e}"),
            }
            open = false;
        }
        if close {
            open = false;
        }
        if let Some(name) = chosen {
            if let Some(session) = load_recipe(&name) {
                self.restore(session);
            }
        }
        if open {
            self.recipe_ui = Some(dialog);
        }
    }

    /// Notifications, sleep prevention and the prompt queue (ADR 0010).
    fn agent_loop(&mut self, ctx: &egui::Context) {
        let focused_id = self
            .tabs
            .get(self.active)
            .and_then(|t| t.focused())
            .map(|p| p.pane_id.clone());
        let mut states: Vec<(String, String, String)> = Vec::new();
        let mut any_processing = false;
        for tab in &self.tabs {
            for pane in &tab.panes {
                let Some(a) = self.state.agent_for(&pane.pane_id) else {
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
                states.push((pane.pane_id.clone(), agent, state));
            }
        }
        let mut alert: Option<(String, String)> = None;
        for (id, agent, state) in states {
            let prev = self.agent_states.insert(id.clone(), state.clone());
            let changed = prev.as_deref() != Some(state.as_str());
            let wants = matches!(state.as_str(), "awaiting" | "error");
            let focused = Some(&id) == focused_id.as_ref();
            if wants && !focused {
                self.attention.insert(id.clone());
            } else {
                self.attention.remove(&id);
            }
            if changed && wants && self.notifications && !focused {
                alert = Some((format!("{agent} \u{00b7} {state}"), self.pane_label(&id)));
            }
        }
        if let Some((title, body)) = alert {
            agentloop::notify(&title, &body);
        }
        if self.prevent_sleep {
            self.sleep.set_awake(any_processing);
        }
        if let Some(pos) = self.queue.iter().position(|q| {
            self.agent_states
                .get(&q.pane_id)
                .is_some_and(|s| s == "idle")
        }) {
            let q = self.queue.remove(pos);
            self.send_to_pane(&q.pane_id, &q.text);
            ctx.request_repaint();
        }
    }

    /// Open a file in the read-only preview / editor window (ADR 0009).
    fn open_editor(&mut self, path: PathBuf) {
        self.open_editor_at(path, None);
    }

    fn open_editor_at(&mut self, path: PathBuf, line: Option<usize>) {
        const MAX: u64 = 2 * 1024 * 1024;
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() > MAX => {
                eprintln!("miaotty: {} is too large to preview", path.display());
                return;
            }
            Err(_) => return,
            _ => {}
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => return,
        };
        let text = String::from_utf8_lossy(&bytes).to_string();
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path.clone());
        self.recent_files.truncate(50);
        self.editor = Some(Editor {
            path,
            original: text.clone(),
            text,
            readonly: true,
            raw: false,
            target: line,
            jumped: false,
            remote: None,
        });
    }

    fn editor_window(&mut self, ctx: &egui::Context) {
        let fg = self.theme.fg;
        let muted = egui::Color32::from_gray(120);
        let base = self
            .editor
            .as_ref()
            .and_then(|e| e.path.parent().map(|p| p.to_path_buf()));
        let l_close = self.t("Close");
        let l_save = self.t("Save");
        let l_reload = self.t("Reload");
        let l_raw = self.t("Raw");
        let l_md = self.t("Markdown");
        let l_preview = self.t("Preview");
        let l_edit = self.t("Edit");
        let l_open_ext = self.t("Open Externally");
        let l_edit_tab = self.t("Edit in Tab");
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let mut open = true;
        let title = editor
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| editor.path.display().to_string());
        let dirty = editor.text != editor.original;
        let is_md = matches!(
            editor
                .path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .as_deref(),
            Some("md" | "markdown")
        );
        let mut save = false;
        let mut reload = false;
        let mut close = false;
        let mut open_ext = false;
        let mut edit_tab = false;
        egui::Window::new(format!("\u{25a4} {title}"))
            .open(&mut open)
            .default_size([640.0, 480.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(editor.path.display().to_string())
                            .small()
                            .color(fg),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(l_close).clicked() {
                            close = true;
                        }
                        if ui
                            .add_enabled(!editor.readonly && dirty, egui::Button::new(l_save))
                            .clicked()
                        {
                            save = true;
                        }
                        if ui.button(l_reload).clicked() {
                            reload = true;
                        }
                        if ui.button(l_open_ext).clicked() {
                            open_ext = true;
                        }
                        if ui.button(l_edit_tab).clicked() {
                            edit_tab = true;
                        }
                        if is_md {
                            let label = if editor.raw { l_md } else { l_raw };
                            if ui.button(label).clicked() {
                                editor.raw = !editor.raw;
                            }
                        }
                        let mode = if editor.readonly { l_preview } else { l_edit };
                        if ui.button(mode).clicked() {
                            editor.readonly = !editor.readonly;
                        }
                    });
                });
                ui.separator();
                if editor.readonly && is_md && !editor.raw {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            markdown_ui(ui, &editor.text, fg, base.as_deref(), self.lang);
                        });
                } else if editor.readonly {
                    source_ui(ui, &editor.text, editor.target, &mut editor.jumped, fg);
                } else {
                    // Editable: a pinned line-number gutter beside the editor.
                    // The gutter sits outside the scroll area, so it does not
                    // move when long lines scroll horizontally.
                    let digits = editor.text.lines().count().to_string().len().max(2);
                    let gutter_w = 10.0 + digits as f32 * 7.5;
                    let mut output: Option<egui::text_edit::TextEditOutput> = None;
                    ui.horizontal_top(|ui| {
                        let height = ui.available_height().max(20.0);
                        let (gutter, _) = ui.allocate_exact_size(
                            egui::vec2(gutter_w, height),
                            egui::Sense::hover(),
                        );
                        ui.vertical(|ui| {
                            egui::ScrollArea::both()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    output = Some(
                                        egui::TextEdit::multiline(&mut editor.text)
                                            .code_editor()
                                            .desired_width(f32::INFINITY)
                                            .show(ui),
                                    );
                                });
                        });
                        if let Some(out) = &output {
                            paint_gutter(ui, gutter, out, muted);
                        }
                    });
                }
            });
        if reload {
            if let Some(editor) = self.editor.as_mut() {
                let bytes = match &editor.remote {
                    Some(r) => ssh::read_remote(&r.dest, &r.path).ok(),
                    None => std::fs::read(&editor.path).ok(),
                };
                if let Some(bytes) = bytes {
                    editor.text = String::from_utf8_lossy(&bytes).to_string();
                    editor.original = editor.text.clone();
                }
            }
        }
        if save {
            if let Some(editor) = self.editor.as_mut() {
                let result = match &editor.remote {
                    Some(r) => ssh::write_remote(&r.dest, &r.path, editor.text.as_bytes()),
                    None => std::fs::write(&editor.path, editor.text.as_bytes()),
                };
                match result {
                    Ok(()) => editor.original = editor.text.clone(),
                    Err(e) => {
                        let msg = format!("save failed: {e}");
                        eprintln!("miaotty: {msg}");
                        self.integration_msg = Some(msg);
                    }
                }
            }
        }
        if open_ext {
            if let Some(editor) = self.editor.as_ref() {
                open_path_externally(&editor.path);
            }
        }
        if edit_tab {
            let editor_cmd = self
                .external_editor
                .clone()
                .unwrap_or_else(|| "${EDITOR:-vi}".to_string());
            let cmd = self.editor.as_ref().map(|e| {
                format!(
                    "{} {}",
                    editor_cmd,
                    ssh::shell_quote(&e.path.display().to_string())
                )
            });
            if let Some(cmd) = cmd {
                self.open_command_tab(&cmd);
            }
        }
        if close {
            open = false;
        }
        if !open {
            self.editor = None;
        }
    }

    /// Translate a visible string key (ADR 0013).
    fn t(&self, key: &'static str) -> &'static str {
        i18n::t(self.lang, key)
    }

    /// Start (or report) an update check against the configured URL.
    fn check_updates(&mut self) {
        match &self.update_config {
            Some(url) => {
                self.update_rx = Some(spawn_update_check(url.clone()));
                self.update_msg = Some("Checking\u{2026}".to_string());
            }
            None => self.update_msg = Some(self.t("No update URL configured").to_string()),
        }
    }

    /// Replace the running `.app` with the verified download and relaunch
    /// (macOS). Elsewhere (or when not running from a bundle) the artifact is
    /// opened instead.
    fn install_update(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_os = "macos"))]
        let _ = ctx;
        let Some(artifact) = self.update_ready.clone() else {
            self.update_msg = Some(self.t("No verified download").to_string());
            return;
        };
        #[cfg(target_os = "macos")]
        {
            let Some(bundle) = std::env::current_exe()
                .ok()
                .as_deref()
                .and_then(bundle_root)
            else {
                self.update_msg = Some(self.t("Not running from an app bundle").to_string());
                open_path_externally(&artifact);
                return;
            };
            let stage = std::env::temp_dir().join(format!("miaotty-update-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&stage);
            let unzip = std::process::Command::new("unzip")
                .args(["-q", "-o"])
                .arg(&artifact)
                .arg("-d")
                .arg(&stage)
                .status();
            if !matches!(unzip, Ok(s) if s.success()) {
                self.update_msg = Some(self.t("Could not unpack the update").to_string());
                return;
            }
            let Some(new_app) = find_app(&stage) else {
                self.update_msg = Some(self.t("No .app in the update").to_string());
                return;
            };
            let script =
                std::env::temp_dir().join(format!("miaotty-install-{}.sh", std::process::id()));
            if std::fs::write(
                &script,
                install_script(std::process::id(), &bundle, &new_app),
            )
            .is_err()
            {
                self.update_msg = Some(self.t("Could not stage the installer").to_string());
                return;
            }
            let _ = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("nohup sh '{}' >/dev/null 2>&1 &", script.display()))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            self.update_msg = Some(self.t("Installing and relaunching\u{2026}").to_string());
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        #[cfg(target_os = "windows")]
        {
            let exe = std::env::current_exe().unwrap_or_else(|_| artifact.clone());
            let script =
                std::env::temp_dir().join(format!("miaotty-install-{}.cmd", std::process::id()));
            if std::fs::write(
                &script,
                windows_install_script(std::process::id(), &artifact, &exe),
            )
            .is_err()
            {
                self.update_msg = Some(self.t("Could not stage the installer").to_string());
                return;
            }
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let _ = std::process::Command::new("cmd")
                .args(["/c", "start", "", "/b"])
                .arg(&script)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            self.update_msg = Some(self.t("Installing and relaunching\u{2026}").to_string());
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        #[cfg(target_os = "linux")]
        {
            // Only an AppImage is a single file we can swap; anything else (.deb,
            // tarball) is the user's package manager's job.
            if let Ok(appimage) = std::env::var("APPIMAGE") {
                let script =
                    std::env::temp_dir().join(format!("miaotty-install-{}.sh", std::process::id()));
                if std::fs::write(
                    &script,
                    linux_install_script(
                        std::process::id(),
                        std::path::Path::new(&appimage),
                        &artifact,
                    ),
                )
                .is_err()
                {
                    self.update_msg = Some(self.t("Could not stage the installer").to_string());
                    return;
                }
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!(
                        "setsid nohup sh '{}' >/dev/null 2>&1 &",
                        script.display()
                    ))
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                self.update_msg = Some(self.t("Installing and relaunching\u{2026}").to_string());
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.update_msg = Some(self.t("Open the download to install").to_string());
                open_path_externally(&artifact);
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            self.update_msg = Some(self.t("Open the download to install").to_string());
            open_path_externally(&artifact);
        }
    }

    /// Whether a download is offered for this platform right now.
    fn update_available(&self) -> bool {
        self.update_manifest
            .as_ref()
            .and_then(|m| m.for_platform())
            .is_some_and(|a| !a.url.is_empty())
    }

    /// Download and verify the platform artifact of the checked manifest.
    fn download_update(&mut self) {
        let artifact = self
            .update_manifest
            .as_ref()
            .and_then(|m| m.for_platform())
            .cloned();
        match artifact {
            Some(a) if !a.url.is_empty() => {
                self.update_rx = Some(spawn_update_download(
                    a.url,
                    a.sha256,
                    a.signature,
                    self.update_pubkey.clone(),
                ));
                self.update_msg = Some("Downloading\u{2026}".to_string());
            }
            _ => {
                self.update_msg = Some(self.t("No download for this platform").to_string());
            }
        }
    }

    /// The theme background with the configured opacity applied.
    fn bg(&self) -> egui::Color32 {
        let c = self.theme.bg;
        if self.opacity >= 1.0 {
            c
        } else {
            egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), self.alpha)
        }
    }

    /// The window backdrop: the theme background darkened, so the terminal
    /// "container" card (drawn in `bg()`) reads as a raised surface.
    fn window_bg(&self) -> egui::Color32 {
        let c = self.bg();
        let dark = |v: u8| ((v as f32) * 0.82).round() as u8;
        egui::Color32::from_rgba_unmultiplied(dark(c.r()), dark(c.g()), dark(c.b()), self.alpha)
    }

    /// Evaluate the view rule engine for a pane (ADR 0007).
    fn view_for(&self, pane: &Pane) -> Option<miao_term_config::view::Resolved> {
        let agent = self
            .state
            .agent_for(&pane.pane_id)
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

    /// The label for a tab: the view rule result, else the cwd folder, else the
    /// tab's stored title.
    fn tab_title(&self, tab: &Tab) -> String {
        if let Some(pane) = tab.focused() {
            if let Some(view) = self.view_for(pane) {
                if !view.title.is_empty() {
                    return view.title;
                }
            }
            if let Some(name) = pane
                .term
                .cwd()
                .and_then(|p| std::path::Path::new(p).file_name())
            {
                let name = name.to_string_lossy();
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }
        tab.title.clone()
    }

    fn mark_all_dirty(&mut self) {
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                pane.dirty = true;
            }
        }
    }

    fn save_config(&self) {
        let bg = self.theme.bg;
        let fg = self.theme.fg;
        let mut out = String::new();
        out.push_str(&format!("font-size = {}\n", self.font_size));
        if let Some(family) = &self.font_family {
            out.push_str(&format!("font-family = {family:?}\n"));
        }
        out.push_str(&format!("line-height = {}\n", self.line_height_ratio));
        let cursor = match self.cursor_style {
            miao_term_config::CursorStyle::Block => "block",
            miao_term_config::CursorStyle::Bar => "bar",
            miao_term_config::CursorStyle::Underline => "underline",
        };
        out.push_str(&format!("cursor-style = {cursor:?}\n"));
        out.push_str(&format!("background-opacity = {}\n", self.opacity));
        out.push_str(&format!("notifications = {}\n", self.notifications));
        out.push_str(&format!("prevent-sleep = {}\n", self.prevent_sleep));
        if self.lang == i18n::Lang::Zh {
            out.push_str("language = \"zh\"\n");
        }
        if let Some(url) = &self.update_config {
            out.push_str(&format!("update-check-url = \"{url}\"\n"));
        }
        out.push_str("[badges]\n");
        out.push_str(&format!("processing = {}\n", self.badges.processing));
        out.push_str(&format!("idle = {}\n", self.badges.idle));
        out.push_str(&format!("awaiting = {}\n", self.badges.awaiting));
        out.push_str(&format!("error = {}\n", self.badges.error));
        out.push_str("\n[colors]\n");
        out.push_str(&format!(
            "background = \"#{:02x}{:02x}{:02x}\"\n",
            bg.r(),
            bg.g(),
            bg.b()
        ));
        out.push_str(&format!(
            "foreground = \"#{:02x}{:02x}{:02x}\"\n",
            fg.r(),
            fg.g(),
            fg.b()
        ));
        out.push_str("palette = [\n");
        for c in &self.theme.palette {
            out.push_str(&format!(
                "  \"#{:02x}{:02x}{:02x}\",\n",
                c.r(),
                c.g(),
                c.b()
            ));
        }
        out.push_str("]\n");

        if let Some(path) = miao_term_config::Config::path() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::write(&path, out) {
                Ok(()) => eprintln!("miaotty: wrote {}", path.display()),
                Err(e) => eprintln!("miaotty: failed to write config: {e}"),
            }
        }
    }

    /// The "view/edit a remote file" dialog (ADR 0021).
    fn remote_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.remote_ui.take() else {
            return;
        };
        let mut open = true;
        let mut load = false;
        let title = if dialog.edit {
            self.t("Edit Remote File\u{2026}")
        } else {
            self.t("View Remote File\u{2026}")
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(self.t("SSH destination"));
                    ui.text_edit_singleline(&mut dialog.dest);
                });
                ui.horizontal(|ui| {
                    ui.label(self.t("Remote path"));
                    ui.text_edit_singleline(&mut dialog.path);
                });
                let ready = !dialog.dest.trim().is_empty() && !dialog.path.trim().is_empty();
                if ui
                    .add_enabled(ready, egui::Button::new(self.t("Load")))
                    .clicked()
                {
                    load = true;
                }
            });
        if load {
            let dest = dialog.dest.trim().to_string();
            let path = dialog.path.trim().to_string();
            let edit = dialog.edit;
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = ssh::read_remote(&dest, &path)
                    .map(|bytes| {
                        (
                            RemoteRef {
                                dest: dest.clone(),
                                path: path.clone(),
                            },
                            String::from_utf8_lossy(&bytes).to_string(),
                        )
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(result);
            });
            self.remote_rx = Some((edit, rx));
            open = false;
        }
        if open {
            self.remote_ui = Some(dialog);
        }
    }

    /// Quick Terminal hotkey status and external-binding snippets (ADR 0019).
    fn quick_hotkey_ui(&mut self, ui: &mut egui::Ui) {
        let muted = egui::Color32::from_gray(120);
        ui.label(section("QUICK TERMINAL"));
        ui.label(match &self.quick_hotkey {
            Some(spec) => format!("{} · {spec}", self.t("Hotkey")),
            None => self.t("No hotkey configured").to_string(),
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(self.t("Bind externally:"))
                    .small()
                    .color(muted),
            );
            for tool in integration::HOTKEY_TOOLS {
                if ui.small_button(*tool).clicked() {
                    ui.ctx().copy_text(integration::hotkey_snippet(tool));
                }
            }
        });
    }

    /// Agent integration: detect agents, install hooks, launch them (ADR 0016).
    fn agent_integrations_ui(&mut self, ui: &mut egui::Ui) {
        let muted = egui::Color32::from_gray(120);
        let ok = egui::Color32::from_rgb(0xa3, 0xbe, 0x8c);
        ui.label(section("AGENT INTEGRATIONS"));
        let mut install: Option<&'static str> = None;
        let mut copy: Option<String> = None;
        let mut launch: Option<String> = None;
        for agent in integration::AGENTS {
            ui.horizontal(|ui| {
                ui.label(agent.name);
                let found = integration::detected(agent.bin);
                let label = if found {
                    self.t("detected")
                } else {
                    self.t("not found")
                };
                ui.label(
                    egui::RichText::new(label)
                        .small()
                        .color(if found { ok } else { muted }),
                );
                if ui.small_button(self.t("Install hook")).clicked() {
                    install = Some(agent.name);
                }
                if ui.small_button(self.t("Copy snippet")).clicked() {
                    if let Some(path) = integration::script_path(agent.name) {
                        copy = Some(integration::snippet(agent, &path));
                    }
                }
                if ui.small_button(self.t("Launch")).clicked() {
                    launch = Some(integration::launch_command(agent));
                }
            });
        }
        if let Some(msg) = &self.integration_msg {
            ui.label(egui::RichText::new(msg).small().color(muted));
        }
        if let Some(name) = install {
            self.integration_msg = Some(match integration::install(name) {
                Ok(path) => format!("installed {}", path.display()),
                Err(e) => format!("install failed: {e}"),
            });
        }
        if let Some(text) = copy {
            ui.ctx().copy_text(text);
            self.integration_msg = Some(self.t("Snippet copied").to_string());
        }
        if let Some(cmd) = launch {
            self.open_command_tab(&cmd);
        }
    }

    /// The View-rule editor: rule list with priority, a per-rule form and a
    /// live preview (ADR 0007).
    fn view_rules_ui(&mut self, ui: &mut egui::Ui) {
        let muted = egui::Color32::from_gray(120);
        ui.label(section("VIEW RULES"));
        ui.label(
            egui::RichText::new("~/.config/miaotty/views.json")
                .small()
                .color(muted),
        );

        let snapshot: Vec<(usize, String, String)> = self
            .rules
            .rules
            .iter()
            .enumerate()
            .map(|(i, rule)| {
                let label = rule.name.clone().unwrap_or_else(|| format!("rule {i}"));
                (i, label, match_summary(rule))
            })
            .collect();

        let mut move_up = None;
        let mut move_down = None;
        let mut remove = None;
        for (i, label, summary) in snapshot {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.view_edit == Some(i), format!("{label}  {summary}"))
                    .clicked()
                {
                    self.view_edit = Some(i);
                }
                if ui.small_button("\u{2191}").clicked() {
                    move_up = Some(i);
                }
                if ui.small_button("\u{2193}").clicked() {
                    move_down = Some(i);
                }
                if ui.small_button("\u{2715}").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = move_up {
            self.rules.move_up(i);
            if self.view_edit == Some(i) {
                self.view_edit = Some(i - 1);
            }
        }
        if let Some(i) = move_down {
            self.rules.move_down(i);
            if self.view_edit == Some(i) {
                self.view_edit = Some(i + 1);
            }
        }
        if let Some(i) = remove {
            self.rules.rules.remove(i);
            self.view_edit = None;
        }

        ui.horizontal(|ui| {
            if ui.button(self.t("+ Rule")).clicked() {
                self.rules.rules.push(miao_term_config::view::Rule {
                    title: Some("{folder}".to_string()),
                    ..Default::default()
                });
                self.view_edit = Some(self.rules.rules.len() - 1);
            }
            if ui.button(self.t("Save views.json")).clicked() {
                match self.rules.save() {
                    Ok(()) => eprintln!("miaotty: wrote views.json"),
                    Err(e) => eprintln!("miaotty: failed to write views.json: {e}"),
                }
            }
        });

        if let Some(i) = self.view_edit {
            if i < self.rules.rules.len() {
                ui.separator();
                self.rule_editor(ui, i);
            }
        }
    }

    fn rule_editor(&mut self, ui: &mut egui::Ui, i: usize) {
        let fg = self.theme.fg;
        let muted = egui::Color32::from_gray(120);
        let sel_bg = ui.style().visuals.selection.bg_fill;
        let rule = &mut self.rules.rules[i];

        ui.horizontal(|ui| {
            ui.label("Name");
            opt_text(ui, &mut rule.name);
        });
        ui.label(egui::RichText::new("match").small().color(muted));
        for (label, field) in [
            ("path", &mut rule.r#match.path),
            ("command", &mut rule.r#match.command),
            ("agent", &mut rule.r#match.agent),
            ("host", &mut rule.r#match.host),
            ("file", &mut rule.r#match.file),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                opt_text(ui, field);
            });
        }
        ui.horizontal(|ui| {
            ui.label("alias");
            opt_text(ui, &mut rule.alias);
        });
        ui.horizontal(|ui| {
            ui.label("title");
            opt_text(ui, &mut rule.title);
        });
        ui.horizontal(|ui| {
            ui.label("badge");
            opt_text(ui, &mut rule.badge);
        });
        ui.horizontal(|ui| {
            ui.label("icon");
            if ui.small_button("clear").clicked() {
                rule.icon = None;
            }
            let icon = rule.icon.get_or_insert_with(Default::default);
            ui.label("name");
            opt_text(ui, &mut icon.name);
            ui.label("emoji");
            opt_text(ui, &mut icon.emoji);
            ui.label("color");
            opt_text(ui, &mut icon.color);
        });
        ui.horizontal_wrapped(|ui| {
            for name in icons::names() {
                let selected = rule.icon.as_ref().and_then(|i| i.name.as_deref()) == Some(name);
                let (rect, resp) =
                    ui.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::click());
                let painter = ui.painter();
                if selected {
                    painter.rect_filled(rect, 3.0, sel_bg);
                } else if resp.hovered() {
                    painter.rect_filled(rect, 3.0, egui::Color32::from_gray(60));
                }
                icons::draw(painter, rect.shrink(3.0), name, fg);
                if resp.clicked() {
                    rule.icon.get_or_insert_with(Default::default).name = Some(name.to_string());
                }
            }
        });

        ui.separator();
        let ctx = preview_context(&rule.r#match);
        let mut single = miao_term_config::view::RuleSet::default();
        single.rules.push(rule.clone());
        if let Some(view) = single.evaluate(&ctx) {
            ui.horizontal(|ui| {
                if let Some(icon) = &view.icon {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
                    icons::draw_view(ui.painter(), rect, icon, fg);
                }
                ui.label(egui::RichText::new(&view.title).monospace().color(fg));
            });
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut install = false;
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(self.t("Font size"));
                    if ui
                        .add(egui::Slider::new(&mut self.font_size, 8.0..=32.0))
                        .changed()
                    {
                        self.mark_all_dirty();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(self.t("Font family"));
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut self.settings_family)
                                .desired_width(180.0),
                        )
                        .changed()
                    {
                        let trimmed = self.settings_family.trim();
                        self.font_family = (!trimmed.is_empty()).then(|| trimmed.to_string());
                        self.mark_all_dirty();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(self.t("Opacity"));
                    if ui
                        .add(egui::Slider::new(&mut self.opacity, 0.2..=1.0))
                        .changed()
                    {
                        self.alpha = (self.opacity * 255.0).round() as u8;
                    }
                });
                let notify_label = self.t("Notify");
                let awake_label = self.t("Keep awake");
                ui.horizontal(|ui| {
                    ui.label(self.t("Agents"));
                    ui.checkbox(&mut self.notifications, notify_label);
                    ui.checkbox(&mut self.prevent_sleep, awake_label);
                });
                ui.horizontal(|ui| {
                    ui.label(self.t("Badges"));
                    ui.checkbox(&mut self.badges.processing, "processing");
                    ui.checkbox(&mut self.badges.idle, "idle");
                    ui.checkbox(&mut self.badges.awaiting, "awaiting");
                    ui.checkbox(&mut self.badges.error, "error");
                });
                ui.horizontal(|ui| {
                    ui.label(self.t("Sleep guard"));
                    ui.label(if self.sleep.awake() {
                        "awake"
                    } else {
                        "\u{2014}"
                    });
                    if ui.button(self.t("Check for Updates")).clicked() {
                        self.check_updates();
                    }
                    let can_download = self.update_available();
                    if ui
                        .add_enabled(can_download, egui::Button::new(self.t("Download Update")))
                        .clicked()
                    {
                        self.download_update();
                    }
                });
                if let Some(msg) = &self.update_msg {
                    ui.label(
                        egui::RichText::new(msg)
                            .small()
                            .color(egui::Color32::from_gray(150)),
                    );
                }
                if self.update_ready.is_some() {
                    ui.horizontal(|ui| {
                        if ui.button(self.t("Open Download")).clicked() {
                            if let Some(path) = self.update_ready.clone() {
                                open_path_externally(&path);
                            }
                        }
                        if ui
                            .button(self.t("Install and Relaunch"))
                            .on_hover_text(
                                self.t("Replaces this app with the download, then restarts it."),
                            )
                            .clicked()
                        {
                            install = true;
                        }
                    });
                }
                if install {
                    self.install_update(ctx);
                }
                ui.horizontal(|ui| {
                    ui.label(self.t("Line height"));
                    if ui
                        .add(egui::Slider::new(&mut self.line_height_ratio, 0.9..=2.0))
                        .changed()
                    {
                        self.mark_all_dirty();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(self.t("Cursor"));
                    use miao_term_config::CursorStyle::*;
                    for (label, style) in [("block", Block), ("bar", Bar), ("underline", Underline)]
                    {
                        if ui.button(label).clicked() {
                            self.cursor_style = style;
                            self.mark_all_dirty();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(self.t("Theme"));
                    for name in ["nord", "dracula", "gruvbox", "solarized", "tokyo-night"] {
                        if ui.button(name).clicked() {
                            if let Some(theme) = miao_term_config::theme_by_name(name) {
                                self.theme = Theme::from_config(&theme);
                                self.mark_all_dirty();
                            }
                        }
                    }
                });
                ui.separator();
                self.quick_hotkey_ui(ui);
                ui.separator();
                self.agent_integrations_ui(ui);
                ui.separator();
                self.view_rules_ui(ui);
                ui.separator();
                if ui.button(self.t("Save to config.toml")).clicked() {
                    self.save_config();
                }
            });
        if !open {
            self.show_settings = false;
        }
    }

    fn menu_bar(&mut self, ctx: &egui::Context) {
        let mut verb: Option<Verb> = None;
        let mut open_palette = false;
        let bg = self.window_bg();
        let fg = self.theme.fg;
        egui::TopBottomPanel::top("menubar")
            .frame(
                egui::Frame::default()
                    .fill(bg)
                    .inner_margin(egui::Margin::symmetric(6.0, 1.0)),
            )
            .show(ctx, |ui| {
                ui.visuals_mut().override_text_color = Some(fg);
                ui.horizontal(|ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("New Tab").clicked() {
                            verb = Some(Verb::NewTab);
                            ui.close_menu();
                        }
                        if ui.button("Close Tab").clicked() {
                            verb = Some(Verb::CloseTab);
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Save Recipe…").clicked() {
                            verb = Some(Verb::SaveRecipe);
                            ui.close_menu();
                        }
                        if ui.button("Open Recipe…").clicked() {
                            verb = Some(Verb::OpenRecipe);
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui.button("Find…").clicked() {
                            verb = Some(Verb::Find);
                            ui.close_menu();
                        }
                        if ui.button("Copy Path").clicked() {
                            verb = Some(Verb::CopyPath);
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("View", |ui| {
                        if ui.button("Command Palette…").clicked() {
                            open_palette = true;
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Toggle Tabs Panel").clicked() {
                            verb = Some(Verb::ToggleSidebar);
                            ui.close_menu();
                        }
                        if ui.button("Toggle Details Panel").clicked() {
                            verb = Some(Verb::ToggleDetails);
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Increase Font Size").clicked() {
                            verb = Some(Verb::IncreaseFont);
                            ui.close_menu();
                        }
                        if ui.button("Decrease Font Size").clicked() {
                            verb = Some(Verb::DecreaseFont);
                            ui.close_menu();
                        }
                        if ui.button("Reset Font Size").clicked() {
                            verb = Some(Verb::ResetFont);
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Shell", |ui| {
                        if ui.button("Split Right").clicked() {
                            verb = Some(Verb::SplitRight);
                            ui.close_menu();
                        }
                        if ui.button("Split Down").clicked() {
                            verb = Some(Verb::SplitDown);
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Clear Screen").clicked() {
                            verb = Some(Verb::ClearScreen);
                            ui.close_menu();
                        }
                        if ui.button("Reveal in Finder").clicked() {
                            verb = Some(Verb::RevealInFinder);
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Window", |ui| {
                        if ui.button("Settings…").clicked() {
                            verb = Some(Verb::Settings);
                            ui.close_menu();
                        }
                        if ui.button("New SSH Session…").clicked() {
                            verb = Some(Verb::NewSsh);
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Help", |ui| {
                        ui.hyperlink_to(
                            "Documentation",
                            "https://github.com/oxdingzg/miao-term#readme",
                        );
                        if ui.button("Check for Updates").clicked() {
                            verb = Some(Verb::CheckUpdates);
                            ui.close_menu();
                        }
                    });
                });
            });
        if open_palette {
            self.palette = Some(Palette {
                query: String::new(),
                selected: 0,
            });
            self.palette_focus = true;
        }
        if let Some(v) = verb {
            self.run_palette(PaletteAction::Run(v));
        }
    }

    fn tab_bar(&mut self, ctx: &egui::Context) {
        let fg = self.theme.fg;
        let active = self.active;
        let sel_bg = ctx.style().visuals.selection.bg_fill;
        let titles: Vec<TabBarRow> = self
            .tabs
            .iter()
            .map(|tab| {
                let focused = tab.focused();
                let title = format!(
                    "{}{}{}",
                    tab.prefix.as_deref().unwrap_or(""),
                    self.tab_title(tab),
                    tab.mark.as_deref().unwrap_or("")
                );
                let view = focused.and_then(|p| self.view_for(p));
                let color = focused
                    .and_then(|p| self.state.agent_for(&p.pane_id))
                    .and_then(|a| a.get("state").and_then(|v| v.as_str()).map(str::to_string))
                    .filter(|s| self.badges.enabled(s))
                    .map(|s| agent_color(&s));
                let att = focused.is_some_and(|p| self.attention.contains(&p.pane_id));
                (
                    title,
                    color,
                    view.and_then(|v| v.icon),
                    att,
                    tab.group.clone(),
                )
            })
            .collect();

        let mut rects: Vec<egui::Rect> = Vec::new();
        let mut click: Option<usize> = None;
        let mut started: Option<usize> = None;
        let mut stopped = false;
        let mut close: Option<usize> = None;
        let mut add = false;

        egui::TopBottomPanel::top("tabbar")
            .frame(
                egui::Frame::default()
                    .fill(self.bg())
                    .inner_margin(egui::Margin::symmetric(6.0, 3.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let mut prev_group: Option<String> = None;
                    for (i, (title, color, icon, att, group)) in titles.into_iter().enumerate() {
                        if i > 0 && group != prev_group {
                            ui.label(
                                egui::RichText::new("\u{2502}").color(egui::Color32::from_gray(80)),
                            );
                        }
                        prev_group = group;
                        if att {
                            ui.colored_label(egui::Color32::from_rgb(0xeb, 0xcb, 0x8b), "\u{0021}");
                        }
                        if let Some(icon) = &icon {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::Vec2::splat(13.0), egui::Sense::hover());
                            icons::draw_view(ui.painter(), rect, icon, fg);
                        }
                        if let Some(c) = color {
                            ui.colored_label(c, "\u{25cf}");
                        }
                        let mut text = egui::RichText::new(title).size(12.0).color(fg);
                        if i == active {
                            text = text.background_color(sel_bg);
                        }
                        let resp = ui.add(
                            egui::Label::new(text)
                                .sense(egui::Sense::click_and_drag())
                                .selectable(false),
                        );
                        rects.push(resp.rect);
                        if resp.clicked() {
                            click = Some(i);
                        }
                        if resp.drag_started() {
                            started = Some(i);
                        }
                        if resp.drag_stopped() {
                            stopped = true;
                        }
                        if ui.small_button("\u{00d7}").clicked() {
                            close = Some(i);
                        }
                    }
                    if ui.small_button("+").clicked() {
                        add = true;
                    }
                });
            });

        if let Some(i) = started {
            self.tab_drag = Some(i);
        }
        if stopped {
            self.tab_drag = None;
        }
        if let Some(drag) = self.tab_drag {
            if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                if let Some(target) = rects.iter().position(|r| r.contains(pos)) {
                    if target != drag {
                        let was_active = self.active == drag;
                        let old_active = self.active;
                        let moved = self.tabs.remove(drag);
                        self.tabs.insert(target, moved);
                        self.active = if was_active {
                            target
                        } else {
                            let mut a = old_active;
                            if drag < a {
                                a -= 1;
                            }
                            if target <= a {
                                a += 1;
                            }
                            a.min(self.tabs.len() - 1)
                        };
                        self.tab_drag = Some(target);
                        self.publish_panes();
                    }
                }
            }
        }

        if add {
            self.new_tab();
        }
        if let Some(i) = close {
            self.active = i;
            self.close_active();
        } else if let Some(i) = click {
            self.active = i;
            self.selection = None;
            self.scroll = 0;
        }
    }

    fn find_bar(&mut self, ctx: &egui::Context) {
        let query = self.find_query.to_lowercase();
        let matches = self
            .tabs
            .get(self.active)
            .and_then(|t| t.focused())
            .map(|p| find_matches(p.term.screen(), &query).len())
            .unwrap_or(0);
        let fg = self.theme.fg;
        let mut open = true;
        egui::TopBottomPanel::top("find")
            .frame(
                egui::Frame::default()
                    .fill(self.bg())
                    .inner_margin(egui::Margin::same(4.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Find").color(fg));
                    ui.text_edit_singleline(&mut self.find_query)
                        .request_focus();
                    ui.label(
                        egui::RichText::new(format!("{matches} matches"))
                            .color(egui::Color32::from_gray(150)),
                    );
                    if ui.button(self.t("Close")).clicked() {
                        open = false;
                    }
                });
            });
        if !open {
            self.find_open = false;
        }
    }

    fn terminal_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(self.window_bg()))
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                // Cell size comes from the renderer's own font so glyphs line up.
                let line_height = (self.font_size * self.line_height_ratio).round();
                let (cw, ch) =
                    self.metrics
                        .cell(self.font_size, line_height, self.font_family.as_deref());
                if cw <= 0.0 || ch <= 0.0 {
                    return;
                }

                // Window-level shortcuts (⌘ on macOS).
                ctx.input(|i| {
                    if self.palette.is_some() {
                        return;
                    }
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
                                    (egui::Key::T, false, _) => self.new_tab(),
                                    (egui::Key::W, _, _) => self.close_active(),
                                    (egui::Key::D, true, _) => self.split_active(SplitDir::Down),
                                    (egui::Key::D, false, true) => {
                                        self.show_details = !self.show_details
                                    }
                                    (egui::Key::D, false, false) => {
                                        self.split_active(SplitDir::Right)
                                    }
                                    (egui::Key::ArrowRight, _, true) => self.focus_cycle(true),
                                    (egui::Key::ArrowLeft, _, true) => self.focus_cycle(false),
                                    (egui::Key::CloseBracket, _, _) => self.focus_cycle(true),
                                    (egui::Key::OpenBracket, _, _) => self.focus_cycle(false),
                                    (egui::Key::F, _, _) => self.find_open = !self.find_open,
                                    (egui::Key::Plus, _, _) | (egui::Key::Equals, _, _) => {
                                        self.font_size += 1.0
                                    }
                                    (egui::Key::Minus, _, _) => {
                                        self.font_size = (self.font_size - 1.0).max(6.0)
                                    }
                                    (egui::Key::Num0, _, _) => self.font_size = 13.0,
                                    (egui::Key::Comma, _, _) => {
                                        self.show_settings = true;
                                        self.settings_family =
                                            self.font_family.clone().unwrap_or_default();
                                    }
                                    (egui::Key::E, true, _) => self.open_composer(),
                                    (egui::Key::T, true, _) => self.toggle_quick_terminal(),
                                    // Command palette — Otty uses ⌘⇧P (we also keep ⌘K).
                                    (egui::Key::K, _, _)
                                    | (egui::Key::P, true, _)
                                    | (egui::Key::O, true, _) => {
                                        self.palette = Some(Palette {
                                            query: String::new(),
                                            selected: 0,
                                        });
                                        self.palette_focus = true;
                                    }
                                    // Otty-style panel toggles.
                                    (egui::Key::L, true, _) => {
                                        self.show_sidebar = !self.show_sidebar
                                    }
                                    (egui::Key::R, true, _) => {
                                        self.show_details = !self.show_details
                                    }
                                    // ⌘1–9 switch tabs (Otty).
                                    (egui::Key::Num1, _, _) => {
                                        self.run_palette(PaletteAction::SwitchTab(0))
                                    }
                                    (egui::Key::Num2, _, _) => {
                                        self.run_palette(PaletteAction::SwitchTab(1))
                                    }
                                    (egui::Key::Num3, _, _) => {
                                        self.run_palette(PaletteAction::SwitchTab(2))
                                    }
                                    (egui::Key::Num4, _, _) => {
                                        self.run_palette(PaletteAction::SwitchTab(3))
                                    }
                                    (egui::Key::Num5, _, _) => {
                                        self.run_palette(PaletteAction::SwitchTab(4))
                                    }
                                    (egui::Key::Num6, _, _) => {
                                        self.run_palette(PaletteAction::SwitchTab(5))
                                    }
                                    (egui::Key::Num7, _, _) => {
                                        self.run_palette(PaletteAction::SwitchTab(6))
                                    }
                                    (egui::Key::Num8, _, _) => {
                                        self.run_palette(PaletteAction::SwitchTab(7))
                                    }
                                    (egui::Key::Num9, _, _) => {
                                        self.run_palette(PaletteAction::SwitchTab(8))
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                });

                // Draggable split dividers.
                let ti = self.active;
                let mut handles = Vec::new();
                {
                    let mut path = Vec::new();
                    collect_splits(&self.tabs[ti].layout, rect, &mut path, &mut handles);
                }
                if !handles.is_empty() {
                    let (pos, down, pressed) = ctx.input(|i| {
                        (
                            i.pointer.interact_pos(),
                            i.pointer.primary_down(),
                            i.pointer.primary_pressed(),
                        )
                    });
                    if pressed {
                        if let Some(p) = pos {
                            if let Some(h) =
                                handles.iter().find(|h| h.divider.expand(2.0).contains(p))
                            {
                                self.split_drag = Some(h.path.clone());
                            }
                        }
                    }
                    if !down {
                        self.split_drag = None;
                    }
                    if let (Some(p), Some(path)) = (pos, self.split_drag.clone()) {
                        if let Some(h) = handles.iter().find(|h| h.path == path) {
                            let ratio = match h.dir {
                                SplitDir::Right => (p.x - h.rect.left()) / h.rect.width().max(1.0),
                                SplitDir::Down => (p.y - h.rect.top()) / h.rect.height().max(1.0),
                            };
                            set_ratio(&mut self.tabs[ti].layout, &path, ratio.clamp(0.1, 0.9));
                        }
                    }
                    if let Some(p) = pos {
                        if let Some(h) = handles.iter().find(|h| h.divider.expand(2.0).contains(p))
                        {
                            ctx.set_cursor_icon(match h.dir {
                                SplitDir::Right => egui::CursorIcon::ResizeHorizontal,
                                SplitDir::Down => egui::CursorIcon::ResizeVertical,
                            });
                        }
                    }
                }

                // Pane rectangles for the active tab (from the split tree).
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
        // Otty-style container: a rounded card inset from the pane edges, with
        // the terminal grid drawn on padding inside the card.
        let outer = rect;
        let card = outer.shrink(6.0);
        let rect = card.shrink2(egui::vec2(12.0, 8.0));
        {
            let painter = ui.painter_at(outer);
            painter.rect_filled(card, egui::Rounding::same(10.0), self.bg());
            painter.rect_stroke(
                card,
                egui::Rounding::same(10.0),
                egui::Stroke::new(1.0_f32, self.theme.palette[8]),
            );
        }

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
            card,
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

            let (app_cursor, bracketed, kitty) = {
                let screen = self.tabs[ti].panes[pi].term.screen();
                (
                    screen.application_cursor(),
                    screen.bracketed_paste(),
                    screen.kitty_disambiguate(),
                )
            };
            // Enable the OS input method while the terminal is focused (and no
            // other widget is capturing text), anchoring the candidate window at
            // the cursor cell. Each command is sent only when it changes:
            // re-sending `IMEAllowed(true)` every frame resets the input method
            // and makes it flicker.
            if self.ime_enabled && ctx.memory(|m| m.focused().is_none()) {
                if !self.ime_allowed {
                    ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(true));
                    self.ime_allowed = true;
                }
                let (crow, ccol) = self.tabs[ti].panes[pi].term.screen().cursor_position();
                let ime_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        rect.left() + ccol as f32 * cw,
                        rect.top() + crow as f32 * ch,
                    ),
                    egui::vec2(cw, ch),
                );
                if self.ime_rect != Some(ime_rect) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::IMERect(ime_rect));
                    self.ime_rect = Some(ime_rect);
                }
            } else if self.ime_allowed {
                ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(false));
                self.ime_allowed = false;
                self.ime_rect = None;
            }

            let mut out = Vec::new();
            // `Preedit` is only emitted when it changes, so persist the last one
            // instead of clearing it every frame (which would flash).
            let mut preedit: Option<String> = None;
            let has_selection = self.selection.is_some();
            if self.palette.is_none() {
                ctx.input(|i| {
                    for ev in &i.events {
                        if let egui::Event::Ime(egui::ImeEvent::Preedit(t)) = ev {
                            preedit = Some(t.clone());
                        }
                        encode_input(ev, app_cursor, bracketed, kitty, has_selection, &mut out);
                    }
                });
            }
            if let Some(p) = preedit {
                self.ime_preedit = p;
            }
            if !out.is_empty() {
                self.tabs[ti].panes[pi].term.write(&out);
                self.ime_preedit.clear();
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
        let mut need_prepare = false;
        {
            let pane = &mut self.tabs[ti].panes[pi];
            let screen = pane.term.screen();
            draw_cursor = focused
                && self.scroll == 0
                && self.cursor_on
                && !screen.hide_cursor()
                && self.ime_preedit.is_empty();
            draw_screen(
                ui,
                screen,
                cw,
                ch,
                rect,
                if focused { self.selection } else { None },
                &self.theme,
                self.cursor_style,
                draw_cursor,
            );
            // Inline IME composition (preedit), underlined at the cursor cell.
            if focused && !self.ime_preedit.is_empty() {
                let (crow, ccol) = screen.cursor_position();
                let pos = egui::pos2(
                    rect.left() + ccol as f32 * cw,
                    rect.top() + crow as f32 * ch,
                );
                let painter = ui.painter_at(rect);
                let galley = painter.layout_no_wrap(
                    self.ime_preedit.clone(),
                    egui::FontId::monospace(self.font_size),
                    self.theme.fg,
                );
                let w = galley.size().x.max(cw);
                painter.rect_filled(
                    egui::Rect::from_min_size(pos, egui::vec2(w, ch)),
                    egui::Rounding::ZERO,
                    self.theme.bg,
                );
                painter.galley(pos, galley, self.theme.fg);
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(pos.x, pos.y + ch - 2.0),
                        egui::pos2(pos.x + w, pos.y + ch),
                    ),
                    egui::Rounding::ZERO,
                    self.theme.fg,
                );
            }
            if focused && !self.find_query.is_empty() {
                let painter = ui.painter_at(rect);
                for (frow, start, end) in find_matches(screen, &self.find_query.to_lowercase()) {
                    for col in start..end.min(cols) {
                        let r = egui::Rect::from_min_size(
                            egui::pos2(
                                rect.left() + col as f32 * cw,
                                rect.top() + frow as f32 * ch,
                            ),
                            egui::vec2(cw, ch),
                        );
                        painter.rect_filled(r, egui::Rounding::ZERO, FIND);
                    }
                }
            }
            let cursor_cell =
                if draw_cursor && self.cursor_style == miao_term_config::CursorStyle::Block {
                    Some(screen.cursor_position())
                } else {
                    None
                };
            if pane.dirty {
                pane.rows = Arc::new(build_rows(screen, &self.theme, cursor_cell));
                pane.dirty = false;
                need_prepare = true;
            }
        }
        let rows = Arc::clone(&self.tabs[ti].panes[pi].rows);
        let scale = ctx.pixels_per_point();
        let fg = self.theme.fg;
        ui.painter().add(egui::Shape::Callback(
            egui_wgpu::Callback::new_paint_callback(
                rect,
                TermCallback {
                    pane_id: pane_id.clone(),
                    prepare: need_prepare,
                    rows,
                    size: [rect.width(), rect.height()],
                    scale,
                    font_size: self.font_size,
                    line_height: ch,
                    default_color: (fg.r(), fg.g(), fg.b()),
                    family: self.font_family.clone(),
                },
            ),
        ));
    }
}

fn word_selection(screen: &ATerm, row: u16, col: u16, cols: u16) -> Selection {
    let is_word = |c: u16| -> bool {
        screen
            .cell(row, c)
            .map(|cell| cell.ch.is_alphanumeric() || "_-./~".contains(cell.ch))
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
    screen: &ATerm,
    cw: f32,
    ch: f32,
    rect: egui::Rect,
    selection: Option<Selection>,
    theme: &Theme,
    cursor_style: miao_term_config::CursorStyle,
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
            let bg = map_color(&cell.bg, false, theme);
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
            let width = screen
                .cell(crow, ccol)
                .map(|c| c.ch.width().unwrap_or(1))
                .unwrap_or(1)
                .max(1) as f32;
            let cur_rect = egui::Rect::from_min_size(
                egui::pos2(ox + ccol as f32 * cw, oy + crow as f32 * ch),
                egui::vec2(cw * width, ch),
            );
            match cursor_style {
                miao_term_config::CursorStyle::Block => {
                    painter.rect_filled(cur_rect, egui::Rounding::ZERO, theme.fg);
                }
                miao_term_config::CursorStyle::Bar => {
                    let bar = egui::Rect::from_min_size(cur_rect.min, egui::vec2(2.0, ch));
                    painter.rect_filled(bar, egui::Rounding::ZERO, theme.fg);
                }
                miao_term_config::CursorStyle::Underline => {
                    let line = egui::Rect::from_min_max(
                        egui::pos2(cur_rect.left(), cur_rect.bottom() - 2.0),
                        cur_rect.max,
                    );
                    painter.rect_filled(line, egui::Rounding::ZERO, theme.fg);
                }
            }
        }
    }

    // Scrollback indicator.
    let scrollback = screen.scrollback_len();
    if scrollback > 0 {
        let total = (scrollback + rows as usize) as f32;
        let track = rect.height();
        let thumb = (track * rows as f32 / total).max(16.0);
        let pos = screen.scroll_offset() as f32 / scrollback as f32;
        let y = rect.top() + (track - thumb) * (1.0 - pos);
        let bar =
            egui::Rect::from_min_size(egui::pos2(rect.right() - 6.0, y), egui::vec2(4.0, thumb));
        painter.rect_filled(bar, egui::Rounding::ZERO, egui::Color32::from_gray(110));
    }
}

/// Build per-row, per-color runs from the screen for the GPU renderer.
fn build_rows(
    screen: &ATerm,
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
            let ch = cell.ch;
            let width = ch.width().unwrap_or(0).max(1) as u16;
            let mut buf = [0u8; 4];
            let text = ch.encode_utf8(&mut buf).to_string();
            // The cell under a block cursor is drawn in the background color so
            // it reads as inverted against the cursor block.
            let color = if cell.inverse || cursor == Some((row, col)) {
                let c = theme.bg;
                (c.r(), c.g(), c.b())
            } else {
                let c = map_color(&cell.fg, true, theme);
                (c.r(), c.g(), c.b())
            };
            match spans.last_mut() {
                Some(last) if last.color == color => last.text.push_str(&text),
                _ => spans.push(miao_term_render::Span::new(text, color)),
            }
            col += width;
        }
        out.push(spans);
    }
    out
}

/// One `TermRenderer` per pane, so each pane keeps its own prepared glyphs and
/// we can skip re-preparing panes whose content did not change (damage).
struct PaneRenderers {
    format: egui_wgpu::wgpu::TextureFormat,
    map: std::collections::HashMap<String, miao_term_render::TermRenderer>,
}

/// egui→wgpu paint callback that draws the terminal glyphs via `term-render`.
struct TermCallback {
    pane_id: String,
    /// Re-shape glyphs this frame (the pane's rows changed).
    prepare: bool,
    rows: Arc<Vec<Vec<miao_term_render::Span>>>,
    /// Pane grid size in logical points (the callback's viewport).
    size: [f32; 2],
    scale: f32,
    font_size: f32,
    line_height: f32,
    default_color: (u8, u8, u8),
    family: Option<String>,
}

impl egui_wgpu::CallbackTrait for TermCallback {
    fn prepare(
        &self,
        device: &egui_wgpu::wgpu::Device,
        queue: &egui_wgpu::wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut egui_wgpu::wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<egui_wgpu::wgpu::CommandBuffer> {
        if let Some(store) = resources.get_mut::<PaneRenderers>() {
            let format = store.format;
            let renderer = store
                .map
                .entry(self.pane_id.clone())
                .or_insert_with(|| miao_term_render::TermRenderer::new(device, queue, format));
            if self.prepare {
                // egui-wgpu sets the render viewport to this callback's rect, so
                // glyphon must be told that viewport's pixel size and draw with
                // viewport-relative coordinates (origin 0,0) — not the full
                // surface with absolute coordinates.
                renderer.prepare(
                    device,
                    queue,
                    (
                        (self.size[0] * self.scale) as u32,
                        (self.size[1] * self.scale) as u32,
                    ),
                    self.scale,
                    self.font_size,
                    self.line_height,
                    0.0,
                    0.0,
                    self.default_color,
                    self.family.as_deref(),
                    self.rows.as_slice(),
                );
            }
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::epaint::PaintCallbackInfo,
        pass: &mut egui_wgpu::wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(store) = resources.get::<PaneRenderers>() {
            if let Some(renderer) = store.map.get(&self.pane_id) {
                renderer.render(pass);
            }
        }
    }
}

fn ctrl_byte(key: egui::Key) -> Option<u8> {
    use egui::Key::*;
    Some(match key {
        A => 0x01,
        B => 0x02,
        C => 0x03,
        D => 0x04,
        E => 0x05,
        F => 0x06,
        G => 0x07,
        H => 0x08,
        I => 0x09,
        J => 0x0a,
        K => 0x0b,
        L => 0x0c,
        M => 0x0d,
        N => 0x0e,
        O => 0x0f,
        P => 0x10,
        Q => 0x11,
        R => 0x12,
        S => 0x13,
        T => 0x14,
        U => 0x15,
        V => 0x16,
        W => 0x17,
        X => 0x18,
        Y => 0x19,
        Z => 0x1a,
        _ => return None,
    })
}

/// Base codepoint for a printable key (lowercase), for kitty CSI-u encoding.
fn key_codepoint(key: egui::Key) -> Option<u32> {
    use egui::Key::*;
    Some(match key {
        A => 'a' as u32,
        B => 'b' as u32,
        C => 'c' as u32,
        D => 'd' as u32,
        E => 'e' as u32,
        F => 'f' as u32,
        G => 'g' as u32,
        H => 'h' as u32,
        I => 'i' as u32,
        J => 'j' as u32,
        K => 'k' as u32,
        L => 'l' as u32,
        M => 'm' as u32,
        N => 'n' as u32,
        O => 'o' as u32,
        P => 'p' as u32,
        Q => 'q' as u32,
        R => 'r' as u32,
        S => 's' as u32,
        T => 't' as u32,
        U => 'u' as u32,
        V => 'v' as u32,
        W => 'w' as u32,
        X => 'x' as u32,
        Y => 'y' as u32,
        Z => 'z' as u32,
        Num0 => '0' as u32,
        Num1 => '1' as u32,
        Num2 => '2' as u32,
        Num3 => '3' as u32,
        Num4 => '4' as u32,
        Num5 => '5' as u32,
        Num6 => '6' as u32,
        Num7 => '7' as u32,
        Num8 => '8' as u32,
        Num9 => '9' as u32,
        Space => ' ' as u32,
        _ => return None,
    })
}

fn encode_input(
    ev: &egui::Event,
    app_cursor: bool,
    bracketed: bool,
    kitty: bool,
    has_selection: bool,
    out: &mut Vec<u8>,
) {
    match ev {
        egui::Event::Text(t) => {
            out.extend_from_slice(t.as_bytes());
        }
        // OS input method (e.g. Pinyin / Kotoeri): send the committed text to the
        // shell. Pre-edit text is shown by the IME overlay, not sent.
        egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
            out.extend_from_slice(text.as_bytes());
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
            // Kitty keyboard protocol: disambiguate special keys and Ctrl+keys
            // as CSI-u (`ESC [ code ; mods u`).
            if kitty && !modifiers.mac_cmd {
                let mut n: u8 = 1;
                if modifiers.shift {
                    n += 1;
                }
                if modifiers.alt {
                    n += 2;
                }
                if modifiers.ctrl {
                    n += 4;
                }
                let special = match key {
                    egui::Key::Escape => Some(27u32),
                    egui::Key::Enter => Some(13),
                    egui::Key::Tab => Some(9),
                    egui::Key::Backspace => Some(127),
                    _ => None,
                };
                if let Some(cp) = special {
                    if n > 1 {
                        out.extend_from_slice(format!("\x1b[{cp};{n}u").as_bytes());
                        return;
                    }
                }
                if modifiers.ctrl {
                    if let Some(cp) = key_codepoint(*key) {
                        out.extend_from_slice(format!("\x1b[{cp};{n}u").as_bytes());
                        return;
                    }
                }
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

            // Backspace variants: Ctrl+Backspace = ^W (delete word), Alt+Backspace = ESC DEL.
            if *key == egui::Key::Backspace {
                if modifiers.ctrl && !modifiers.alt {
                    out.push(0x17);
                    return;
                }
                if modifiers.alt && !modifiers.ctrl {
                    out.extend_from_slice(b"\x1b\x7f");
                    return;
                }
            }

            // Modifier-aware navigation (word movement, selection, etc.).
            if modifiers.shift || modifiers.alt || modifiers.ctrl {
                let mut n: u8 = 1;
                if modifiers.shift {
                    n += 1;
                }
                if modifiers.alt {
                    n += 2;
                }
                if modifiers.ctrl {
                    n += 4;
                }
                let seq = match key {
                    egui::Key::ArrowUp => Some(format!("\x1b[1;{n}A")),
                    egui::Key::ArrowDown => Some(format!("\x1b[1;{n}B")),
                    egui::Key::ArrowRight => Some(format!("\x1b[1;{n}C")),
                    egui::Key::ArrowLeft => Some(format!("\x1b[1;{n}D")),
                    egui::Key::Home => Some(format!("\x1b[1;{n}H")),
                    egui::Key::End => Some(format!("\x1b[1;{n}F")),
                    egui::Key::Delete => Some(format!("\x1b[3;{n}~")),
                    egui::Key::PageUp => Some(format!("\x1b[5;{n}~")),
                    egui::Key::PageDown => Some(format!("\x1b[6;{n}~")),
                    _ => None,
                };
                if let Some(seq) = seq {
                    out.extend_from_slice(seq.as_bytes());
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

#[cfg(test)]
mod palette_tests {
    use super::palette_score;

    #[test]
    fn resume_prefers_explicit_then_session() {
        let explicit = serde_json::json!({ "resume": "claude -c", "session_id": "abc" });
        assert_eq!(
            super::resume_command(&explicit, "claude").as_deref(),
            Some("claude -c")
        );
        let session = serde_json::json!({ "session_id": "abc" });
        assert_eq!(
            super::resume_command(&session, "codex").as_deref(),
            Some("codex resume abc")
        );
        assert!(super::resume_command(&serde_json::json!({}), "claude").is_none());
    }

    #[test]
    fn scores_substring_then_subsequence() {
        assert_eq!(palette_score("New Tab", "command", ""), Some(0));
        assert_eq!(palette_score("New Tab", "command", "new"), Some(0));
        assert!(palette_score("New Tab", "command", "nt").is_some());
        assert!(palette_score("New Tab", "command", "xyz").is_none());
    }

    #[test]
    fn earlier_substring_position_ranks_first() {
        assert!(palette_score("New Tab", "command", "new") < palette_score("Renew", "tab", "new"));
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn tab_session_defaults_new_fields() {
        let old = r#"{"title":"t","panes":[{"cwd":null}],"layout":{"Leaf":0},"active":0}"#;
        let ts: TabSession = serde_json::from_str(old).unwrap();
        assert!(ts.prefix.is_none() && ts.mark.is_none() && ts.group.is_none());
    }

    #[test]
    fn tab_session_round_trips_grouping() {
        let ts = TabSession {
            title: "t".into(),
            prefix: Some("[w]".into()),
            mark: Some("*".into()),
            group: Some("g".into()),
            panes: vec![PaneSession { cwd: None }],
            layout: LayoutNode::Leaf(0),
            active: 0,
        };
        let json = serde_json::to_string(&ts).unwrap();
        let back: TabSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.prefix.as_deref(), Some("[w]"));
        assert_eq!(back.group.as_deref(), Some("g"));
    }

    #[test]
    fn platform_install_scripts() {
        let win = windows_install_script(
            4242,
            std::path::Path::new(r"C:\Users\me\Downloads\miaotty-0.1.0-x86_64.msi"),
            std::path::Path::new(r"C:\Program Files\miaotty\bin\miaotty.exe"),
        );
        assert!(win.contains("PID eq 4242"), "waits for our pid");
        assert!(win.contains("msiexec /i"), "runs the installer");
        assert!(win.contains("miaotty-cli.exe"), "replaces the CLI too");

        let lin = linux_install_script(
            4242,
            std::path::Path::new("/opt/miaotty.AppImage"),
            std::path::Path::new("/tmp/downloaded.AppImage"),
        );
        assert!(lin.starts_with("#!/bin/sh"));
        assert!(lin.contains("kill -0 4242"), "waits for our pid");
        assert!(lin.contains("exec \"/opt/miaotty.AppImage\""), "relaunches");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn bundle_root_and_install_script() {
        let exe = std::path::Path::new("/Applications/miaotty.app/Contents/MacOS/miaotty");
        assert_eq!(
            bundle_root(exe).as_deref(),
            Some(std::path::Path::new("/Applications/miaotty.app"))
        );
        assert!(bundle_root(std::path::Path::new("/tmp/target/debug/miaotty")).is_none());

        let script = install_script(
            42,
            std::path::Path::new("/Applications/miaotty.app"),
            std::path::Path::new("/tmp/new/miaotty.app"),
        );
        assert!(script.contains("while kill -0 42"));
        assert!(script.contains("mv \"/Applications/miaotty.app\" \"$old\""));
        assert!(script.contains("open \"/Applications/miaotty.app\""));
    }

    #[test]
    fn markdown_images_and_footnotes() {
        assert_eq!(parse_image("![alt](img.png)"), Some(("alt", "img.png")));
        assert!(parse_image("text ![a](b)").is_none());
        let uri = image_uri("img.png", Some(std::path::Path::new("/doc"))).unwrap();
        assert!(uri.starts_with("file:///"), "{uri}");
        assert!(uri.ends_with("img.png"), "{uri}");
        assert!(!uri.contains('\\'), "{uri}");
        assert!(image_uri("https://x/y.png", None).is_none());
        assert!(image_uri("script.rs", Some(std::path::Path::new("/doc"))).is_none());

        let lines = ["a", "[^1]: note one", "b [^1]"];
        let (body, defs) = take_footnotes(&lines);
        assert_eq!(body, vec!["a", "b [^1]"]);
        assert_eq!(defs, vec![("1".to_string(), "note one".to_string())]);
        assert_eq!(footnote_refs("see [^1] and [^x]"), "see [1] and [x]");
    }

    #[test]
    fn gutter_numbering() {
        assert_eq!(gutter_numbers(&[true, true, false]), vec![1, 2, 3]);
        assert_eq!(gutter_numbers(&[false, true]), vec![1, 1]);
        assert!(gutter_numbers(&[]).is_empty());
    }

    #[test]
    fn markdown_tables() {
        assert!(is_table_sep("| --- | :--: |"));
        assert!(is_table_sep("|---|"));
        assert!(!is_table_sep("| a | b |"));
        assert_eq!(
            split_row("| a | b |"),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            table_align("|:--|--:|:-:|"),
            vec![egui::Align::LEFT, egui::Align::RIGHT, egui::Align::Center]
        );
    }

    #[test]
    fn markdown_links_and_rules() {
        assert!(is_rule("---"));
        assert!(is_rule(" *** "));
        assert!(!is_rule("--"));
        assert_eq!(
            link_segments("see [docs](https://x) now"),
            vec![
                ("see ".to_string(), None),
                ("docs".to_string(), Some("https://x".to_string())),
                (" now".to_string(), None),
            ]
        );
        assert_eq!(link_segments("plain"), vec![("plain".to_string(), None)]);
    }

    #[test]
    fn markdown_helpers() {
        assert_eq!(heading("# Hi"), Some((1, "Hi")));
        assert_eq!(heading("### Deep"), Some((3, "Deep")));
        assert_eq!(heading("#nospace"), None);
        assert_eq!(bullet("- item"), Some("item"));
        assert_eq!(bullet("  * item"), Some("item"));
        assert_eq!(strip_inline("a**b** `c` _d_"), "ab c d");
    }

    #[test]
    fn sanitize_recipe_names() {
        assert_eq!(sanitize_name("my work/1"), "my work_1");
    }
}

/// Performance gate for the hot app paths (ADR 0018). Run with
/// `cargo test -p miaotty-app --release -- --ignored`.
#[cfg(test)]
mod perf_tests {
    use super::*;

    /// Record a measured metric for the CI baseline comparison (ADR 0028).
    fn record_metric(key: &str, value: f64) {
        // Walk up to the workspace root (the dir holding Cargo.lock) so both
        // crates write the same file.
        let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = loop {
            if dir.join("Cargo.lock").is_file() {
                break dir.join("target/perf-measured.json");
            }
            match dir.parent() {
                Some(p) => dir = p.to_path_buf(),
                None => break dir.join("target/perf-measured.json"),
            }
        };
        let mut map: std::collections::BTreeMap<String, f64> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        map.insert(key.to_string(), value);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(
            &path,
            serde_json::to_string_pretty(&map).unwrap_or_default(),
        );
    }

    fn scale() -> f64 {
        std::env::var("MIAOTTY_PERF_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0)
    }

    /// Compare a measured metric against the committed baseline and fail on a
    /// regression beyond `regression_pct` (ADR 0023).
    fn baseline_gate(key: &str, measured: f64, higher_is_better: bool) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../benches/budgets.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return;
        };
        let Some(entry) = value.get(key) else { return };
        let Some(base) = entry.get("baseline").and_then(|b| b.as_f64()) else {
            return;
        };
        let pct = entry
            .get("regression_pct")
            .and_then(|p| p.as_f64())
            .unwrap_or(25.0);
        let limit = if higher_is_better {
            base * (1.0 - pct / 100.0)
        } else {
            base * (1.0 + pct / 100.0)
        };
        let ok = if higher_is_better {
            measured >= limit
        } else {
            measured <= limit
        };
        assert!(
            ok,
            "{key}: measured {measured:.4} regressed beyond {pct}% of baseline {base:.4} (limit {limit:.4})"
        );
    }

    #[test]
    #[ignore = "perf gate; run `cargo test --release -- --ignored`"]
    fn build_rows_frame_budget() {
        let mut screen = ATerm::new(100, 30, 10_000);
        for _ in 0..40 {
            screen.process(b"\x1b[31mred\x1b[0m \x1b[32mgreen\x1b[0m text 1234567890\r\n");
        }
        let theme = Theme::from_config(&miao_term_config::Theme::default());
        let cursor = Some(screen.cursor());
        std::hint::black_box(build_rows(&screen, &theme, cursor));
        let n = 300;
        let start = Instant::now();
        for _ in 0..n {
            std::hint::black_box(build_rows(&screen, &theme, cursor));
        }
        let per_ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("build_rows: {per_ms:.3} ms/frame");
        assert!(
            per_ms <= 4.0 * scale(),
            "row build {per_ms:.3} ms exceeds the 4 ms/frame budget"
        );
        baseline_gate("build_rows_frame_ms", per_ms / scale(), false);
        record_metric("build_rows_frame_ms", per_ms / scale());
    }

    #[test]
    #[ignore = "perf gate; run `cargo test --release -- --ignored`"]
    fn palette_ranking_budget() {
        let entries: Vec<PaletteEntry> = (0..10_000)
            .map(|i| PaletteEntry {
                kind: "file".to_string(),
                label: format!("file-{i}.rs"),
                icon: None,
                action: PaletteAction::SwitchTab(0),
            })
            .collect();
        let n = 20;
        let start = Instant::now();
        for _ in 0..n {
            std::hint::black_box(rank_entries(&entries, "file-9"));
        }
        let per_ms = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("palette rank over 10k entries: {per_ms:.3} ms");
        assert!(
            per_ms <= 100.0 * scale(),
            "palette ranking {per_ms:.3} ms exceeds the 100 ms budget"
        );
        baseline_gate("palette_rank_10k_ms", per_ms / scale(), false);
        record_metric("palette_rank_10k_ms", per_ms / scale());
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(update::is_newer("1.2.0", "1.1.9"));
        assert!(update::is_newer("v2.0.0", "1.9.9"));
        assert!(!update::is_newer("1.0.0", "1.0.0"));
        assert!(!update::is_newer("0.9.9", "1.0.0"));
    }
}
