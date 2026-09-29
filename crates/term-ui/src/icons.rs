//! UI icons, rendered from the Phosphor Icons font (MIT) so they are crisp and
//! consistent. The host registers the `ph` (fill) and `ph-bold` families.

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

/// The Phosphor (fill) glyph for an icon.
pub fn glyph(icon: Icon) -> char {
    match icon {
        Icon::Terminal => '\u{e47e}',
        Icon::Info => '\u{e2ce}',
        Icon::Agent => '\u{e762}',   // robot
        Icon::Outline => '\u{e2f0}', // list
        Icon::Git | Icon::GitBranch => '\u{e278}',
        Icon::Files | Icon::Folder => '\u{e24a}', // folder
        Icon::Ports => '\u{e946}',                // plug
        Icon::Queue => '\u{eadc}',                // list-checks
        Icon::Sidebar => '\u{ec24}',              // sidebar-simple
        Icon::Details => '\u{e546}',              // columns
        Icon::Plus => '\u{e3d4}',
        Icon::File => '\u{e230}',
        Icon::Server => '\u{e2a0}', // hard-drives
        Icon::Command => '\u{e1c4}',
        Icon::Search => '\u{e30c}',  // magnifying-glass
        Icon::Refresh => '\u{e094}', // arrows-clockwise
    }
}

/// Draw `icon` centered in `rect`, filled with `color`.
pub fn draw(p: &egui::Painter, rect: egui::Rect, icon: Icon, color: egui::Color32) {
    let size = rect.height().min(rect.width()).max(8.0);
    let font = egui::FontId::new(size, egui::FontFamily::Name("ph".into()));
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
