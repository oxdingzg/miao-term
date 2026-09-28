//! A small self-authored, lucide/feather-style stroke icon set.
//!
//! Icons are defined as primitives on a 24×24 grid and drawn directly with the
//! egui painter, so there is no SVG rasterizer dependency (which would pull in
//! MPL-licensed crates, excluded by ADR 0006). Icons are tinted per use.

use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use miao_term_config::view::Icon as ViewIcon;

/// A drawing primitive on the 24×24 icon grid (y grows downward).
pub enum Prim {
    /// A straight segment.
    Line((f32, f32), (f32, f32)),
    /// A connected polyline, optionally implicitly closed by repeating the
    /// first point.
    Poly(&'static [(f32, f32)]),
    /// A circle (center, radius).
    Circle((f32, f32), f32),
    /// An axis-aligned ellipse (center, rx, ry).
    Ellipse((f32, f32), f32, f32),
    /// An arc (center, radius, start radians, end radians); 0 rad points right,
    /// angles grow clockwise on screen because y grows downward.
    Arc((f32, f32), f32, f32, f32),
}

const TAU: f32 = std::f32::consts::TAU;
const PI: f32 = std::f32::consts::PI;

/// The icon table. Names are kebab-case and stable (persisted in views.json).
pub static ICONS: &[(&str, &[Prim])] = &[
    (
        "folder",
        &[Prim::Poly(&[
            (3.0, 5.0),
            (10.0, 5.0),
            (12.0, 7.0),
            (21.0, 7.0),
            (21.0, 19.0),
            (3.0, 19.0),
            (3.0, 5.0),
        ])],
    ),
    (
        "file",
        &[
            Prim::Poly(&[
                (13.0, 3.0),
                (6.0, 3.0),
                (6.0, 21.0),
                (18.0, 21.0),
                (18.0, 8.0),
                (13.0, 3.0),
            ]),
            Prim::Poly(&[(13.0, 3.0), (13.0, 8.0), (18.0, 8.0)]),
        ],
    ),
    (
        "file-text",
        &[
            Prim::Poly(&[
                (13.0, 3.0),
                (6.0, 3.0),
                (6.0, 21.0),
                (18.0, 21.0),
                (18.0, 8.0),
                (13.0, 3.0),
            ]),
            Prim::Poly(&[(13.0, 3.0), (13.0, 8.0), (18.0, 8.0)]),
            Prim::Line((9.0, 13.0), (15.0, 13.0)),
            Prim::Line((9.0, 17.0), (13.0, 17.0)),
        ],
    ),
    (
        "terminal",
        &[
            Prim::Poly(&[(4.0, 7.0), (9.0, 12.0), (4.0, 17.0)]),
            Prim::Line((12.0, 17.0), (20.0, 17.0)),
        ],
    ),
    (
        "code",
        &[
            Prim::Poly(&[(8.0, 8.0), (4.0, 12.0), (8.0, 16.0)]),
            Prim::Poly(&[(16.0, 8.0), (20.0, 12.0), (16.0, 16.0)]),
        ],
    ),
    (
        "git-branch",
        &[
            Prim::Circle((6.0, 6.0), 2.5),
            Prim::Circle((6.0, 18.0), 2.5),
            Prim::Line((6.0, 8.5), (6.0, 15.5)),
            Prim::Circle((18.0, 6.0), 2.5),
            Prim::Line((18.0, 8.5), (18.0, 12.0)),
            Prim::Poly(&[(18.0, 12.0), (10.0, 15.0), (6.0, 15.5)]),
        ],
    ),
    (
        "git-commit",
        &[
            Prim::Circle((12.0, 12.0), 4.0),
            Prim::Line((2.0, 12.0), (8.0, 12.0)),
            Prim::Line((16.0, 12.0), (22.0, 12.0)),
        ],
    ),
    (
        "github",
        &[
            Prim::Circle((12.0, 12.0), 9.0),
            Prim::Poly(&[(9.0, 16.0), (9.0, 19.5), (6.0, 19.5)]),
            Prim::Poly(&[(15.0, 16.0), (15.0, 19.5), (18.0, 19.5)]),
        ],
    ),
    (
        "globe",
        &[
            Prim::Circle((12.0, 12.0), 9.0),
            Prim::Line((3.0, 12.0), (21.0, 12.0)),
            Prim::Ellipse((12.0, 12.0), 4.5, 9.0),
        ],
    ),
    (
        "bug",
        &[
            Prim::Ellipse((12.0, 13.0), 5.0, 6.5),
            Prim::Line((12.0, 6.5), (12.0, 19.5)),
            Prim::Line((4.0, 9.0), (7.5, 10.5)),
            Prim::Line((20.0, 9.0), (16.5, 10.5)),
            Prim::Line((4.0, 17.0), (7.5, 15.5)),
            Prim::Line((20.0, 17.0), (16.5, 15.5)),
            Prim::Poly(&[(9.0, 5.0), (12.0, 8.0), (15.0, 5.0)]),
        ],
    ),
    (
        "flame",
        &[
            Prim::Poly(&[(12.0, 3.0), (15.0, 7.0), (17.0, 11.0)]),
            Prim::Poly(&[(12.0, 3.0), (9.0, 7.0), (7.0, 11.0)]),
            Prim::Arc((12.0, 14.0), 5.0, 0.0, PI),
        ],
    ),
    (
        "cpu",
        &[
            Prim::Poly(&[
                (6.0, 6.0),
                (18.0, 6.0),
                (18.0, 18.0),
                (6.0, 18.0),
                (6.0, 6.0),
            ]),
            Prim::Poly(&[
                (10.0, 10.0),
                (14.0, 10.0),
                (14.0, 14.0),
                (10.0, 14.0),
                (10.0, 10.0),
            ]),
            Prim::Line((9.0, 2.0), (9.0, 6.0)),
            Prim::Line((15.0, 2.0), (15.0, 6.0)),
            Prim::Line((9.0, 18.0), (9.0, 22.0)),
            Prim::Line((15.0, 18.0), (15.0, 22.0)),
            Prim::Line((2.0, 9.0), (6.0, 9.0)),
            Prim::Line((2.0, 15.0), (6.0, 15.0)),
            Prim::Line((18.0, 9.0), (22.0, 9.0)),
            Prim::Line((18.0, 15.0), (22.0, 15.0)),
        ],
    ),
    (
        "cloud",
        &[
            Prim::Arc((9.0, 14.0), 3.5, PI, TAU),
            Prim::Arc((15.0, 13.0), 4.5, PI, TAU),
            Prim::Arc((12.0, 16.0), 6.0, PI, TAU),
            Prim::Line((6.0, 18.0), (18.0, 18.0)),
        ],
    ),
    (
        "database",
        &[
            Prim::Ellipse((12.0, 6.0), 7.0, 3.0),
            Prim::Ellipse((12.0, 12.0), 7.0, 3.0),
            Prim::Ellipse((12.0, 18.0), 7.0, 3.0),
        ],
    ),
    (
        "package",
        &[
            Prim::Poly(&[
                (12.0, 3.0),
                (21.0, 8.0),
                (21.0, 16.0),
                (12.0, 21.0),
                (3.0, 16.0),
                (3.0, 8.0),
                (12.0, 3.0),
            ]),
            Prim::Line((3.0, 8.0), (12.0, 13.0)),
            Prim::Line((21.0, 8.0), (12.0, 13.0)),
            Prim::Line((12.0, 13.0), (12.0, 21.0)),
        ],
    ),
    (
        "box",
        &[
            Prim::Poly(&[
                (3.0, 7.0),
                (21.0, 7.0),
                (21.0, 18.0),
                (3.0, 18.0),
                (3.0, 7.0),
            ]),
            Prim::Line((3.0, 11.0), (21.0, 11.0)),
        ],
    ),
    (
        "coffee",
        &[
            Prim::Poly(&[(4.0, 8.0), (4.0, 15.0), (16.0, 15.0), (16.0, 8.0)]),
            Prim::Arc((16.0, 11.0), 3.0, -PI / 2.0, PI / 2.0),
            Prim::Line((3.0, 19.0), (19.0, 19.0)),
            Prim::Line((8.0, 3.0), (8.0, 5.5)),
            Prim::Line((12.0, 3.0), (12.0, 5.5)),
        ],
    ),
    (
        "heart",
        &[Prim::Poly(&[
            (12.0, 20.0),
            (4.0, 13.0),
            (4.0, 8.0),
            (8.0, 6.0),
            (12.0, 9.0),
            (16.0, 6.0),
            (20.0, 8.0),
            (20.0, 13.0),
            (12.0, 20.0),
        ])],
    ),
    (
        "star",
        &[Prim::Poly(&[
            (12.0, 3.0),
            (14.5, 9.0),
            (21.0, 9.0),
            (16.0, 13.0),
            (17.5, 19.5),
            (12.0, 16.0),
            (6.5, 19.5),
            (8.0, 13.0),
            (3.0, 9.0),
            (9.5, 9.0),
            (12.0, 3.0),
        ])],
    ),
    (
        "flag",
        &[
            Prim::Line((5.0, 3.0), (5.0, 21.0)),
            Prim::Poly(&[
                (5.0, 4.0),
                (17.0, 4.0),
                (15.0, 8.0),
                (17.0, 12.0),
                (5.0, 12.0),
            ]),
        ],
    ),
    (
        "zap",
        &[Prim::Poly(&[
            (13.0, 2.0),
            (5.0, 14.0),
            (11.0, 14.0),
            (10.0, 22.0),
            (18.0, 10.0),
            (12.0, 10.0),
            (13.0, 2.0),
        ])],
    ),
    (
        "lock",
        &[
            Prim::Poly(&[
                (5.0, 11.0),
                (19.0, 11.0),
                (19.0, 21.0),
                (5.0, 21.0),
                (5.0, 11.0),
            ]),
            Prim::Arc((12.0, 9.0), 3.5, PI, TAU),
        ],
    ),
    (
        "search",
        &[
            Prim::Circle((11.0, 11.0), 7.0),
            Prim::Line((16.0, 16.0), (21.0, 21.0)),
        ],
    ),
    (
        "settings",
        &[
            Prim::Circle((12.0, 12.0), 3.0),
            Prim::Line((12.0, 3.0), (12.0, 5.5)),
            Prim::Line((12.0, 18.5), (12.0, 21.0)),
            Prim::Line((3.0, 12.0), (5.5, 12.0)),
            Prim::Line((18.5, 12.0), (21.0, 12.0)),
            Prim::Line((5.6, 5.6), (7.4, 7.4)),
            Prim::Line((16.6, 16.6), (18.4, 18.4)),
            Prim::Line((5.6, 18.4), (7.4, 16.6)),
            Prim::Line((16.6, 7.4), (18.4, 5.6)),
        ],
    ),
    (
        "user",
        &[
            Prim::Circle((12.0, 8.0), 4.0),
            Prim::Arc((12.0, 17.0), 6.0, PI, TAU),
        ],
    ),
    (
        "home",
        &[
            Prim::Poly(&[(3.0, 11.0), (12.0, 3.0), (21.0, 11.0)]),
            Prim::Poly(&[(5.0, 10.0), (5.0, 20.0), (19.0, 20.0), (19.0, 10.0)]),
        ],
    ),
    (
        "bell",
        &[
            Prim::Arc((12.0, 15.0), 6.0, PI, TAU),
            Prim::Line((6.0, 15.0), (6.0, 19.0)),
            Prim::Line((18.0, 15.0), (18.0, 19.0)),
            Prim::Line((4.0, 19.0), (20.0, 19.0)),
            Prim::Arc((12.0, 19.0), 2.0, 0.0, PI),
        ],
    ),
    (
        "layers",
        &[
            Prim::Poly(&[
                (12.0, 3.0),
                (21.0, 8.0),
                (12.0, 13.0),
                (3.0, 8.0),
                (12.0, 3.0),
            ]),
            Prim::Poly(&[(3.0, 13.0), (12.0, 18.0), (21.0, 13.0)]),
        ],
    ),
    (
        "claude",
        &[Prim::Poly(&[
            (12.0, 3.0),
            (12.0, 21.0),
            (4.0, 7.0),
            (20.0, 7.0),
            (4.0, 17.0),
            (20.0, 17.0),
            (4.0, 7.0),
        ])],
    ),
];

/// Look up an icon's primitives by name.
pub fn icon(name: &str) -> Option<&'static [Prim]> {
    ICONS.iter().find(|(n, _)| *n == name).map(|(_, p)| *p)
}

/// All icon names, in table order (used by the picker).
pub fn names() -> impl Iterator<Item = &'static str> {
    ICONS.iter().map(|(n, _)| *n)
}

/// Draw an icon into `rect`, tinted `color`. Returns false if the name is
/// unknown.
pub fn draw(painter: &Painter, rect: Rect, name: &str, color: Color32) -> bool {
    let Some(prims) = icon(name) else {
        return false;
    };
    let side = rect.width().min(rect.height());
    let scale = side / 24.0;
    let origin = rect.center() - Vec2::splat(side / 2.0);
    let at = |x: f32, y: f32| Pos2::new(origin.x + x * scale, origin.y + y * scale);
    let stroke = Stroke::new((side / 12.0).max(1.0), color);
    for prim in prims {
        match prim {
            Prim::Line(a, b) => {
                painter.line_segment([at(a.0, a.1), at(b.0, b.1)], stroke);
            }
            Prim::Poly(points) => {
                let pts: Vec<Pos2> = points.iter().map(|(x, y)| at(*x, *y)).collect();
                painter.add(Shape::line(pts, stroke));
            }
            Prim::Circle((cx, cy), r) => {
                painter.circle_stroke(at(*cx, *cy), r * scale, stroke);
            }
            Prim::Ellipse((cx, cy), rx, ry) => {
                let pts = sample(32, |t| {
                    let a = t * TAU;
                    at(cx + rx * a.cos(), cy + ry * a.sin())
                });
                painter.add(Shape::line(pts, stroke));
            }
            Prim::Arc((cx, cy), r, a0, a1) => {
                let pts = sample(24, |t| {
                    let a = a0 + (a1 - a0) * t;
                    at(cx + r * a.cos(), cy + r * a.sin())
                });
                painter.add(Shape::line(pts, stroke));
            }
        }
    }
    true
}

/// Draw a view-engine icon descriptor: a named stroke icon, else an emoji, else
/// a filled dot for a color-only icon.
pub fn draw_view(painter: &Painter, rect: Rect, icon: &ViewIcon, default: Color32) {
    let color = icon
        .rgb()
        .map(|c| Color32::from_rgb(c.0, c.1, c.2))
        .unwrap_or(default);
    if let Some(name) = &icon.name {
        if draw(painter, rect, name, color) {
            return;
        }
    }
    if let Some(emoji) = &icon.emoji {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            emoji,
            FontId::proportional(rect.height() * 0.85),
            color,
        );
    } else if icon.name.is_none() {
        painter.circle_filled(rect.center(), rect.width() * 0.35, color);
    }
}

fn sample(n: usize, f: impl Fn(f32) -> Pos2) -> Vec<Pos2> {
    (0..=n).map(|i| f(i as f32 / n as f32)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_known() {
        let mut seen = std::collections::HashSet::new();
        for name in names() {
            assert!(seen.insert(name), "duplicate icon name: {name}");
        }
        for expected in [
            "folder",
            "file",
            "terminal",
            "git-branch",
            "claude",
            "search",
        ] {
            assert!(icon(expected).is_some(), "missing icon: {expected}");
        }
        assert!(icon("nope").is_none());
    }

    #[test]
    fn geometry_stays_on_the_grid() {
        for (name, prims) in ICONS {
            for prim in *prims {
                let pts: Vec<(f32, f32)> = match prim {
                    Prim::Line(a, b) => vec![*a, *b],
                    Prim::Poly(p) => p.to_vec(),
                    Prim::Circle((cx, cy), r) => {
                        vec![(cx - r, cy - r), (cx + r, cy + r)]
                    }
                    Prim::Ellipse((cx, cy), rx, ry) => {
                        vec![(cx - rx, cy - ry), (cx + rx, cy + ry)]
                    }
                    Prim::Arc((cx, cy), r, _, _) => vec![(cx - r, cy - r), (cx + r, cy + r)],
                };
                for (x, y) in pts {
                    assert!(
                        (-1.0..=25.0).contains(&x) && (-1.0..=25.0).contains(&y),
                        "icon {name} point ({x},{y}) off grid"
                    );
                }
            }
        }
    }
}
