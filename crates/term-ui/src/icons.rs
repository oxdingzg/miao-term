//! UI icons, rendered from the Tabler Icons font (MIT) so they are crisp and
//! consistent. The host registers the `tabler` family.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Terminal,
    Info,
    Agent,
    Outline,
    Git,
    Files,
    Ports,
    Queue,
    Sidebar,
    Details,
    Plus,
    Folder,
    File,
    Server,
    GitBranch,
    Command,
    Search,
    Refresh,
}

/// The Tabler Icons glyph for an icon.
pub fn glyph(icon: Icon) -> char {
    match icon {
        Icon::Terminal => '\u{ebdc}',
        Icon::Info => '\u{eac5}',                  // info-circle
        Icon::Agent => '\u{f00b}',                 // robot
        Icon::Outline => '\u{eb6b}',               // list
        Icon::Git | Icon::GitBranch => '\u{eab2}', // git-branch
        Icon::Files | Icon::Folder => '\u{eaad}',  // folder
        Icon::Ports => '\u{ebd9}',                 // plug
        Icon::Queue => '\u{eb6a}',                 // list-check
        Icon::Sidebar => '\u{eada}',               // layout-sidebar
        Icon::Details => '\u{ead4}',               // layout-columns
        Icon::Plus => '\u{eb0b}',
        Icon::File => '\u{eaa4}',
        Icon::Server => '\u{eb1f}',
        Icon::Command => '\u{ea78}',
        Icon::Search => '\u{eb1c}',
        Icon::Refresh => '\u{eb13}',
    }
}

/// Draw `icon` centered in `rect`, filled with `color`.
pub fn draw(p: &egui::Painter, rect: egui::Rect, icon: Icon, color: egui::Color32) {
    let size = rect.height().min(rect.width()).max(8.0);
    let font = egui::FontId::new(size, egui::FontFamily::Name("tabler".into()));
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph(icon),
        font,
        color,
    );
}

/// A small icon button (allocates space, draws the icon, hover highlight).
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, color: egui::Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 20.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() {
            ui.painter().rect_filled(
                rect,
                egui::Rounding::same(4.0),
                ui.visuals().widgets.hovered.bg_fill,
            );
        }
        draw(ui.painter(), rect, icon, color);
    }
    resp
}
