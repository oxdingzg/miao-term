//! Reusable egui chrome widgets, shared by both hosts. They only take plain
//! data and return actions, so they are independent of any host's state type.

use crate::theme::Rgb;

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
    pub new_tab: bool,
}

/// A horizontal tab bar: clickable, draggable labels, a close affordance, `+`.
pub fn tab_bar(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    titles: &[String],
    active: usize,
) -> TabBarEvents {
    const DRAG_ID: &str = "miao_tab_drag";
    let mut ev = TabBarEvents::default();
    ui.visuals_mut().selection.bg_fill = bg_color(theme.palette[4]);
    ui.visuals_mut().override_text_color = Some(fg_color(theme));
    let font = egui::FontId::proportional(13.0);
    let text_color = fg_color(theme);
    let mut rects: Vec<egui::Rect> = Vec::with_capacity(titles.len());
    for (i, title) in titles.iter().enumerate() {
        let galley = ui
            .painter()
            .layout_no_wrap(title.clone(), font.clone(), text_color);
        let desired = egui::vec2(galley.size().x + 16.0, 22.0);
        let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
        let bg = if i == active {
            bg_color(theme.palette[4])
        } else if resp.hovered() {
            bg_color(lighten(theme.bg, 0.10))
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 4.0, bg);
        let pos = rect.min + egui::vec2(8.0, (rect.height() - galley.size().y) * 0.5);
        ui.painter().galley(pos, galley, text_color);
        if resp.clicked() {
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
        if titles.len() > 1 && ui.small_button("\u{00d7}").clicked() {
            ev.close = Some(i);
        }
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
    if ui.button("+").on_hover_text("New Tab").clicked() {
        ev.new_tab = true;
    }
    ev
}

/// A vertical session list for the sidebar.
pub fn sidebar(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    titles: &[String],
    badges: &[Option<Rgb>],
    active: usize,
    heading: &str,
) -> Option<usize> {
    let mut switch = None;
    ui.visuals_mut().selection.bg_fill = bg_color(theme.palette[4]);
    ui.visuals_mut().override_text_color = Some(fg_color(theme));
    ui.label(section(&format!("{heading} ({})", titles.len())));
    ui.separator();
    for (i, title) in titles.iter().enumerate() {
        ui.horizontal(|ui| {
            let (irect, _) = ui.allocate_exact_size(egui::Vec2::splat(14.0), egui::Sense::hover());
            crate::icons::draw(
                ui.painter(),
                irect,
                crate::icons::Icon::Terminal,
                fg_color(theme),
            );
            match badges.get(i).copied().flatten() {
                Some(c) => {
                    ui.colored_label(bg_color(c), "\u{25cf}");
                }
                None => {
                    ui.label("  ");
                }
            }
            if ui
                .selectable_label(i == active, egui::RichText::new(title).size(13.0))
                .clicked()
            {
                switch = Some(i);
            }
        });
    }
    switch
}

/// A row of selectable details tabs; returns the newly selected index.
pub fn details_tabs(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    tabs: &[(crate::icons::Icon, &str)],
    active: usize,
) -> Option<usize> {
    let mut sel = None;
    ui.visuals_mut().selection.bg_fill = bg_color(theme.palette[4]);
    ui.visuals_mut().override_text_color = Some(fg_color(theme));
    ui.horizontal_wrapped(|ui| {
        for (i, (icon, l)) in tabs.iter().enumerate() {
            // Reserve leading space so the icon sits left of the label text.
            let resp = ui.selectable_label(i == active, format!("    {l}"));
            if resp.clicked() {
                sel = Some(i);
            }
            let ir = egui::Rect::from_center_size(
                egui::pos2(resp.rect.left() + 10.0, resp.rect.center().y),
                egui::Vec2::splat(13.0),
            );
            crate::icons::draw(ui.painter(), ir, *icon, fg_color(theme));
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
    theme: &crate::theme::Theme,
    items: &[String],
    input: &mut String,
) -> QueueEvents {
    let mut ev = QueueEvents::default();
    ui.visuals_mut().override_text_color = Some(fg_color(theme));
    ui.label(section(&format!("Queue ({})", items.len())));
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(input)
                .hint_text("Prompt to run…")
                .desired_width(160.0),
        );
        if ui.button("Add").clicked() {
            ev.add = true;
        }
        if ui.button("Send All").clicked() {
            ev.send_all = true;
        }
        if ui.button("Clear").clicked() {
            ev.clear = true;
        }
    });
    ui.separator();
    for (i, item) in items.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.small_button("\u{25b6}").on_hover_text("Send").clicked() {
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
pub fn info(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    title: &str,
    rows: &[(String, String)],
) {
    ui.visuals_mut().override_text_color = Some(fg_color(theme));
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

fn lighten(c: Rgb, f: f32) -> Rgb {
    let l = |v: u8| (v as f32 + (255.0 - v as f32) * f).clamp(0.0, 255.0) as u8;
    Rgb(l(c.0), l(c.1), l(c.2))
}

fn panel_frame(theme: &crate::theme::Theme, margin: egui::Margin) -> egui::Frame {
    egui::Frame::default()
        .fill(bg_color(theme.bg))
        .stroke(egui::Stroke::new(
            1.0_f32,
            bg_color(lighten(theme.bg, 0.10)),
        ))
        .inner_margin(margin)
}

/// A menu command id; each host maps it to its own action.
#[derive(Clone, Copy, PartialEq, Eq)]
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
    CheckUpdates,
    Copy,
    Paste,
    SplitRight,
    SplitDown,
    ToggleSidebar,
    ToggleDetails,
    FontUp,
    FontDown,
    Settings,
    Quit,
}

/// One tab as the chrome needs it.
pub struct ChromeTab {
    pub title: String,
    pub badge: Option<Rgb>,
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
}

pub const CHROME_MENU_H: f32 = 24.0;
pub const CHROME_TAB_H: f32 = 30.0;
pub const CHROME_STATUS_H: f32 = 22.0;
pub const CHROME_SIDEBAR_W: f32 = 200.0;
pub const CHROME_DETAILS_W: f32 = 300.0;

/// Draw the whole surrounding UI (menu, tabs, sidebar, details, status).
pub fn render(ctx: &egui::Context, host: &mut impl Chrome) {
    use crate::icons::{icon_button, Icon};
    let theme = crate::theme::Theme::default();
    let lang = host.lang();

    // Snapshot (owned), so nothing borrows the host while egui closures run.
    let tabs = host.tabs();
    let titles: Vec<String> = tabs.iter().map(|t| t.title.clone()).collect();
    let badges: Vec<Option<Rgb>> = tabs.iter().map(|t| t.badge).collect();
    let active = host.active_tab();
    let show_sidebar = host.show_sidebar();
    let show_details = host.show_details();
    let details_tab = host.details_tab();
    let details_title = host.details_title();
    let details_rows = host.details_rows();
    let details_is_queue = host.details_is_queue();
    let status = host.status();
    let queue_items = host.queue();
    let mut queue_input = host.take_queue_input();

    let mut menu: Option<MenuId> = None;
    let mut switch = None;
    let mut close = None;
    let mut rename = None;
    let mut reorder = None;
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

    egui::TopBottomPanel::top("menu")
        .exact_height(CHROME_MENU_H)
        .frame(panel_frame(&theme, egui::Margin::symmetric(6.0, 1.0)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.visuals_mut().override_text_color = Some(fg_color(&theme));
                ui.menu_button(t(lang, "File", "文件"), |ui| {
                    menu_item(
                        ui,
                        t(lang, "New Tab", "新建标签"),
                        MenuId::NewTab,
                        &mut menu,
                    );
                    menu_item(
                        ui,
                        t(lang, "Close Pane / Tab", "关闭 Pane/标签"),
                        MenuId::ClosePane,
                        &mut menu,
                    );
                    ui.separator();
                    menu_item(
                        ui,
                        t(lang, "New SSH Session…", "新建 SSH 会话…"),
                        MenuId::NewSsh,
                        &mut menu,
                    );
                    menu_item(
                        ui,
                        t(lang, "Open Remote File…", "打开远端文件…"),
                        MenuId::OpenRemote,
                        &mut menu,
                    );
                    ui.separator();
                    menu_item(
                        ui,
                        t(lang, "Save Recipe…", "保存配方…"),
                        MenuId::SaveRecipe,
                        &mut menu,
                    );
                    menu_item(
                        ui,
                        t(lang, "Open Recipe…", "打开配方…"),
                        MenuId::OpenRecipe,
                        &mut menu,
                    );
                    ui.separator();
                    menu_item(
                        ui,
                        t(lang, "Open File…", "打开文件…"),
                        MenuId::OpenFile,
                        &mut menu,
                    );
                    menu_item(ui, t(lang, "Save", "保存"), MenuId::Save, &mut menu);
                    ui.separator();
                    menu_item(ui, t(lang, "Quit", "退出"), MenuId::Quit, &mut menu);
                });
                ui.menu_button(t(lang, "Edit", "编辑"), |ui| {
                    menu_item(ui, t(lang, "Copy", "复制"), MenuId::Copy, &mut menu);
                    menu_item(ui, t(lang, "Paste", "粘贴"), MenuId::Paste, &mut menu);
                });
                ui.menu_button(t(lang, "View", "视图"), |ui| {
                    menu_item(
                        ui,
                        t(lang, "Toggle Sidebar", "开关侧栏"),
                        MenuId::ToggleSidebar,
                        &mut menu,
                    );
                    menu_item(
                        ui,
                        t(lang, "Toggle Details", "开关详情"),
                        MenuId::ToggleDetails,
                        &mut menu,
                    );
                    ui.separator();
                    menu_item(
                        ui,
                        t(lang, "Increase Font Size", "增大字号"),
                        MenuId::FontUp,
                        &mut menu,
                    );
                    menu_item(
                        ui,
                        t(lang, "Decrease Font Size", "减小字号"),
                        MenuId::FontDown,
                        &mut menu,
                    );
                    menu_item(ui, t(lang, "Settings", "设置"), MenuId::Settings, &mut menu);
                });
                ui.menu_button(t(lang, "Shell", "终端"), |ui| {
                    menu_item(
                        ui,
                        t(lang, "Split Right", "向右分屏"),
                        MenuId::SplitRight,
                        &mut menu,
                    );
                    menu_item(
                        ui,
                        t(lang, "Split Down", "向下分屏"),
                        MenuId::SplitDown,
                        &mut menu,
                    );
                });
                ui.menu_button(t(lang, "Agent", "Agent"), |ui| {
                    menu_item(ui, "Composer", MenuId::Composer, &mut menu);
                });
                ui.menu_button(t(lang, "Help", "帮助"), |ui| {
                    menu_item(
                        ui,
                        t(lang, "Check for Updates", "检查更新"),
                        MenuId::CheckUpdates,
                        &mut menu,
                    );
                    ui.hyperlink_to(
                        t(lang, "Documentation", "文档"),
                        "https://github.com/oxdingzg/miao-term#readme",
                    );
                });
            });
        });

    egui::TopBottomPanel::top("tabs")
        .exact_height(CHROME_TAB_H)
        .frame(panel_frame(&theme, egui::Margin::symmetric(6.0, 3.0)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let ev = tab_bar(ui, &theme, &titles, active);
                switch = ev.switch;
                close = ev.close;
                rename = ev.rename;
                reorder = ev.reorder;
                new_tab = ev.new_tab;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("A+").clicked() {
                        font_delta = 1.0;
                    }
                    if ui.button("A-").clicked() {
                        font_delta = -1.0;
                    }
                    if icon_button(ui, Icon::Sidebar, fg_color(&theme))
                        .on_hover_text(t(lang, "Toggle sidebar", "开关侧栏"))
                        .clicked()
                    {
                        toggle_sidebar = true;
                    }
                    if icon_button(ui, Icon::Details, fg_color(&theme))
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
            .frame(panel_frame(&theme, egui::Margin::same(6.0)))
            .show(ctx, |ui| {
                let heading = t(lang, "Sessions", "会话");
                if let Some(i) = sidebar(ui, &theme, &titles, &badges, active, heading) {
                    switch = Some(i);
                }
            });
    }

    if show_details {
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
            .frame(panel_frame(&theme, egui::Margin::same(8.0)))
            .show(ctx, |ui| {
                if let Some(i) = details_tabs(ui, &theme, &tabs_icons, details_tab) {
                    details_sel = Some(i);
                }
                ui.separator();
                if details_is_queue {
                    qev = queue(ui, &theme, &queue_items, &mut queue_input);
                } else {
                    info(ui, &theme, &details_title, &details_rows);
                }
            });
    }

    egui::TopBottomPanel::bottom("status")
        .exact_height(CHROME_STATUS_H)
        .frame(panel_frame(&theme, egui::Margin::symmetric(8.0, 2.0)))
        .show(ctx, |ui| {
            ui.visuals_mut().override_text_color = Some(fg_color(&theme));
            ui.label(egui::RichText::new(status).size(11.0));
        });

    // Apply.
    host.set_queue_input(queue_input);
    if let Some(id) = menu {
        host.on_menu(id);
    }
    if let Some(i) = switch {
        host.on_switch_tab(i);
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
