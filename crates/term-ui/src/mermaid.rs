//! A small, self-contained Mermaid renderer for the common case: `graph` /
//! `flowchart` with `TD|TB|BT|LR|RL`, `id[Label]` / `id(Label)` / `id{Label}`,
//! and `-->` / `---` / `-.->` / `==>` edges (optionally labelled).
//!
//! This is deliberately a **subset**: anything else makes [`parse`] return
//! `None`, and the caller falls back to an external renderer or a placeholder.
//! Keeping it honest beats silently drawing the wrong thing (cf. ADR 0029).

use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    TopDown,
    BottomUp,
    LeftRight,
    RightLeft,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeKind {
    Box,
    Round,
    Diamond,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
    pub dotted: bool,
}

#[derive(Clone, Debug)]
pub struct Graph {
    pub dir: Dir,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// Strip bracket wrappers (`[..]`, `(..)`, `{..}`, `([..])`, `((..))`, `{{..}}`).
fn split_node(text: &str) -> (String, Option<String>, NodeKind) {
    let t = text.trim();
    let id_end = t
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(t.len());
    let id = t[..id_end].to_string();
    let rest = t[id_end..].trim();
    if rest.is_empty() {
        return (id, None, NodeKind::Box);
    }
    let first = rest.chars().next().unwrap();
    let kind = match first {
        '(' => NodeKind::Round,
        '{' => NodeKind::Diamond,
        _ => NodeKind::Box,
    };
    let closing = match first {
        '(' => ')',
        '{' => '}',
        '[' => ']',
        _ => return (id, None, NodeKind::Box),
    };
    let inner = rest
        .trim_start_matches(['[', '(', '{'])
        .trim_end_matches([']', ')', '}'])
        .trim();
    // Drop a matching pair of the same bracket for `((x))` / `{{x}}`.
    let inner = inner
        .strip_prefix(closing_to_open(closing))
        .and_then(|s| s.strip_suffix(closing))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(inner);
    let label = inner.trim_matches('"').to_string();
    (id, if label.is_empty() { None } else { Some(label) }, kind)
}

fn closing_to_open(c: char) -> char {
    match c {
        ')' => '(',
        '}' => '{',
        _ => '[',
    }
}

/// Find the first edge operator at bracket depth 0. Returns `(start, len, dotted)`.
fn find_operator(s: &str) -> Option<(usize, usize, bool)> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'[' | b'(' | b'{' => depth += 1,
            b']' | b')' | b'}' => depth -= 1,
            b'-' | b'=' if depth == 0 => {
                let rest = &s[i..];
                let dotted = rest.starts_with("-.");
                let (len, _) = if rest.starts_with("-.->") {
                    (4, true)
                } else if rest.starts_with("--x")
                    || rest.starts_with("--o")
                    || rest.starts_with("-->")
                    || rest.starts_with("==>")
                {
                    (3, false)
                } else if rest.starts_with("-.-") {
                    (3, true)
                } else if rest.starts_with("--") || rest.starts_with("==") {
                    (2, false)
                } else {
                    i += 1;
                    continue;
                };
                return Some((i, len, dotted));
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split one statement into an alternating node / operator chain.
fn parse_statement(
    stmt: &str,
    g: &mut Graph,
    index: &mut std::collections::HashMap<String, usize>,
) {
    let mut add = |text: &str, g: &mut Graph| -> Option<usize> {
        let (id, label, kind) = split_node(text);
        if id.is_empty() {
            return None;
        }
        Some(match index.get(&id) {
            Some(&i) => {
                if let Some(l) = label {
                    g.nodes[i].label = l;
                    g.nodes[i].kind = kind;
                }
                i
            }
            None => {
                let i = g.nodes.len();
                g.nodes.push(Node {
                    id: id.clone(),
                    label: label.unwrap_or_else(|| id.clone()),
                    kind,
                });
                index.insert(id, i);
                i
            }
        })
    };

    let mut rest = stmt.trim();
    let Some((op_start, op_len, dotted)) = find_operator(rest) else {
        let _ = add(rest, g);
        return;
    };
    let mut prev = match add(rest[..op_start].trim(), g) {
        Some(i) => i,
        None => return,
    };
    rest = &rest[op_start + op_len..];
    // Each edge carries the operator that *precedes* its target, plus an
    // optional label written as `-->|text|` or `-- text -->`.
    let mut cur_dotted = dotted;
    let mut cur_len = op_len;
    let mut label: Option<String> = None;
    loop {
        if label.is_none() {
            let trimmed = rest.trim_start();
            if cur_len == 2 && !trimmed.starts_with('|') {
                if let Some((text, tail)) = trimmed.split_once("--") {
                    if !text.trim().is_empty() {
                        label = Some(text.trim().to_string());
                    }
                    rest = tail.strip_prefix('>').unwrap_or(tail);
                }
            }
            let trimmed = rest.trim_start();
            if label.is_none() {
                if let Some(after) = trimmed.strip_prefix('|') {
                    if let Some((text, tail)) = after.split_once('|') {
                        label = Some(text.trim().to_string());
                        rest = tail;
                    }
                }
            }
        }
        // Next node ref: up to the next operator (or end).
        let next_op = find_operator(rest);
        let (node_text, tail, next_dotted, next_len) = match next_op {
            Some((s, l, d)) => (&rest[..s], &rest[s + l..], d, l),
            None => (rest, "", false, 0),
        };
        let Some(to) = add(node_text.trim(), g) else {
            break;
        };
        g.edges.push(Edge {
            from: prev,
            to,
            label: label.take(),
            dotted: cur_dotted,
        });
        prev = to;
        rest = tail;
        cur_dotted = next_dotted;
        cur_len = next_len;
        if rest.trim().is_empty() {
            break;
        }
    }
}

/// Parse a Mermaid `graph`/`flowchart` source. `None` if it is not one we can
/// draw (unsupported diagram type, or no nodes).
pub fn parse(src: &str) -> Option<Graph> {
    let mut dir = None;
    let mut g = Graph {
        dir: Dir::TopDown,
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut index = std::collections::HashMap::new();
    for raw in src.lines() {
        let no_comment = match raw.split_once("%%") {
            Some((head, _)) => head,
            None => raw,
        };
        for stmt in no_comment.split(';') {
            let t = stmt.trim();
            if t.is_empty() {
                continue;
            }
            let lower = t.to_ascii_lowercase();
            if lower == "subgraph" || lower == "end" {
                continue;
            }
            if let Some(rest) = lower.strip_prefix("direction ") {
                if let Some(d) = parse_dir(rest) {
                    g.dir = d;
                }
                continue;
            }
            if let Some(rest) = t.split_whitespace().collect::<Vec<_>>().as_slice().first() {
                let head = rest.to_ascii_lowercase();
                if head == "style"
                    || head == "classdef"
                    || head == "class"
                    || head == "linkstyle"
                    || head == "click"
                {
                    continue;
                }
            }
            if dir.is_none() {
                // First meaningful line must declare the direction.
                let mut it = t.split_whitespace();
                let kind = it.next().unwrap_or("").to_ascii_lowercase();
                if kind == "graph" || kind == "flowchart" {
                    let d = parse_dir(it.next().unwrap_or("TD")).unwrap_or(Dir::TopDown);
                    g.dir = d;
                    dir = Some(d);
                    continue;
                }
                return None;
            }
            parse_statement(t, &mut g, &mut index);
        }
    }
    dir?;
    if g.nodes.is_empty() {
        return None;
    }
    Some(g)
}

fn parse_dir(s: &str) -> Option<Dir> {
    Some(match s.trim().to_ascii_uppercase().as_str() {
        "TD" | "TB" => Dir::TopDown,
        "BT" => Dir::BottomUp,
        "LR" => Dir::LeftRight,
        "RL" => Dir::RightLeft,
        _ => return None,
    })
}

/// Longest-path layer per node (cycles broken by iteration order).
fn assign_layers(g: &Graph) -> Vec<usize> {
    let n = g.nodes.len();
    let mut layer = vec![0usize; n];
    for _ in 0..n {
        let mut changed = false;
        for e in &g.edges {
            if e.from != e.to && layer[e.to] < layer[e.from] + 1 {
                layer[e.to] = layer[e.from] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    layer
}

/// Draw the graph, allocating the canvas it needs. Returns the allocated
/// response's rect size.
pub fn show(ui: &mut egui::Ui, g: &Graph, fg: egui::Color32, panel: egui::Color32) {
    let font = egui::FontId::proportional(13.0);
    let pad = egui::vec2(14.0, 8.0);
    let gap = egui::vec2(36.0, 44.0);
    let galleys: Vec<Arc<egui::Galley>> = g
        .nodes
        .iter()
        .map(|n| {
            ui.painter()
                .layout_no_wrap(n.label.clone(), font.clone(), fg)
        })
        .collect();
    let sizes: Vec<egui::Vec2> = galleys.iter().map(|gl| gl.size() + pad * 2.0).collect();

    let layers = assign_layers(g);
    let max_layer = layers.iter().copied().max().unwrap_or(0);
    let mut by_layer: Vec<Vec<usize>> = vec![Vec::new(); max_layer + 1];
    for (i, &l) in layers.iter().enumerate() {
        by_layer[l].push(i);
    }

    let horizontal = matches!(g.dir, Dir::LeftRight | Dir::RightLeft);
    // "span" is the extent along the layer axis (width for TD, height for LR).
    let span: Vec<f32> = by_layer
        .iter()
        .map(|members| {
            members
                .iter()
                .map(|&i| if horizontal { sizes[i].y } else { sizes[i].x })
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap_or(0.0)
        })
        .collect();

    let mut centers = vec![egui::pos2(0.0, 0.0); g.nodes.len()];
    let axis_gap = if horizontal { gap.x } else { gap.y };
    let mut offset = 0.0f32;
    for (l, members) in by_layer.iter().enumerate() {
        let cross_gap = if horizontal { gap.y } else { gap.x };
        let total: f32 = members
            .iter()
            .map(|&i| if horizontal { sizes[i].y } else { sizes[i].x })
            .sum::<f32>()
            + cross_gap * (members.len().saturating_sub(1)) as f32;
        let mut cross = -total / 2.0;
        for &i in members {
            let (w, h) = (sizes[i].x, sizes[i].y);
            let along = offset + span[l] / 2.0;
            let c = if horizontal {
                egui::pos2(along, cross + h / 2.0)
            } else {
                egui::pos2(cross + w / 2.0, along)
            };
            centers[i] = c;
            cross += (if horizontal { h } else { w }) + cross_gap;
        }
        offset += span[l] + axis_gap;
    }

    // Mirror for the reversed directions.
    if matches!(g.dir, Dir::RightLeft) {
        for c in &mut centers {
            c.x = -c.x;
        }
    }
    if matches!(g.dir, Dir::BottomUp) {
        for c in &mut centers {
            c.y = -c.y;
        }
    }

    // Normalize to a positive bounding box with a margin.
    let margin = 10.0f32;
    let mut min = egui::pos2(f32::MAX, f32::MAX);
    let mut max = egui::pos2(f32::MIN, f32::MIN);
    for (i, c) in centers.iter().enumerate() {
        min.x = min.x.min(c.x - sizes[i].x / 2.0);
        min.y = min.y.min(c.y - sizes[i].y / 2.0);
        max.x = max.x.max(c.x + sizes[i].x / 2.0);
        max.y = max.y.max(c.y + sizes[i].y / 2.0);
    }
    let origin = egui::vec2(margin - min.x, margin - min.y);
    let size = egui::vec2(max.x - min.x + margin * 2.0, max.y - min.y + margin * 2.0);
    let size = egui::vec2(size.x.min(ui.available_width().max(200.0)), size.y);
    let (resp, painter) = ui.allocate_painter(size, egui::Sense::hover());
    let base = resp.rect.min.to_vec2() + origin;
    for c in &mut centers {
        *c += base;
    }

    // Edges first, then nodes on top.
    let stroke = egui::Stroke::new(1.4_f32, fg.gamma_multiply(0.8));
    for e in &g.edges {
        if e.from == e.to {
            continue;
        }
        let a = edge_point(centers[e.from], sizes[e.from], centers[e.to]);
        let b = edge_point(centers[e.to], sizes[e.to], centers[e.from]);
        if e.dotted {
            painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 4.0, 3.0));
        } else {
            painter.line_segment([a, b], stroke);
        }
        // Arrowhead at `b`.
        let dir = (b - a).normalized();
        let n = egui::vec2(-dir.y, dir.x);
        let tip = b;
        let back = b - dir * 9.0;
        painter.add(egui::Shape::convex_polygon(
            vec![tip, back + n * 4.0, back - n * 4.0],
            fg.gamma_multiply(0.8),
            egui::Stroke::NONE,
        ));
        if let Some(label) = &e.label {
            let mid = a + (b - a) * 0.5;
            let gl = painter.layout_no_wrap(label.clone(), egui::FontId::proportional(11.0), fg);
            let r = egui::Rect::from_center_size(mid, gl.size() + egui::vec2(6.0, 2.0));
            painter.rect_filled(r, egui::Rounding::same(3.0), panel);
            painter.galley(r.min + egui::vec2(3.0, 1.0), gl, fg);
        }
    }

    for (i, c) in centers.iter().enumerate() {
        let rect = egui::Rect::from_center_size(*c, sizes[i]);
        let round = match g.nodes[i].kind {
            NodeKind::Round => egui::Rounding::same(sizes[i].y / 2.0),
            _ => egui::Rounding::same(4.0),
        };
        painter.rect_filled(rect, round, panel);
        let border = fg.gamma_multiply(0.5);
        for (a, b) in [
            (rect.left_top(), rect.right_top()),
            (rect.right_top(), rect.right_bottom()),
            (rect.right_bottom(), rect.left_bottom()),
            (rect.left_bottom(), rect.left_top()),
        ] {
            painter.line_segment([a, b], egui::Stroke::new(1.2_f32, border));
        }
        let gl = &galleys[i];
        let pos = *c - gl.size() / 2.0;
        painter.galley(pos, gl.clone(), fg);
    }
}

/// Point on the border of a box (centered at `c`, size `s`) in the direction of
/// `toward`.
fn edge_point(c: egui::Pos2, s: egui::Vec2, toward: egui::Pos2) -> egui::Pos2 {
    let d = toward - c;
    if d.x == 0.0 && d.y == 0.0 {
        return c;
    }
    let tx = if d.x != 0.0 {
        (s.x / 2.0) / d.x.abs()
    } else {
        f32::MAX
    };
    let ty = if d.y != 0.0 {
        (s.y / 2.0) / d.y.abs()
    } else {
        f32::MAX
    };
    c + d * tx.min(ty)
}

/// Whether `cmd` can be found on `PATH` (or as an absolute path).
pub fn on_path(cmd: &str) -> bool {
    if cmd.contains('/') || cmd.contains('\\') {
        return std::path::Path::new(cmd).is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let exts: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    for dir in std::env::split_paths(&path) {
        for ext in exts {
            if dir.join(format!("{cmd}{ext}")).is_file() {
                return true;
            }
        }
    }
    false
}

fn hash_str(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Render `source` to a PNG with an external Mermaid CLI (e.g. `mmdc`), caching
/// the result in `out_dir`. Returns the image path, or `None` if the tool is
/// missing or failed. Opt-in: callers should only pass an explicit `cmd`.
pub fn render_external(
    source: &str,
    cmd: &str,
    out_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    if !on_path(cmd) {
        return None;
    }
    std::fs::create_dir_all(out_dir).ok()?;
    let stem = format!("{:016x}", hash_str(source));
    let mmd = out_dir.join(format!("{stem}.mmd"));
    let png = out_dir.join(format!("{stem}.png"));
    if png.is_file() {
        return Some(png);
    }
    std::fs::write(&mmd, source).ok()?;
    let status = std::process::Command::new(cmd)
        .arg("-i")
        .arg(&mmd)
        .arg("-o")
        .arg(&png)
        .args(["-b", "transparent"])
        .status()
        .ok()?;
    if status.success() && png.is_file() {
        Some(png)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flowchart_with_edges_and_labels() {
        let src =
            "flowchart TD\n  A[Start] --> B{OK?}\n  B -->|yes| C(Run)\n  B -->|no| D; D ==> A";
        let g = parse(src).expect("should parse");
        assert_eq!(g.dir, Dir::TopDown);
        assert_eq!(g.nodes.len(), 4);
        assert_eq!(g.nodes[0].label, "Start");
        assert_eq!(g.nodes[1].kind, NodeKind::Diamond);
        assert_eq!(g.nodes[2].kind, NodeKind::Round);
        assert_eq!(g.edges.len(), 4);
        assert_eq!(g.edges[1].label.as_deref(), Some("yes"));
        assert!(!g.edges[3].dotted);
    }

    #[test]
    fn parses_direction_and_chains() {
        let g = parse("graph LR\n A --> B --> C").unwrap();
        assert_eq!(g.dir, Dir::LeftRight);
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.edges.len(), 2);
        assert_eq!((g.edges[0].from, g.edges[0].to), (0, 1));
        assert_eq!((g.edges[1].from, g.edges[1].to), (1, 2));
    }

    #[test]
    fn rejects_unsupported() {
        assert!(parse("sequenceDiagram\n A->>B: hi").is_none());
        assert!(parse("graph TD\n").is_none());
        assert!(parse("just some text").is_none());
    }

    #[test]
    fn layers_follow_edges() {
        let g = parse("graph TD\n A --> B\n A --> C\n B --> D\n C --> D").unwrap();
        let l = assign_layers(&g);
        assert_eq!(l[0], 0); // A
        assert_eq!(l[3], 2); // D
        assert_eq!(l[1], l[2]); // B and C same layer
    }

    #[test]
    fn show_emits_shapes_headless() {
        let g = parse("flowchart TD\n A[One] --> B(Two)\n B -->|x| C{Three}").unwrap();
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &g, egui::Color32::WHITE, egui::Color32::BLACK);
            });
        });
        assert!(!out.shapes.is_empty(), "mermaid::show produced no shapes");
    }

    #[test]
    fn external_tool_detection() {
        assert!(!on_path("definitely-not-a-real-tool-xyz"));
        assert!(on_path("/bin/sh") || cfg!(windows));
    }

    #[test]
    fn dotted_and_loops() {
        let g = parse("graph TD\n A -.-> B\n B --> A").unwrap();
        assert!(g.edges[0].dotted);
        assert!(!g.edges[1].dotted);
    }
}
