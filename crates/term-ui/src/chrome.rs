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
    pub new_tab: bool,
}

/// A horizontal tab bar: clickable labels, a close affordance, and `+`.
pub fn tab_bar(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    titles: &[String],
    active: usize,
) -> TabBarEvents {
    let mut ev = TabBarEvents::default();
    ui.visuals_mut().selection.bg_fill = bg_color(theme.palette[4]);
    ui.visuals_mut().override_text_color = Some(fg_color(theme));
    for (i, title) in titles.iter().enumerate() {
        let resp = ui.selectable_label(i == active, title);
        if resp.clicked() {
            ev.switch = Some(i);
        }
        if resp.double_clicked() {
            ev.rename = Some(i);
        }
        if titles.len() > 1 && ui.small_button("\u{00d7}").clicked() {
            ev.close = Some(i);
        }
        ui.add_space(2.0);
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
    active: usize,
) -> Option<usize> {
    let mut switch = None;
    ui.visuals_mut().selection.bg_fill = bg_color(theme.palette[4]);
    ui.visuals_mut().override_text_color = Some(fg_color(theme));
    ui.label(section(&format!("Sessions ({})", titles.len())));
    ui.separator();
    for (i, title) in titles.iter().enumerate() {
        if ui
            .selectable_label(i == active, egui::RichText::new(title).size(13.0))
            .clicked()
        {
            switch = Some(i);
        }
    }
    switch
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
