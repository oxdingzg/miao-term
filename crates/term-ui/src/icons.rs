//! Hand-drawn UI icons in the reference app's style: compact, bold, mostly
//! filled shapes with a ~2px stroke, drawn with egui primitives (no font
//! glyphs, so they always render).

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

/// Draw `icon` centered in `rect`.
pub fn draw(p: &egui::Painter, rect: egui::Rect, icon: Icon, color: egui::Color32) {
    let s = egui::Stroke::new(1.9_f32, color);
    let r = rect.shrink(1.0);
    let c = r.center();
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([egui::pos2(a.0, a.1), egui::pos2(b.0, b.1)], s);
    };
    let tri = |pts: [(f32, f32); 3]| {
        p.add(egui::Shape::convex_polygon(
            pts.iter().map(|(x, y)| egui::pos2(*x, *y)).collect(),
            color,
            egui::Stroke::NONE,
        ));
    };
    match icon {
        Icon::Terminal => {
            p.rect_stroke(r, egui::Rounding::same(3.0), s);
            let (l, t) = (r.left() + 3.5, c.y - 4.0);
            tri([(l, t), (l + 4.0, t + 4.0), (l, t + 8.0)]);
            line((l + 6.5, t + 8.0), (l + 10.0, t + 8.0));
        }
        Icon::Sidebar => {
            p.rect_stroke(r, egui::Rounding::same(3.0), s);
            let x = r.left() + 4.5;
            line((x, r.top()), (x, r.bottom()));
        }
        Icon::Details => {
            p.rect_stroke(r, egui::Rounding::same(3.0), s);
            let x = r.right() - 4.5;
            line((x, r.top()), (x, r.bottom()));
        }
        Icon::Folder | Icon::Files => {
            let fr = egui::Rect::from_min_max(
                egui::pos2(r.left() + 0.5, r.top() + 3.5),
                egui::pos2(r.right() - 0.5, r.bottom() - 0.5),
            );
            p.rect_filled(fr, egui::Rounding::same(2.0), color);
            // The folder tab (a notch above the box, in the panel background).
            p.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(fr.left() + 0.5, fr.top() - 2.5),
                    egui::pos2(fr.left() + 4.0, fr.top() - 2.5),
                    egui::pos2(fr.left() + 5.5, fr.top() + 0.5),
                    egui::pos2(fr.left() + 0.5, fr.top() + 0.5),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        Icon::Server => {
            for k in 0..2 {
                let y = r.top() + 2.5 + k as f32 * 5.2;
                p.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(r.left() + 0.5, y),
                        egui::pos2(r.right() - 0.5, y + 3.6),
                    ),
                    egui::Rounding::same(1.4),
                    color,
                );
                p.circle_filled(
                    egui::pos2(r.right() - 3.5, y + 1.8),
                    0.9,
                    egui::Color32::from_black_alpha(200),
                );
            }
        }
        Icon::GitBranch | Icon::Git => {
            p.circle_filled(egui::pos2(c.x - 3.2, r.top() + 3.2), 2.4, color);
            p.circle_filled(egui::pos2(c.x - 3.2, r.bottom() - 3.2), 2.4, color);
            p.circle_filled(egui::pos2(c.x + 3.2, r.top() + 3.2), 2.4, color);
            line((c.x - 3.2, r.top() + 5.4), (c.x - 3.2, r.bottom() - 5.4));
            line((c.x - 3.2, c.y - 4.5), (c.x + 3.2, r.top() + 3.2));
        }
        Icon::File => {
            let (l, t, rr, b) = (
                r.left() + 2.5,
                r.top() + 1.5,
                r.right() - 2.5,
                r.bottom() - 1.5,
            );
            p.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(l, t),
                    egui::pos2(rr - 4.0, t),
                    egui::pos2(rr, t + 4.0),
                    egui::pos2(rr, b),
                    egui::pos2(l, b),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        Icon::Plus => {
            line((c.x, r.top() + 2.0), (c.x, r.bottom() - 2.0));
            line((r.left() + 2.0, c.y), (r.right() - 2.0, c.y));
        }
        Icon::Info => {
            p.circle_stroke(c, r.width() * 0.46, s);
            p.circle_filled(egui::pos2(c.x, c.y - 3.0), 1.3, color);
            line((c.x, c.y - 1.0), (c.x, c.y + 3.6));
        }
        Icon::Agent => {
            // Robot head: filled body with two knocked-out eyes + antenna.
            let body = egui::Rect::from_min_max(
                egui::pos2(r.left() + 1.5, r.top() + 4.0),
                egui::pos2(r.right() - 1.5, r.bottom() - 1.0),
            );
            p.rect_filled(body, egui::Rounding::same(3.0), color);
            line((c.x, r.top() + 0.8), (c.x, body.top()));
            p.circle_filled(egui::pos2(c.x, r.top() + 0.6), 1.2, color);
            let eye = egui::Color32::from_black_alpha(200);
            p.circle_filled(egui::pos2(c.x - 2.4, body.center().y), 1.3, eye);
            p.circle_filled(egui::pos2(c.x + 2.4, body.center().y), 1.3, eye);
        }
        Icon::Outline | Icon::Queue => {
            for k in 0..3 {
                let y = r.top() + 3.0 + k as f32 * 3.9;
                p.circle_filled(egui::pos2(r.left() + 2.2, y), 1.2, color);
                line((r.left() + 5.5, y), (r.right() - 1.0, y));
            }
        }
        Icon::Ports => {
            let body = egui::Rect::from_min_max(
                egui::pos2(c.x - 4.5, r.top() + 1.5),
                egui::pos2(c.x + 4.5, c.y),
            );
            p.rect_filled(body, egui::Rounding::same(2.0), color);
            line((c.x - 2.2, body.top()), (c.x - 2.2, r.top() - 0.5));
            line((c.x + 2.2, body.top()), (c.x + 2.2, r.top() - 0.5));
            line((c.x, body.bottom()), (c.x, r.bottom() - 0.5));
        }
        Icon::Command => {
            let sq = r.shrink(3.0);
            p.rect_stroke(sq, egui::Rounding::same(1.0), s);
            line((c.x, sq.top()), (c.x, sq.bottom()));
            line((sq.left(), c.y), (sq.right(), c.y));
            for (x, y) in [
                (sq.left(), sq.top()),
                (sq.right(), sq.top()),
                (sq.left(), sq.bottom()),
                (sq.right(), sq.bottom()),
            ] {
                p.circle_stroke(egui::pos2(x, y), 2.0, s);
            }
        }
        Icon::Search => {
            let cc = egui::pos2(c.x - 1.8, c.y - 1.8);
            p.circle_stroke(cc, r.width() * 0.34, s);
            line(
                (cc.x + 3.6, cc.y + 3.6),
                (r.right() - 0.2, r.bottom() - 0.2),
            );
        }
        Icon::Refresh => {
            let rad = r.width() * 0.42;
            let mut pts = Vec::new();
            let start = -0.6_f32;
            let end = start + std::f32::consts::TAU * 0.82;
            for k in 0..=20 {
                let a = start + (end - start) * k as f32 / 20.0;
                pts.push(egui::pos2(c.x + rad * a.cos(), c.y + rad * a.sin()));
            }
            p.add(egui::Shape::line(pts, s));
            let tip = egui::pos2(c.x + rad * end.cos(), c.y + rad * end.sin());
            tri([
                (tip.x + 1.0, tip.y - 4.0),
                (tip.x + 1.0, tip.y + 4.0),
                (tip.x + 5.5, tip.y),
            ]);
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
