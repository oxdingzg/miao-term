//! Small hand-drawn outline icons (no font glyphs, so they always render).

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
    let s = egui::Stroke::new(1.4_f32, color);
    let r = rect.shrink(1.5);
    let c = r.center();
    let round = egui::Rounding::same(2.5);
    let line = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([egui::pos2(a.0, a.1), egui::pos2(b.0, b.1)], s);
    };
    match icon {
        Icon::Terminal => {
            p.rect_stroke(r, round, s);
            let (l, t) = (r.left() + 3.5, r.center().y - 3.5);
            line((l, t), (l + 3.0, t + 3.0));
            line((l + 3.0, t + 3.0), (l, t + 6.0));
            line((l + 6.0, t + 6.0), (l + 9.0, t + 6.0));
        }
        Icon::Sidebar => {
            p.rect_stroke(r, round, s);
            let x = r.left() + 4.0;
            line((x, r.top()), (x, r.bottom()));
        }
        Icon::Details => {
            p.rect_stroke(r, round, s);
            let x = r.right() - 4.0;
            line((x, r.top()), (x, r.bottom()));
        }
        Icon::Folder => {
            let fr = egui::Rect::from_min_max(
                egui::pos2(r.left() + 0.5, r.top() + 3.5),
                egui::pos2(r.right() - 0.5, r.bottom() - 0.5),
            );
            p.rect_stroke(fr, egui::Rounding::same(2.0), s);
            line((fr.left(), fr.top()), (fr.left() + 3.5, fr.top()));
            line(
                (fr.left() + 3.5, fr.top()),
                (fr.left() + 5.0, r.top() + 0.5),
            );
            line(
                (fr.left() + 5.0, r.top() + 0.5),
                (fr.right() - 4.0, r.top() + 0.5),
            );
        }
        Icon::Server => {
            for k in 0..2 {
                let y = r.top() + 2.5 + k as f32 * 5.0;
                p.rect_stroke(
                    egui::Rect::from_min_max(
                        egui::pos2(r.left() + 0.5, y),
                        egui::pos2(r.right() - 0.5, y + 3.5),
                    ),
                    egui::Rounding::same(1.2),
                    s,
                );
                p.circle_filled(egui::pos2(r.right() - 3.5, y + 1.75), 0.9, color);
            }
        }
        Icon::GitBranch | Icon::Git => {
            p.circle_stroke(egui::pos2(c.x - 3.2, r.top() + 3.2), 1.8, s);
            p.circle_stroke(egui::pos2(c.x - 3.2, r.bottom() - 3.2), 1.8, s);
            p.circle_stroke(egui::pos2(c.x + 3.2, r.top() + 3.2), 1.8, s);
            line((c.x - 3.2, r.top() + 5.0), (c.x - 3.2, r.bottom() - 5.0));
            line((c.x - 3.2, c.y - 4.0), (c.x + 3.2, r.top() + 3.2));
        }
        Icon::File => {
            let (l, t, rr, b) = (
                r.left() + 2.5,
                r.top() + 1.0,
                r.right() - 2.5,
                r.bottom() - 1.0,
            );
            let stroke = egui::Stroke::new(1.4_f32, color);
            p.add(egui::Shape::closed_line(
                vec![
                    egui::pos2(l, t),
                    egui::pos2(rr - 3.5, t),
                    egui::pos2(rr, t + 3.5),
                    egui::pos2(rr, b),
                    egui::pos2(l, b),
                ],
                stroke,
            ));
            line((rr - 3.5, t), (rr - 3.5, t + 3.5));
            line((rr - 3.5, t + 3.5), (rr, t + 3.5));
        }
        Icon::Files => {
            let fr = egui::Rect::from_min_max(
                egui::pos2(r.left() + 0.5, r.top() + 3.5),
                egui::pos2(r.right() - 0.5, r.bottom() - 0.5),
            );
            p.rect_stroke(fr, egui::Rounding::same(2.0), s);
            line((fr.left(), fr.top()), (fr.left() + 3.5, fr.top()));
            line(
                (fr.left() + 3.5, fr.top()),
                (fr.left() + 5.0, r.top() + 0.5),
            );
            line(
                (fr.left() + 5.0, r.top() + 0.5),
                (fr.right() - 4.0, r.top() + 0.5),
            );
        }
        Icon::Plus => {
            line((c.x, r.top() + 2.0), (c.x, r.bottom() - 2.0));
            line((r.left() + 2.0, c.y), (r.right() - 2.0, c.y));
        }
        Icon::Info => {
            p.circle_stroke(c, r.width() * 0.46, s);
            p.circle_filled(egui::pos2(c.x, c.y - 3.2), 1.0, color);
            line((c.x, c.y - 1.0), (c.x, c.y + 3.4));
        }
        Icon::Agent => {
            // Robot head: rounded body + antenna + two eyes.
            let body = egui::Rect::from_min_max(
                egui::pos2(r.left() + 1.5, r.top() + 4.0),
                egui::pos2(r.right() - 1.5, r.bottom() - 1.5),
            );
            p.rect_stroke(body, egui::Rounding::same(2.5), s);
            line((c.x, r.top() + 1.0), (c.x, body.top()));
            p.circle_filled(egui::pos2(c.x, r.top() + 0.8), 0.9, color);
            p.circle_filled(egui::pos2(c.x - 2.2, body.center().y), 1.0, color);
            p.circle_filled(egui::pos2(c.x + 2.2, body.center().y), 1.0, color);
        }
        Icon::Outline => {
            for k in 0..3 {
                let y = r.top() + 3.0 + k as f32 * 3.8;
                p.circle_filled(egui::pos2(r.left() + 2.0, y), 0.9, color);
                line((r.left() + 5.0, y), (r.right() - 1.0, y));
            }
        }
        Icon::Ports => {
            // Plug: body + two prongs + cable.
            let body = egui::Rect::from_min_max(
                egui::pos2(c.x - 4.0, r.top() + 1.5),
                egui::pos2(c.x + 4.0, c.y - 0.5),
            );
            p.rect_stroke(body, egui::Rounding::same(1.5), s);
            line((c.x - 2.0, body.top()), (c.x - 2.0, r.top() - 0.5));
            line((c.x + 2.0, body.top()), (c.x + 2.0, r.top() - 0.5));
            line((c.x, body.bottom()), (c.x, r.bottom() - 0.5));
        }
        Icon::Queue => {
            for k in 0..3 {
                let y = r.top() + 3.0 + k as f32 * 3.8;
                p.circle_filled(egui::pos2(r.left() + 2.0, y), 0.9, color);
                line((r.left() + 5.0, y), (r.right() - 1.0, y));
            }
        }
        Icon::Command => {
            // The four-loop ⌘: a rounded square with a centre cross and corner rings.
            let sq = r.shrink(2.5);
            p.rect_stroke(sq, egui::Rounding::same(1.0), s);
            line((c.x, sq.top()), (c.x, sq.bottom()));
            line((sq.left(), c.y), (sq.right(), c.y));
            for (x, y) in [
                (sq.left(), sq.top()),
                (sq.right(), sq.top()),
                (sq.left(), sq.bottom()),
                (sq.right(), sq.bottom()),
            ] {
                p.circle_stroke(egui::pos2(x, y), 1.6, s);
            }
        }
        Icon::Search => {
            let cc = egui::pos2(c.x - 1.5, c.y - 1.5);
            p.circle_stroke(cc, r.width() * 0.33, s);
            line(
                (cc.x + 3.2, cc.y + 3.2),
                (r.right() - 0.5, r.bottom() - 0.5),
            );
        }
        Icon::Refresh => {
            // A ~300° arc with an arrowhead, drawn as a polyline.
            let rad = r.width() * 0.42;
            let mut pts = Vec::new();
            let start = -0.6_f32;
            let end = start + std::f32::consts::TAU * 0.82;
            for k in 0..=18 {
                let a = start + (end - start) * k as f32 / 18.0;
                pts.push(egui::pos2(c.x + rad * a.cos(), c.y + rad * a.sin()));
            }
            p.add(egui::Shape::line(pts, s));
            let tip = egui::pos2(c.x + rad * end.cos(), c.y + rad * end.sin());
            let n = 3.5;
            p.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(tip.x + 1.5, tip.y - n),
                    egui::pos2(tip.x + 1.5, tip.y + n),
                    egui::pos2(tip.x + 5.0, tip.y),
                ],
                color,
                egui::Stroke::NONE,
            ));
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
