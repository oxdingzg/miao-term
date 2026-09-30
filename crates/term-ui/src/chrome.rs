//! Reusable egui chrome widgets, shared by both hosts. They only take plain
//! data and return actions, so they are independent of any host's state type.

use crate::theme::{Chrome as ChromeColors, Rgb};

pub fn fg_color(t: &crate::theme::Theme) -> egui::Color32 {
    egui::Color32::from_rgb(t.fg.0, t.fg.1, t.fg.2)
}

pub fn bg_color(c: Rgb) -> egui::Color32 {
    egui::Color32::from_rgb(c.0, c.1, c.2)
}

pub fn section(text: &str) -> egui::RichText {
    egui::RichText::new(text.to_uppercase())
        .size(10.0)
        .strong()
        .color(egui::Color32::from_gray(140))
}

/// What the user did in the tab bar.
#[derive(Default)]
pub struct TabBarEvents {
    pub switch: Option<usize>,
    pub close: Option<usize>,
    pub rename: Option<usize>,
    pub reorder: Option<(usize, usize)>,
    pub duplicate: Option<usize>,
    pub close_others: Option<usize>,
    pub close_below: Option<usize>,
    pub move_up: Option<usize>,
    pub move_down: Option<usize>,
    pub set_prefix: Option<usize>,
    pub new_tab: bool,
}

/// A horizontal tab bar: clickable, draggable labels, a close affordance, `+`.
pub fn tab_bar(
    ui: &mut egui::Ui,
    ch: &ChromeColors,
    titles: &[String],
    icons: &[crate::icons::Icon],
    active: usize,
    lang: Lang,
) -> TabBarEvents {
    const DRAG_ID: &str = "miao_tab_drag";
    let mut ev = TabBarEvents::default();
    ui.visuals_mut().selection.bg_fill = bg_color(ch.active);
    ui.visuals_mut().override_text_color = Some(bg_color(ch.text));
    let font = egui::FontId::proportional(13.0);
    let text_color = bg_color(ch.text);
    let mut rects: Vec<egui::Rect> = Vec::with_capacity(titles.len());
    for (i, title) in titles.iter().enumerate() {
        let icon = icons[i];
        let galley = ui
            .painter()
            .layout_no_wrap(title.clone(), font.clone(), text_color);
        let closable = titles.len() > 1;
        let extra = if closable { 46.0 } else { 30.0 };
        let desired = egui::vec2(galley.size().x + extra, 22.0);
        let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
        let bg = if i == active {
            bg_color(ch.active)
        } else if resp.hovered() {
            bg_color(ch.hover)
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 4.0, bg);
        let ir = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 12.0, rect.center().y),
            egui::Vec2::splat(14.0),
        );
        crate::icons::draw(ui.painter(), ir, icon, text_color);
        let pos = rect.min + egui::vec2(22.0, (rect.height() - galley.size().y) * 0.5);
        ui.painter().galley(pos, galley, text_color);
        // Close affordance, inside the chip.
        if closable {
            let xr = egui::Rect::from_center_size(
                egui::pos2(rect.right() - 11.0, rect.center().y),
                egui::Vec2::splat(14.0),
            );
            let x_resp = ui.interact(xr, ui.id().with(("tabclose", i)), egui::Sense::click());
            let ccol = if x_resp.hovered() {
                text_color
            } else {
                egui::Color32::from_gray(150)
            };
            let (a, b) = (xr.shrink(4.0), xr.shrink(4.0));
            let stroke = egui::Stroke::new(1.4_f32, ccol);
            ui.painter()
                .line_segment([a.left_top(), b.right_bottom()], stroke);
            ui.painter()
                .line_segment([a.right_top(), b.left_bottom()], stroke);
            if x_resp.clicked() {
                ev.close = Some(i);
            }
        }
        if ev.close.is_none() && resp.clicked() {
            ev.switch = Some(i);
        }
        if resp.double_clicked() {
            ev.rename = Some(i);
        }
        if resp.drag_started() {
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(egui::Id::new(DRAG_ID), i));
        }
        rects.push(rect);
        ui.add_space(2.0);
    }
    // Resolve a finished drag against the collected chip rects.
    if ui.ctx().input(|i| i.pointer.any_released()) {
        let from = ui
            .ctx()
            .memory_mut(|m| m.data.remove_temp::<usize>(egui::Id::new(DRAG_ID)));
        if let (Some(from), Some(p)) = (from, ui.ctx().pointer_interact_pos()) {
            let mut target = from;
            for (j, r) in rects.iter().enumerate() {
                if p.x >= r.left() && p.x <= r.right() {
                    target = j;
                    break;
                }
            }
            if target != from {
                ev.reorder = Some((from, target));
            }
        }
    }
    if ui
        .button("+")
        .on_hover_text(t(lang, "New Tab", "新建标签"))
        .clicked()
    {
        ev.new_tab = true;
    }
    ev
}

/// A vertical session list for the sidebar.
#[allow(clippy::too_many_arguments)]
pub fn sidebar(
    ui: &mut egui::Ui,
    ch: &ChromeColors,
    titles: &[String],
    icons: &[crate::icons::Icon],
    badges: &[Option<Rgb>],
    metas: &[String],
    active: usize,
    heading: &str,
) -> Option<usize> {
    let mut switch = None;
    ui.visuals_mut().selection.bg_fill = bg_color(ch.active);
    ui.visuals_mut().override_text_color = Some(bg_color(ch.text));
    ui.label(section(&format!("{heading} ({})", titles.len())));
    ui.separator();
    for (i, title) in titles.iter().enumerate() {
        ui.horizontal(|ui| {
            let (irect, _) = ui.allocate_exact_size(egui::Vec2::splat(14.0), egui::Sense::hover());
            crate::icons::draw(
                ui.painter(),
                irect,
                icons
                    .get(i)
                    .copied()
                    .unwrap_or(crate::icons::Icon::Terminal),
                bg_color(ch.text),
            );
            match badges.get(i).copied().flatten() {
                Some(c) => {
                    ui.colored_label(bg_color(c), "\u{25cf}");
                }
                None => {
                    ui.label("  ");
                }
            }
            let resp = ui.selectable_label(i == active, egui::RichText::new(title).size(13.0));
            if resp.clicked() {
                switch = Some(i);
            }
            if let Some(m) = metas.get(i).filter(|m| !m.is_empty()) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(m)
                            .size(10.5)
                            .color(egui::Color32::from_gray(120)),
                    );
                });
            }
        });
    }
    switch
}

/// A row of selectable details tabs; returns the newly selected index.
pub fn details_tabs(
    ui: &mut egui::Ui,
    ch: &ChromeColors,
    tabs: &[(crate::icons::Icon, &str)],
    active: usize,
) -> Option<usize> {
    let mut sel = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, (icon, label)) in tabs.iter().enumerate() {
            // Icon-only tabs (the label is a tooltip), like the reference app.
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 22.0), egui::Sense::click());
            let bg = if i == active {
                Some(bg_color(ch.active))
            } else if resp.hovered() {
                Some(bg_color(ch.hover))
            } else {
                None
            };
            if let Some(b) = bg {
                ui.painter().rect_filled(rect, egui::Rounding::same(5.0), b);
            }
            let ir = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0));
            crate::icons::draw(ui.painter(), ir, *icon, bg_color(ch.text));
            if resp.on_hover_text(*label).clicked() {
                sel = Some(i);
            }
        }
    });
    sel
}

/// Actions from the prompt-queue widget.
#[derive(Default)]
pub struct QueueEvents {
    pub add: bool,
    pub send: Option<usize>,
    pub remove: Option<usize>,
    pub send_all: bool,
    pub clear: bool,
}

/// A minimal prompt queue: type a prompt, queue it, then send to the shell.
pub fn queue(
    ui: &mut egui::Ui,
    ch: &ChromeColors,
    items: &[String],
    input: &mut String,
    lang: Lang,
) -> QueueEvents {
    let mut ev = QueueEvents::default();
    ui.visuals_mut().override_text_color = Some(bg_color(ch.text));
    ui.label(section(&format!(
        "{} ({})",
        t(lang, "Queue", "队列"),
        items.len()
    )));
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(input)
                .hint_text(t(lang, "Prompt to run…", "要执行的提示…"))
                .desired_width(160.0),
        );
        if ui.button(t(lang, "Add", "添加")).clicked() {
            ev.add = true;
        }
        if ui.button(t(lang, "Send All", "全部发送")).clicked() {
            ev.send_all = true;
        }
        if ui.button(t(lang, "Clear", "清空")).clicked() {
            ev.clear = true;
        }
    });
    ui.separator();
    for (i, item) in items.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui
                .small_button("\u{25b6}")
                .on_hover_text(t(lang, "Send", "发送"))
                .clicked()
            {
                ev.send = Some(i);
            }
            if ui.small_button("\u{00d7}").clicked() {
                ev.remove = Some(i);
            }
            ui.label(egui::RichText::new(item).monospace().size(12.0));
        });
    }
    ev
}

/// A two-column label/value info list (details panel).
pub fn info(ui: &mut egui::Ui, ch: &ChromeColors, title: &str, rows: &[(String, String)]) {
    ui.visuals_mut().override_text_color = Some(bg_color(ch.text));
    ui.label(section(title));
    ui.add_space(4.0);
    for (k, v) in rows {
        ui.label(
            egui::RichText::new(k)
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
        ui.label(egui::RichText::new(v).monospace().size(12.0));
        ui.add_space(2.0);
    }
}

use crate::i18n::{t, Lang};

fn panel_frame(ch: &ChromeColors, margin: egui::Margin) -> egui::Frame {
    panel_frame_fill(ch, margin, ch.bg)
}

/// Like [`panel_frame`] but with an explicit surface colour, so the session
/// list and the details inspector can sit above/below the terminal card.
fn panel_frame_fill(ch: &ChromeColors, margin: egui::Margin, fill: Rgb) -> egui::Frame {
    panel_frame_stroke(margin, fill, ch.hover)
}

/// A panel frame with an explicit fill and stroke (the side panels use Otty's
/// border colour so their edge reads as a separator, like `[sidebar]`).
fn panel_frame_stroke(margin: egui::Margin, fill: Rgb, stroke: Rgb) -> egui::Frame {
    egui::Frame::default()
        .fill(bg_color(fill))
        .stroke(egui::Stroke::new(1.0_f32, bg_color(stroke)))
        .inner_margin(margin)
}

/// One row in a details list (Files / Ports / Git / Outline): a single line
/// with an icon, a label and right-aligned meta.
pub struct ChromeItem {
    pub icon: crate::icons::Icon,
    pub label: String,
    pub meta: String,
}

/// Render `items` as a compact one-line-per-row list.
pub fn list(ui: &mut egui::Ui, ch: &ChromeColors, items: &[ChromeItem]) {
    ui.visuals_mut().override_text_color = Some(bg_color(ch.text));
    let muted = egui::Color32::from_gray(132);
    for it in items {
        ui.horizontal(|ui| {
            let (irect, _) = ui.allocate_exact_size(egui::Vec2::splat(14.0), egui::Sense::hover());
            crate::icons::draw(ui.painter(), irect, it.icon, bg_color(ch.text));
            ui.add_space(2.0);
            ui.label(egui::RichText::new(&it.label).monospace().size(12.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !it.meta.is_empty() {
                    ui.label(egui::RichText::new(&it.meta).size(10.5).color(muted));
                }
            });
        });
    }
}

/// A menu command id; each host maps it to its own action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuId {
    NewTab,
    ClosePane,
    OpenFile,
    Save,
    SaveRecipe,
    OpenRecipe,
    NewSsh,
    OpenRemote,
    Composer,
    QuickTerminal,
    CheckUpdates,
    Copy,
    Paste,
    SplitRight,
    SplitDown,
    ToggleSidebar,
    ToggleDetails,
    FontUp,
    FontDown,
    FontReset,
    Settings,
    Palette,
    Find,
    DuplicateTab,
    ReopenClosed,
    ClearScrollback,
    SelectAll,
    CopyAnsi,
    PasteEscaped,
    FindNext,
    FindPrev,
    UseSelForFind,
    JumpToSel,
    FindInAllTabs,
    Fullscreen,
    ClearScreen,
    ReadOnly,
    HintMode,
    Pip,
    CopyPath,
    RevealCwd,
    Quit,
}

/// One tab as the chrome needs it.
pub struct ChromeTab {
    pub title: String,
    pub badge: Option<Rgb>,
    pub icon: crate::icons::Icon,
}

/// The host implements this; [`render`] draws the surrounding UI from it and
/// calls the `on_*` methods to report user actions.
#[allow(unused_variables)]
pub trait Chrome {
    fn lang(&self) -> Lang {
        Lang::En
    }
    fn tabs(&self) -> Vec<ChromeTab> {
        Vec::new()
    }
    fn active_tab(&self) -> usize {
        0
    }
    fn show_sidebar(&self) -> bool {
        true
    }
    fn show_details(&self) -> bool {
        true
    }
    fn details_tab(&self) -> usize {
        0
    }
    fn details_title(&self) -> String {
        String::new()
    }
    fn details_rows(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    fn details_is_queue(&self) -> bool {
        false
    }
    fn read_only(&self) -> bool {
        false
    }
    /// A short right-side status chip (running program / agent), if any.
    fn status_right(&self) -> String {
        String::new()
    }
    /// Whether the host draws the menu bar inside the window. macOS inside an
    /// app bundle returns false: the menu lives in the system menu bar there
    /// (ADR 0031).
    fn draws_menu_bar(&self) -> bool {
        true
    }
    /// The host's active theme (colours for the whole chrome).
    fn theme(&self) -> crate::theme::Theme {
        crate::theme::Theme::default()
    }
    /// A single-line list view for list-like tabs; `None` falls back to k/v.
    fn details_list(&self) -> Option<Vec<ChromeItem>> {
        None
    }
    /// Let the host render the whole details body (e.g. a file tree). Return
    /// true if it drew something.
    fn details_body(&mut self, ui: &mut egui::Ui, lang: Lang) -> bool {
        let _ = (ui, lang);
        false
    }
    fn status(&self) -> String {
        String::new()
    }
    fn queue(&self) -> Vec<String> {
        Vec::new()
    }
    fn take_queue_input(&mut self) -> String {
        String::new()
    }
    fn set_queue_input(&mut self, input: String) {}

    fn on_new_tab(&mut self) {}
    fn on_switch_tab(&mut self, i: usize) {}
    fn on_close_tab(&mut self, i: usize) {}
    fn on_rename_tab(&mut self, i: usize) {}
    fn on_reorder_tab(&mut self, from: usize, to: usize) {}
    fn on_duplicate_tab(&mut self, i: usize) {}
    fn on_close_others(&mut self, i: usize) {}
    fn on_close_below(&mut self, i: usize) {}
    fn on_move_tab(&mut self, i: usize, delta: i32) {}
    fn on_set_prefix(&mut self, i: usize) {}
    fn on_font_delta(&mut self, delta: f32) {}
    fn on_toggle_sidebar(&mut self) {}
    fn on_toggle_details(&mut self) {}
    fn on_details_tab(&mut self, i: usize) {}
    fn on_queue_add(&mut self) {}
    fn on_queue_send(&mut self, i: usize) {}
    fn on_queue_remove(&mut self, i: usize) {}
    fn on_queue_send_all(&mut self) {}
    fn on_queue_clear(&mut self) {}
    fn on_menu(&mut self, id: MenuId) {}

    /// Screen-space rect of every pane in the active tab. Used to place the
    /// per-pane close button; empty when the host has no split panes.
    fn pane_close_rects(&self) -> Vec<(String, egui::Rect)> {
        Vec::new()
    }
    fn on_close_pane(&mut self, id: &str) {
        let _ = id;
    }
}

pub const CHROME_MENU_H: f32 = 24.0;
pub const CHROME_TAB_H: f32 = 30.0;
pub const CHROME_STATUS_H: f32 = 22.0;
pub const CHROME_SIDEBAR_W: f32 = 200.0;
pub const CHROME_DETAILS_W: f32 = 300.0;

/// Draw the whole surrounding UI (menu, tabs, sidebar, details, status).
pub fn render(ctx: &egui::Context, host: &mut impl Chrome) {
    use crate::icons::{icon_button, Icon};
    let theme = host.theme();
    let ch = theme.chrome();
    let lang = host.lang();

    // Snapshot (owned), so nothing borrows the host while egui closures run.
    let tabs = host.tabs();
    let titles: Vec<String> = tabs.iter().map(|t| t.title.clone()).collect();
    let badges: Vec<Option<Rgb>> = tabs.iter().map(|t| t.badge).collect();
    let icons: Vec<crate::icons::Icon> = tabs.iter().map(|t| t.icon).collect();
    let metas: Vec<String> = (0..titles.len())
        .map(|i| {
            if i < 9 {
                format!("\u{2318}{}", i + 1)
            } else {
                String::new()
            }
        })
        .collect();
    let active = host.active_tab();
    let show_sidebar = host.show_sidebar();
    let show_details = host.show_details();
    let details_tab = host.details_tab();
    let details_title = host.details_title();
    let details_rows = host.details_rows();
    let details_is_queue = host.details_is_queue();
    let host_read_only = host.read_only();
    let details_list = host.details_list();
    let status = host.status();
    let status_right = host.status_right();
    let queue_items = host.queue();
    let mut queue_input = host.take_queue_input();

    let mut menu: Option<MenuId> = None;
    let mut switch = None;
    let mut close = None;
    let mut rename = None;
    let mut reorder = None;
    let mut duplicate = None;
    let mut close_others = None;
    let mut close_below = None;
    let mut move_up = None;
    let mut move_down = None;
    let mut set_prefix = None;
    let mut new_tab = false;
    let mut font_delta = 0.0f32;
    let mut toggle_sidebar = false;
    let mut toggle_details = false;
    let mut details_sel = None;
    let mut qev = QueueEvents::default();

    let menu_item = |ui: &mut egui::Ui, label: &str, id: MenuId, out: &mut Option<MenuId>| {
        if ui.button(label).clicked() {
            *out = Some(id);
            ui.close_menu();
        }
    };

    if host.draws_menu_bar() {
        let menu_table = crate::menu::menus(lang);
        egui::TopBottomPanel::top("menu")
            .exact_height(CHROME_MENU_H)
            .frame(panel_frame(&ch, egui::Margin::symmetric(6.0, 1.0)))
            .show(ctx, |ui| {
                // Menu-bar styling: transparent idle, subtle rounded hover/active.
                {
                    let v = ui.visuals_mut();
                    v.override_text_color = Some(bg_color(ch.text));
                    v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                    v.widgets.hovered.weak_bg_fill = bg_color(ch.hover);
                    v.widgets.active.weak_bg_fill = bg_color(ch.active);
                    v.widgets.hovered.bg_fill = bg_color(ch.hover);
                    v.widgets.active.bg_fill = bg_color(ch.active);
                    let r = egui::Rounding::same(5.0);
                    v.widgets.inactive.rounding = r;
                    v.widgets.hovered.rounding = r;
                    v.widgets.active.rounding = r;
                }
                ui.style_mut().spacing.button_padding = egui::vec2(8.0, 3.0);
                ui.style_mut().spacing.item_spacing.x = 2.0;
                ui.horizontal(|ui| {
                    for (title, entries) in &menu_table {
                        ui.menu_button(*title, |ui| {
                            for entry in entries {
                                match entry {
                                    crate::menu::Entry::Item { label, id, .. } => {
                                        let label = if *id == MenuId::ReadOnly && host_read_only {
                                            format!("{label}  \u{2713}")
                                        } else {
                                            label.clone()
                                        };
                                        menu_item(ui, &label, *id, &mut menu);
                                    }
                                    crate::menu::Entry::Separator => {
                                        ui.separator();
                                    }
                                    crate::menu::Entry::Link { label, url } => {
                                        ui.hyperlink_to(label, url);
                                    }
                                }
                            }
                        });
                    }
                });
            });
    }

    egui::TopBottomPanel::top("tabs")
        .exact_height(CHROME_TAB_H)
        .frame(panel_frame(&ch, egui::Margin::symmetric(6.0, 3.0)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let ev = tab_bar(ui, &ch, &titles, &icons, active, lang);
                switch = ev.switch;
                close = ev.close;
                rename = ev.rename;
                reorder = ev.reorder;
                duplicate = ev.duplicate;
                close_others = ev.close_others;
                close_below = ev.close_below;
                move_up = ev.move_up;
                move_down = ev.move_down;
                set_prefix = ev.set_prefix;
                new_tab = ev.new_tab;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("A+").clicked() {
                        font_delta = 1.0;
                    }
                    if ui.button("A-").clicked() {
                        font_delta = -1.0;
                    }
                    if icon_button(ui, Icon::Sidebar, bg_color(ch.text))
                        .on_hover_text(t(lang, "Toggle sidebar", "开关侧栏"))
                        .clicked()
                    {
                        toggle_sidebar = true;
                    }
                    if icon_button(ui, Icon::Details, bg_color(ch.text))
                        .on_hover_text(t(lang, "Toggle details", "开关详情"))
                        .clicked()
                    {
                        toggle_details = true;
                    }
                });
            });
        });

    if show_sidebar {
        egui::SidePanel::left("sessions")
            .exact_width(CHROME_SIDEBAR_W)
            .frame(panel_frame_stroke(
                egui::Margin::same(6.0),
                ch.sidebar,
                ch.border,
            ))
            .show(ctx, |ui| {
                let heading = t(lang, "Sessions", "会话");
                if let Some(i) = sidebar(ui, &ch, &titles, &icons, &badges, &metas, active, heading)
                {
                    switch = Some(i);
                }
            });
    }

    if show_details {
        let host = &mut *host;
        let tabs_icons = [
            (Icon::Info, t(lang, "Info", "信息")),
            (Icon::Agent, "Agent"),
            (Icon::Outline, t(lang, "Outline", "大纲")),
            (Icon::Git, "Git"),
            (Icon::Files, t(lang, "Files", "文件")),
            (Icon::Ports, t(lang, "Ports", "端口")),
            (Icon::Queue, t(lang, "Queue", "队列")),
        ];
        egui::SidePanel::right("details")
            .exact_width(CHROME_DETAILS_W)
            .frame(panel_frame_stroke(
                egui::Margin::same(8.0),
                ch.details,
                ch.border,
            ))
            .show(ctx, |ui| {
                if let Some(i) = details_tabs(ui, &ch, &tabs_icons, details_tab) {
                    details_sel = Some(i);
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if !host.details_body(ui, lang) {
                            if details_is_queue {
                                qev = queue(ui, &ch, &queue_items, &mut queue_input, lang);
                            } else if let Some(items) = &details_list {
                                list(ui, &ch, items);
                            } else {
                                info(ui, &ch, &details_title, &details_rows);
                            }
                        }
                    });
            });
    }

    egui::TopBottomPanel::bottom("status")
        .exact_height(CHROME_STATUS_H)
        .frame(panel_frame(&ch, egui::Margin::symmetric(8.0, 2.0)))
        .show(ctx, |ui| {
            ui.visuals_mut().override_text_color = Some(bg_color(ch.text));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(status).size(11.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!("\u{2318}K  {}", t(lang, "commands", "命令")))
                            .size(11.0)
                            .color(egui::Color32::from_gray(130)),
                    );
                    if !status_right.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("\u{25cf} {status_right}"))
                                .size(11.0)
                                .color(egui::Color32::from_rgb(0xa3, 0xbe, 0x8c)),
                        );
                        ui.add_space(10.0);
                    }
                });
            });
        });

    // Apply.
    host.set_queue_input(queue_input);
    if let Some(id) = menu {
        host.on_menu(id);
    }
    if let Some(i) = switch {
        host.on_switch_tab(i);
    }
    // Per-pane close button, top-right of each pane. Only when the tab is
    // split — otherwise the tab's own close affordance covers it.
    let mut close_pane: Option<String> = None;
    {
        let panes = host.pane_close_rects();
        if panes.len() > 1 {
            for (id, r) in &panes {
                egui::Area::new(egui::Id::new(("pane-close", id)))
                    .order(egui::Order::Foreground)
                    .fixed_pos(egui::pos2(r.max.x - 24.0, r.min.y + 6.0))
                    .show(ctx, |ui| {
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(18.0, 16.0), egui::Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                egui::Rounding::same(4.0),
                                bg_color(ch.hover),
                            );
                        }
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "\u{00d7}",
                            egui::FontId::proportional(13.0),
                            if resp.hovered() {
                                bg_color(ch.text)
                            } else {
                                bg_color(ch.muted)
                            },
                        );
                        if resp.clicked() {
                            close_pane = Some(id.clone());
                        }
                    });
            }
        }
    }
    if let Some(id) = close_pane {
        host.on_close_pane(&id);
    }

    if let Some(i) = close {
        host.on_close_tab(i);
    }
    if let Some(i) = rename {
        host.on_rename_tab(i);
    }
    if let Some((from, to)) = reorder {
        host.on_reorder_tab(from, to);
    }
    if let Some(i) = duplicate {
        host.on_duplicate_tab(i);
    }
    if let Some(i) = close_others {
        host.on_close_others(i);
    }
    if let Some(i) = close_below {
        host.on_close_below(i);
    }
    if let Some(i) = move_up {
        host.on_move_tab(i, -1);
    }
    if let Some(i) = move_down {
        host.on_move_tab(i, 1);
    }
    if let Some(i) = set_prefix {
        host.on_set_prefix(i);
    }
    if new_tab {
        host.on_new_tab();
    }
    if font_delta != 0.0 {
        host.on_font_delta(font_delta);
    }
    if toggle_sidebar {
        host.on_toggle_sidebar();
    }
    if toggle_details {
        host.on_toggle_details();
    }
    if let Some(i) = details_sel {
        host.on_details_tab(i);
    }
    if qev.add {
        host.on_queue_add();
    }
    if let Some(i) = qev.send {
        host.on_queue_send(i);
    }
    if let Some(i) = qev.remove {
        host.on_queue_remove(i);
    }
    if qev.send_all {
        host.on_queue_send_all();
    }
    if qev.clear {
        host.on_queue_clear();
    }
}
