//! Small hand-drawn UI icons (no font glyphs, so they always render).

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
}

/// Draw `icon` centered in `rect`.
pub fn draw(p: &egui::Painter, rect: egui::Rect, icon: Icon, color: egui::Color32) {
    let s = egui::Stroke::new(1.2_f32, color);
    let r = rect.shrink(2.0);
    let c = r.center();
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([egui::pos2(a.0, a.1), egui::pos2(b.0, b.1)], s);
    };
    match icon {
        Icon::Terminal => {
            p.rect_stroke(r, egui::Rounding::same(2.0), s);
            let (l, t) = (r.left() + 3.0, r.top() + 4.0);
            line((l, t), (l + 3.0, t + 2.5));
            line((l + 3.0, t + 2.5), (l, t + 5.0));
            line((l + 6.0, t + 5.0), (l + 9.0, t + 5.0));
        }
        Icon::Sidebar => {
            p.rect_stroke(r, egui::Rounding::same(2.0), s);
            let x = r.left() + 4.0;
            line((x, r.top()), (x, r.bottom()));
        }
        Icon::Details => {
            p.rect_stroke(r, egui::Rounding::same(2.0), s);
            let x = r.right() - 4.0;
            line((x, r.top()), (x, r.bottom()));
        }
        Icon::Plus => {
            line((c.x, r.top() + 2.0), (c.x, r.bottom() - 2.0));
            line((r.left() + 2.0, c.y), (r.right() - 2.0, c.y));
        }
        Icon::Info => {
            p.circle_stroke(c, r.width() * 0.45, s);
            p.circle_filled(egui::pos2(c.x, c.y - 3.0), 1.0, color);
            line((c.x, c.y - 1.0), (c.x, c.y + 3.0));
        }
        Icon::Agent => {
            p.circle_stroke(egui::pos2(c.x, c.y - 3.0), r.width() * 0.22, s);
            line((c.x - 4.0, r.bottom() - 2.0), (c.x + 4.0, r.bottom() - 2.0));
            line((c.x - 4.0, r.bottom() - 2.0), (c.x - 2.5, c.y + 1.0));
            line((c.x + 4.0, r.bottom() - 2.0), (c.x + 2.5, c.y + 1.0));
        }
        Icon::Outline => {
            for k in 0..3 {
                let y = r.top() + 3.0 + k as f32 * 3.5;
                line((r.left() + 2.0, y), (r.right() - 2.0, y));
            }
        }
        Icon::Git => {
            let x = c.x - 3.0;
            line((x, r.top() + 2.0), (x, r.bottom() - 2.0));
            p.circle_stroke(egui::pos2(x, r.top() + 3.0), 1.7, s);
            p.circle_stroke(egui::pos2(x, r.bottom() - 3.0), 1.7, s);
            line((x, c.y), (c.x + 3.0, c.y));
            p.circle_stroke(egui::pos2(c.x + 3.0, c.y - 2.0), 1.7, s);
        }
        Icon::Files => {
            let fr = egui::Rect::from_min_max(
                egui::pos2(r.left() + 1.0, r.top() + 3.0),
                egui::pos2(r.right() - 1.0, r.bottom() - 1.0),
            );
            p.rect_stroke(fr, egui::Rounding::same(2.0), s);
            line(
                (r.left() + 1.0, r.top() + 3.0),
                (r.left() + 4.0, r.top() + 3.0),
            );
        }
        Icon::Ports => {
            p.circle_stroke(egui::pos2(c.x, c.y - 2.0), 2.5, s);
            line((c.x - 3.0, c.y + 1.0), (c.x + 3.0, c.y + 1.0));
            line((c.x, c.y + 1.0), (c.x, r.bottom() - 2.0));
        }
        Icon::Queue => {
            for k in 0..3 {
                let y = r.top() + 3.0 + k as f32 * 3.5;
                p.circle_filled(egui::pos2(r.left() + 3.0, y), 1.0, color);
                line((r.left() + 6.0, y), (r.right() - 2.0, y));
            }
        }
    }
}

/// A small icon button (allocates space, draws the icon, hover highlight).
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, color: egui::Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 18.0), egui::Sense::click());
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
