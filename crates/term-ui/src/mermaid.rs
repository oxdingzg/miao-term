//! A small, self-contained Mermaid renderer for the common cases:
//! `graph` / `flowchart` with `TD|TB|BT|LR|RL`, `id[Label]` / `id(Label)` /
//! `id{Label}`, and `-->` / `---` / `-.->` / `==>` edges (optionally labelled);
//! `sequenceDiagram` with participants, `->`/`->>`/`-->`/`-->>`/`-x`/`--x`
//! messages and `Note over|left of|right of`; `stateDiagram`/`stateDiagram-v2`
//! (with `[*]` pseudo-states); `classDiagram` and `erDiagram` boxes with member
//! lines; and `pie` charts with a title.
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
    /// Start/end marker of a state diagram (`[*]`).
    Circle,
    /// A class/entity box: title plus member lines in `Node::body`.
    Class,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    /// Member/attribute lines drawn under the title (class and ER diagrams).
    pub body: Vec<String>,
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
/// A parsed Mermaid diagram: the caller renders either kind the same way.
pub enum Diagram {
    Graph(Graph),
    Sequence(Sequence),
    Pie(Pie),
}

/// A `pie` chart: an optional title and labelled values.
#[derive(Clone, Debug)]
pub struct Pie {
    pub title: Option<String>,
    pub slices: Vec<(String, f64)>,
}

/// A `sequenceDiagram`: declared/implied participants and ordered steps.
#[derive(Clone, Debug)]
pub struct Sequence {
    pub actors: Vec<Actor>,
    pub steps: Vec<SeqStep>,
}

#[derive(Clone, Debug)]
pub struct Actor {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub enum SeqStep {
    Message {
        from: usize,
        to: usize,
        text: String,
        /// `--` arrows.
        dashed: bool,
        /// Open arrowhead (`->`) rather than a filled one (`->>`).
        open: bool,
    },
    Note {
        over: Vec<usize>,
        text: String,
        side: NoteSide,
    },
}

/// Where a note sits relative to its participants.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoteSide {
    Over,
    Left,
    Right,
}

/// Parse either a graph/flowchart or a sequence diagram.
pub fn parse_diagram(src: &str) -> Option<Diagram> {
    let header = src
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("%%"))?;
    if header == "sequenceDiagram" || header.starts_with("sequenceDiagram ") {
        parse_sequence(src).map(Diagram::Sequence)
    } else if header == "pie" || header.starts_with("pie ") || header.starts_with("pie\t") {
        parse_pie(src).map(Diagram::Pie)
    } else if header.starts_with("stateDiagram") {
        parse_state(src).map(Diagram::Graph)
    } else if header.starts_with("classDiagram") {
        parse_class(src).map(Diagram::Graph)
    } else if header.starts_with("erDiagram") {
        parse_er(src).map(Diagram::Graph)
    } else {
        parse(src).map(Diagram::Graph)
    }
}

/// Render a [`Diagram`].
pub fn show_diagram(ui: &mut egui::Ui, d: &Diagram, fg: egui::Color32, panel: egui::Color32) {
    match d {
        Diagram::Graph(g) => show(ui, g, fg, panel),
        Diagram::Sequence(s) => show_sequence(ui, s, fg, panel),
        Diagram::Pie(p) => show_pie(ui, p, fg, panel),
    }
}

/// The `stateDiagram`/`stateDiagram-v2` case: a graph of states, where `[*]`
/// marks the initial/final pseudo-state (drawn as a small circle).
fn parse_state(src: &str) -> Option<Graph> {
    use std::collections::HashMap;
    let mut g = Graph {
        dir: Dir::TopDown,
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut started = false;
    let intern = |id: &str, g: &mut Graph, index: &mut HashMap<String, usize>| -> usize {
        if let Some(&i) = index.get(id) {
            return i;
        }
        let i = g.nodes.len();
        let (label, kind) = if id == "[*]" {
            (String::new(), NodeKind::Circle)
        } else {
            (id.to_string(), NodeKind::Box)
        };
        g.nodes.push(Node {
            id: id.to_string(),
            label,
            kind,
            body: Vec::new(),
        });
        index.insert(id.to_string(), i);
        i
    };
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }
        if !started {
            if line == "stateDiagram" || line.starts_with("stateDiagram") {
                started = true;
            }
            continue;
        }
        let lower = line.to_ascii_lowercase();
        // Aliased states: `state "Long name" as S`.
        if lower.starts_with("state ") {
            let rest = line[6..].trim();
            let (label, id) = rest.split_once(" as ")?;
            let id = id.trim();
            if id.is_empty() {
                return None;
            }
            let i = intern(id, &mut g, &mut index);
            let label = label.trim().trim_matches('"').trim();
            g.nodes[i].label = if label.is_empty() {
                id.to_string()
            } else {
                label.to_string()
            };
            continue;
        }
        if lower.starts_with("direction ") || lower == "note" || lower.starts_with("note ") {
            // Notes and per-diagram direction are not modelled.
            return None;
        }
        let (left, label, right) = split_relation(line, &["-->"]);
        if right.trim().is_empty() {
            return None;
        }
        let a = intern(left.trim(), &mut g, &mut index);
        let b = intern(right.trim(), &mut g, &mut index);
        g.edges.push(Edge {
            from: a,
            to: b,
            label: label.map(str::to_string),
            dotted: false,
        });
    }
    if !started || g.edges.is_empty() || g.nodes.is_empty() {
        return None;
    }
    Some(g)
}

/// Split `A <rel> B : label` into (left, label, right) for the first of `ops`
/// that appears. Returns `("", None, "")` when none match.
fn split_relation<'a>(line: &'a str, ops: &[&str]) -> (&'a str, Option<&'a str>, &'a str) {
    let mut best: Option<(usize, &str)> = None;
    for op in ops {
        if let Some(p) = line.find(op) {
            if best.map(|(bp, _)| p < bp).unwrap_or(true) {
                best = Some((p, op));
            }
        }
    }
    let Some((pos, op)) = best else {
        return ("", None, "");
    };
    let (head, tail) = line.split_at(pos);
    let tail = &tail[op.len()..];
    let (mid, label) = match tail.split_once(':') {
        Some((m, l)) => (m, Some(l.trim())),
        None => (tail, None),
    };
    (head.trim(), label, mid.trim())
}

/// `classDiagram`: classes (optionally with a member block) and relations.
fn parse_class(src: &str) -> Option<Graph> {
    use std::collections::HashMap;
    let mut g = Graph {
        dir: Dir::TopDown,
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut index: HashMap<String, usize> = HashMap::new();
    let intern = |id: &str, g: &mut Graph, index: &mut HashMap<String, usize>| -> usize {
        if let Some(&i) = index.get(id) {
            return i;
        }
        let i = g.nodes.len();
        g.nodes.push(Node {
            id: id.to_string(),
            label: id.to_string(),
            kind: NodeKind::Class,
            body: Vec::new(),
        });
        index.insert(id.to_string(), i);
        i
    };
    let mut started = false;
    let mut current: Option<usize> = None; // open `class X { ... }` block
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }
        if !started {
            if line.starts_with("classDiagram") {
                started = true;
            }
            continue;
        }
        if line == "}" {
            current = None;
            continue;
        }
        if let Some(i) = current {
            g.nodes[i].body.push(line.to_string());
            continue;
        }
        let lower = line.to_ascii_lowercase();
        // `class Name`, `class Name {`, or `Name : member`.
        if let Some(rest) = lower.strip_prefix("class ") {
            let rest = line[line.len() - rest.len()..].trim();
            let (name, open) = match rest.strip_suffix('{') {
                Some(n) => (n.trim(), true),
                None => (rest, false),
            };
            let i = intern(name, &mut g, &mut index);
            if open {
                current = Some(i);
            }
            continue;
        }
        if let Some((name, member)) = line.split_once(':') {
            let name = name.trim();
            let member = member.trim();
            if name.contains('<') || name.contains('>') || name.contains('-') {
                return None;
            }
            let i = intern(name, &mut g, &mut index);
            if !member.is_empty() {
                g.nodes[i].body.push(member.to_string());
            }
            continue;
        }
        // Relations, longest operator first so `<|--` beats `--`.
        const CLASS_OPS: [&str; 14] = [
            "<|--", "--|>", "<|..", "..|>", "*--", "--*", "o--", "--o", "..>", "<..", "-->", "<--",
            "..", "--",
        ];
        let (left, label, right) = split_relation(line, &CLASS_OPS);
        if left.is_empty() || right.is_empty() || left == line {
            return None;
        }
        let op = &line[left.len()..line.len() - right.len()];
        let dotted = op.contains("..");
        let a = intern(left, &mut g, &mut index);
        let b = intern(right, &mut g, &mut index);
        g.edges.push(Edge {
            from: a,
            to: b,
            label: label.map(str::to_string),
            dotted,
        });
    }
    if !started || g.nodes.is_empty() {
        return None;
    }
    Some(g)
}

/// `erDiagram`: entities (with an attribute block) and cardinality relations.
fn parse_er(src: &str) -> Option<Graph> {
    use std::collections::HashMap;
    let mut g = Graph {
        dir: Dir::LeftRight,
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut index: HashMap<String, usize> = HashMap::new();
    let intern = |id: &str, g: &mut Graph, index: &mut HashMap<String, usize>| -> usize {
        if let Some(&i) = index.get(id) {
            return i;
        }
        let i = g.nodes.len();
        g.nodes.push(Node {
            id: id.to_string(),
            label: id.to_string(),
            kind: NodeKind::Class,
            body: Vec::new(),
        });
        index.insert(id.to_string(), i);
        i
    };
    let mut started = false;
    let mut current: Option<usize> = None;
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }
        if !started {
            if line.starts_with("erDiagram") {
                started = true;
            }
            continue;
        }
        if line == "}" {
            current = None;
            continue;
        }
        if let Some(i) = current {
            g.nodes[i].body.push(line.to_string());
            continue;
        }
        if let Some((head, open)) = line.strip_suffix('{').map(|h| (h.trim(), true)) {
            if open && !head.contains(' ') {
                let i = intern(head, &mut g, &mut index);
                current = Some(i);
                continue;
            }
        }
        // Relations: `A ||--o{ B : label`.
        if line.contains("--") {
            let (left, label, right) = split_relation(line, &["--"]);
            if left.is_empty() || right.is_empty() {
                return None;
            }
            let left = left.trim_end_matches(['|', 'o', '{', '}']);
            let right = right.trim_start_matches(['|', 'o', '{', '}']);
            let a = intern(left.trim(), &mut g, &mut index);
            let b = intern(right.trim(), &mut g, &mut index);
            g.edges.push(Edge {
                from: a,
                to: b,
                label: label.map(str::to_string),
                dotted: false,
            });
            continue;
        }
        return None;
    }
    if !started || g.nodes.is_empty() {
        return None;
    }
    Some(g)
}

/// Intern an actor id, appending it in first-seen order.
fn intern(
    actors: &mut Vec<Actor>,
    index: &mut std::collections::HashMap<String, usize>,
    name: &str,
) -> usize {
    if let Some(&i) = index.get(name) {
        return i;
    }
    actors.push(Actor {
        id: name.to_string(),
        label: name.to_string(),
    });
    index.insert(name.to_string(), actors.len() - 1);
    actors.len() - 1
}

/// The arrows we understand, longest first so `-->>` wins over `->`.
const SEQ_ARROWS: [&str; 6] = ["-->>", "->>", "-->", "->", "--x", "-x"];

fn split_arrow(line: &str) -> Option<(&str, &str, &'static str)> {
    let (pos, tok) = SEQ_ARROWS
        .iter()
        .filter_map(|t| line.find(t).map(|p| (p, *t)))
        .min_by_key(|(p, _)| *p)?;
    let from = &line[..pos];
    let rest = &line[pos + tok.len()..];
    let to = rest.split(':').next().unwrap_or(rest);
    Some((from.trim(), to.trim(), tok))
}

/// Keywords that change the layout we do not model; refusing keeps us honest.
const SEQ_UNSUPPORTED: [&str; 14] = [
    "loop",
    "alt",
    "else",
    "opt",
    "par",
    "and",
    "critical",
    "break",
    "rect",
    "end",
    "activate",
    "deactivate",
    "create",
    "destroy",
];

fn parse_sequence(src: &str) -> Option<Sequence> {
    use std::collections::HashMap;
    let mut actors: Vec<Actor> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut steps: Vec<SeqStep> = Vec::new();
    let mut started = false;
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }
        if !started {
            if line == "sequenceDiagram" || line.starts_with("sequenceDiagram ") {
                started = true;
            }
            continue;
        }
        let lower = line.to_ascii_lowercase();
        // Harmless directives we can ignore.
        if lower == "autonumber"
            || lower.starts_with("autonumber ")
            || lower.starts_with("title ")
            || lower.starts_with("acctitle")
            || lower.starts_with("accdescr")
        {
            continue;
        }
        if let Some(word) = lower.split_whitespace().next() {
            if SEQ_UNSUPPORTED.contains(&word) {
                return None;
            }
        }
        // participant/actor declarations.
        let decl = if lower.starts_with("participant ") {
            Some(&line["participant ".len()..])
        } else if lower.starts_with("actor ") {
            Some(&line["actor ".len()..])
        } else {
            None
        };
        if let Some(rest) = decl {
            let rest = rest.trim();
            let (id, label) = match rest.split_once(" as ") {
                Some((id, alias)) => (id.trim(), alias.trim()),
                None => (rest, rest),
            };
            if id.is_empty() {
                return None;
            }
            let i = intern(&mut actors, &mut index, id);
            actors[i].label = if label.is_empty() {
                id.to_string()
            } else {
                label.to_string()
            };
            continue;
        }
        // Notes.
        if lower.starts_with("note ") {
            let rest = line[5..].trim();
            let (targets, text) = rest.split_once(':')?;
            let (targets, text) = (targets.trim(), text.trim().to_string());
            let t_lower = targets.to_ascii_lowercase();
            let (side, names) = if t_lower.starts_with("over ") {
                (NoteSide::Over, &targets[5..])
            } else if t_lower.starts_with("left of ") {
                (NoteSide::Left, &targets[8..])
            } else if t_lower.starts_with("right of ") {
                (NoteSide::Right, &targets[9..])
            } else {
                return None;
            };
            let mut over = Vec::new();
            for name in names.split(',') {
                let name = name.trim();
                if !name.is_empty() {
                    over.push(intern(&mut actors, &mut index, name));
                }
            }
            if over.is_empty() {
                return None;
            }
            steps.push(SeqStep::Note { over, text, side });
            continue;
        }
        // Messages.
        let (from, to, tok) = split_arrow(line)?;
        let (_, text) = line.split_once(':')?;
        if from.is_empty() || to.is_empty() {
            return None;
        }
        let from = intern(&mut actors, &mut index, from);
        let to = intern(&mut actors, &mut index, to);
        steps.push(SeqStep::Message {
            from,
            to,
            text: text.trim().to_string(),
            dashed: tok.starts_with("--"),
            open: !tok.ends_with('>'),
        });
    }
    if !started || steps.is_empty() {
        return None;
    }
    Some(Sequence { actors, steps })
}

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
                    body: Vec::new(),
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
    let body_font = egui::FontId::proportional(11.0);
    let galleys: Vec<Arc<egui::Galley>> = g
        .nodes
        .iter()
        .map(|n| {
            ui.painter()
                .layout_no_wrap(n.label.clone(), font.clone(), fg)
        })
        .collect();
    let bodies: Vec<Vec<Arc<egui::Galley>>> = g
        .nodes
        .iter()
        .map(|n| {
            n.body
                .iter()
                .map(|l| {
                    ui.painter()
                        .layout_no_wrap(l.clone(), body_font.clone(), fg)
                })
                .collect()
        })
        .collect();
    let line_h = 15.0f32;
    let sizes: Vec<egui::Vec2> = g
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if n.kind == NodeKind::Circle {
                return egui::vec2(18.0, 18.0);
            }
            let body_w = bodies[i]
                .iter()
                .map(|gl| gl.size().x)
                .fold(0.0f32, f32::max);
            let w = (galleys[i].size().x.max(body_w) + pad.x * 2.0).max(64.0);
            let h = galleys[i].size().y + bodies[i].len() as f32 * line_h + pad.y * 2.0;
            egui::vec2(w, h)
        })
        .collect();

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
        if g.nodes[i].kind == NodeKind::Circle {
            painter.circle_filled(*c, sizes[i].x / 2.0, fg.gamma_multiply(0.9));
            continue;
        }
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
        // Title centred in the top strip; member lines left-aligned under it.
        let gl = &galleys[i];
        let title_top = rect.top() + pad.y;
        let mut y = title_top;
        if bodies[i].is_empty() {
            y = c.y - gl.size().y / 2.0;
        }
        painter.galley(egui::pos2(c.x - gl.size().x / 2.0, y), gl.clone(), fg);
        if !bodies[i].is_empty() {
            let mut by = title_top + gl.size().y + 3.0;
            let sep = egui::Stroke::new(1.0_f32, border);
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 4.0, by - 3.0),
                    egui::pos2(rect.right() - 4.0, by - 3.0),
                ],
                sep,
            );
            for line in &bodies[i] {
                painter.galley(egui::pos2(rect.left() + pad.x, by), line.clone(), fg);
                by += line_h;
            }
        }
    }
}

/// Parse a `pie` chart: `pie [showData] [title X]` then `"Label" : value` rows.
fn parse_pie(src: &str) -> Option<Pie> {
    let mut title = None;
    let mut slices = Vec::new();
    let mut started = false;
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }
        if !started {
            if line == "pie" || line.starts_with("pie ") {
                started = true;
                // `pie title X` / `pie showData title X`
                if let Some((_, t)) = line.split_once("title ") {
                    let t = t.trim();
                    if !t.is_empty() {
                        title = Some(t.to_string());
                    }
                }
            }
            continue;
        }
        if let Some(t) = line.strip_prefix("title ") {
            let t = t.trim();
            if !t.is_empty() {
                title = Some(t.to_string());
            }
            continue;
        }
        let (label, value) = line.split_once(':')?;
        let label = label.trim().trim_matches('"').trim();
        let value: f64 = value.trim().parse().ok()?;
        if label.is_empty() || !value.is_finite() || value <= 0.0 {
            return None;
        }
        slices.push((label.to_string(), value));
    }
    if !started || slices.is_empty() {
        return None;
    }
    Some(Pie { title, slices })
}

/// A small qualitative palette for pie slices (readable on a dark panel).
const PIE_COLORS: [(u8, u8, u8); 8] = [
    (0x88, 0xc0, 0xd0),
    (0xa3, 0xbe, 0x8c),
    (0xeb, 0xcb, 0x8b),
    (0xbf, 0x61, 0x6a),
    (0xb4, 0x8e, 0xad),
    (0x81, 0xa1, 0xc1),
    (0x8f, 0xbc, 0xbb),
    (0xd0, 0x87, 0x70),
];

/// Render a pie chart with a legend.
pub fn show_pie(ui: &mut egui::Ui, p: &Pie, fg: egui::Color32, panel: egui::Color32) {
    let font = egui::FontId::proportional(12.0);
    let total: f64 = p.slices.iter().map(|(_, v)| *v).sum();
    let radius = 96.0f32;
    let legend: Vec<(Arc<egui::Galley>, String)> = p
        .slices
        .iter()
        .map(|(label, value)| {
            let pct = *value / total * 100.0;
            let num = if value.fract().abs() < f64::EPSILON {
                format!("{}", *value as i64)
            } else {
                format!("{value:.1}")
            };
            (
                ui.painter().layout_no_wrap(label.clone(), font.clone(), fg),
                format!("{num} ({pct:.0}%)"),
            )
        })
        .collect();
    // Two columns: the label, then its value, each sized to the widest entry.
    let value_galleys: Vec<Arc<egui::Galley>> = legend
        .iter()
        .map(|(_, v)| {
            ui.painter()
                .layout_no_wrap(v.clone(), font.clone(), fg.gamma_multiply(0.8))
        })
        .collect();
    let label_w = legend
        .iter()
        .map(|(gl, _)| gl.size().x)
        .fold(0.0f32, f32::max);
    let value_w = value_galleys
        .iter()
        .map(|gl| gl.size().x)
        .fold(0.0f32, f32::max);
    let legend_w = 14.0 + label_w + 12.0 + value_w + 12.0;
    let title_h = if p.title.is_some() { 20.0 } else { 0.0 };
    let size = egui::vec2(
        (radius * 2.0 + 16.0 + legend_w).min(ui.available_width().max(200.0)),
        (radius * 2.0 + title_h + 8.0).max(80.0),
    );
    let (resp, painter) = ui.allocate_painter(size, egui::Sense::hover());
    let rect = resp.rect;
    if let Some(t) = &p.title {
        let gl = painter.layout_no_wrap(t.clone(), egui::FontId::proportional(13.0), fg);
        painter.galley(rect.min, gl, fg);
    }
    let centre = egui::pos2(rect.left() + radius + 8.0, rect.top() + title_h + radius);
    let mut angle = -std::f32::consts::FRAC_PI_2;
    for (i, (_, value)) in p.slices.iter().enumerate() {
        let sweep = (*value / total) as f32 * std::f32::consts::TAU;
        let (r, g, b) = PIE_COLORS[i % PIE_COLORS.len()];
        let color = egui::Color32::from_rgb(r, g, b);
        let steps = ((sweep / (std::f32::consts::PI / 24.0)).ceil() as usize).max(2);
        let mut pts = Vec::with_capacity(steps + 2);
        pts.push(centre);
        for s in 0..=steps {
            let a = angle + sweep * (s as f32 / steps as f32);
            pts.push(centre + egui::vec2(a.cos(), a.sin()) * radius);
        }
        painter.add(egui::Shape::convex_polygon(
            pts,
            color,
            egui::Stroke::new(1.0_f32, panel),
        ));
        angle += sweep;
    }
    let mut y = rect.top() + title_h + (radius * 2.0 - p.slices.len() as f32 * 18.0) / 2.0;
    for (i, (label_galley, value_text)) in legend.iter().enumerate() {
        let (r, g, b) = PIE_COLORS[i % PIE_COLORS.len()];
        let swatch = egui::Rect::from_min_size(
            egui::pos2(centre.x + radius + 16.0, y + 4.0),
            egui::vec2(9.0, 9.0),
        );
        painter.rect_filled(
            swatch,
            egui::Rounding::same(2.0),
            egui::Color32::from_rgb(r, g, b),
        );
        painter.galley(
            egui::pos2(swatch.right() + 5.0, y),
            label_galley.clone(),
            fg,
        );
        painter.galley(
            egui::pos2(swatch.right() + 5.0 + label_w + 12.0, y),
            value_galleys[i].clone(),
            fg.gamma_multiply(0.8),
        );
        let _ = value_text;
        y += 18.0;
    }
}

/// Render a sequence diagram: actor boxes with lifelines, messages between
/// them and notes. Laid out in a single column per participant, top to bottom.
pub fn show_sequence(ui: &mut egui::Ui, s: &Sequence, fg: egui::Color32, panel: egui::Color32) {
    let actor_font = egui::FontId::proportional(13.0);
    let text_font = egui::FontId::proportional(11.5);
    let pad = egui::vec2(12.0, 6.0);
    let margin = 12.0f32;
    let col_gap = 56.0f32;

    let actor_galleys: Vec<Arc<egui::Galley>> = s
        .actors
        .iter()
        .map(|a| {
            ui.painter()
                .layout_no_wrap(a.label.clone(), actor_font.clone(), fg)
        })
        .collect();
    let actor_sizes: Vec<egui::Vec2> = actor_galleys
        .iter()
        .map(|gl| {
            egui::vec2(
                (gl.size().x + pad.x * 2.0).max(64.0),
                gl.size().y + pad.y * 2.0,
            )
        })
        .collect();

    // Column centres, left to right.
    let mut cx: Vec<f32> = Vec::with_capacity(actor_sizes.len());
    let mut x = margin;
    for (i, sz) in actor_sizes.iter().enumerate() {
        if i == 0 {
            cx.push(x + sz.x / 2.0);
            x += sz.x;
        } else {
            x += col_gap;
            cx.push(x + sz.x / 2.0);
            x += sz.x;
        }
    }
    let row_h = actor_sizes.iter().map(|s| s.y).fold(0.0f32, f32::max);
    let box_bottom = margin + row_h;

    // Steps: lay out vertically and remember what each one needs to draw.
    struct Msg<'a> {
        from: usize,
        to: usize,
        y: f32,
        galley: Arc<egui::Galley>,
        dashed: bool,
        open: bool,
        _t: std::marker::PhantomData<&'a ()>,
    }
    struct Note<'a> {
        rect: egui::Rect,
        galley: Arc<egui::Galley>,
        _h: std::marker::PhantomData<&'a ()>,
    }
    let mut msgs: Vec<Msg> = Vec::new();
    let mut notes: Vec<Note> = Vec::new();
    let mut y = box_bottom + 16.0;
    for step in &s.steps {
        match step {
            SeqStep::Message {
                from,
                to,
                text,
                dashed,
                open,
            } => {
                let galley = ui
                    .painter()
                    .layout_no_wrap(text.clone(), text_font.clone(), fg);
                y += galley.size().y + 6.0;
                msgs.push(Msg {
                    from: *from,
                    to: *to,
                    y,
                    galley,
                    dashed: *dashed,
                    open: *open,
                    _t: std::marker::PhantomData,
                });
                y += 14.0;
            }
            SeqStep::Note { over, text, side } => {
                let galley = ui
                    .painter()
                    .layout_no_wrap(text.clone(), text_font.clone(), fg);
                let lo = over.iter().copied().min().unwrap_or(0);
                let hi = over.iter().copied().max().unwrap_or(0);
                let (left, right) = (cx[lo], cx[hi]);
                let span = (right - left).max(56.0);
                let width = (galley.size().x + pad.x * 2.0).min(span);
                let size = egui::vec2(width, galley.size().y + pad.y * 2.0);
                let centre_x = match side {
                    NoteSide::Over => (left + right) / 2.0,
                    NoteSide::Left => left - 8.0 - size.x / 2.0,
                    NoteSide::Right => right + 8.0 + size.x / 2.0,
                };
                let rect =
                    egui::Rect::from_center_size(egui::pos2(centre_x, y + size.y / 2.0), size);
                y += size.y + 10.0;
                notes.push(Note {
                    rect,
                    galley,
                    _h: std::marker::PhantomData,
                });
            }
        }
    }
    let content_bottom = y + 6.0;
    let width = (x + margin).max(200.0).min(ui.available_width().max(200.0));
    // Notes placed beside a lifeline can stick out; slide them back inside.
    for note in &mut notes {
        let (lo, hi) = (margin, width - margin);
        if note.rect.left() < lo {
            note.rect = note.rect.translate(egui::vec2(lo - note.rect.left(), 0.0));
        } else if note.rect.right() > hi {
            note.rect = note.rect.translate(egui::vec2(hi - note.rect.right(), 0.0));
        }
    }
    let size = egui::vec2(width, content_bottom);
    let (resp, painter) = ui.allocate_painter(size, egui::Sense::hover());
    let origin = resp.rect.min.to_vec2();
    let at = |p: egui::Pos2| p + origin;

    let line = egui::Stroke::new(1.2_f32, fg.gamma_multiply(0.5));
    let flow = egui::Stroke::new(1.3_f32, fg.gamma_multiply(0.85));

    // Lifelines down from each actor box.
    for (i, c) in cx.iter().enumerate() {
        let top = at(egui::pos2(*c, box_bottom));
        let bottom = at(egui::pos2(*c, content_bottom - 4.0));
        painter.extend(egui::Shape::dashed_line(&[top, bottom], line, 4.0, 4.0));
        let box_rect = egui::Rect::from_center_size(
            at(egui::pos2(*c, margin + actor_sizes[i].y / 2.0)),
            actor_sizes[i],
        );
        painter.rect_filled(box_rect, egui::Rounding::same(4.0), panel);
        for (a, b) in [
            (box_rect.left_top(), box_rect.right_top()),
            (box_rect.right_top(), box_rect.right_bottom()),
            (box_rect.right_bottom(), box_rect.left_bottom()),
            (box_rect.left_bottom(), box_rect.left_top()),
        ] {
            painter.line_segment([a, b], line);
        }
        let gl = &actor_galleys[i];
        painter.galley(box_rect.center() - gl.size() / 2.0, gl.clone(), fg);
    }

    // Arrowheads for each message.
    for m in &msgs {
        if m.from == m.to {
            // Self-message: a small loop to the right of the lifeline.
            let c = cx[m.from];
            let y0 = m.y;
            let y1 = m.y + 10.0;
            let x1 = c + 26.0;
            for (a, b) in [
                (egui::pos2(c, y0), egui::pos2(x1, y0)),
                (egui::pos2(x1, y0), egui::pos2(x1, y1)),
                (egui::pos2(x1, y1), egui::pos2(c + 6.0, y1)),
            ] {
                let (a, b) = (at(a), at(b));
                if m.dashed {
                    painter.extend(egui::Shape::dashed_line(&[a, b], flow, 4.0, 3.0));
                } else {
                    painter.line_segment([a, b], flow);
                }
            }
            let tip = at(egui::pos2(c + 6.0, y1));
            painter.add(egui::Shape::convex_polygon(
                vec![tip, tip + egui::vec2(8.0, -4.0), tip + egui::vec2(8.0, 4.0)],
                fg.gamma_multiply(0.85),
                egui::Stroke::NONE,
            ));
            let r = egui::Rect::from_min_size(
                at(egui::pos2(c + 32.0, y0 - m.galley.size().y - 2.0)),
                m.galley.size(),
            );
            painter.galley(r.min, m.galley.clone(), fg);
            continue;
        }
        let a = at(egui::pos2(cx[m.from], m.y));
        let b = at(egui::pos2(cx[m.to], m.y));
        if m.dashed {
            painter.extend(egui::Shape::dashed_line(&[a, b], flow, 4.0, 3.0));
        } else {
            painter.line_segment([a, b], flow);
        }
        let dir = (b - a).normalized();
        let n = egui::vec2(-dir.y, dir.x);
        if m.open {
            // Open head: two strokes.
            painter.line_segment([b, b - dir * 9.0 + n * 4.0], flow);
            painter.line_segment([b, b - dir * 9.0 - n * 4.0], flow);
        } else {
            painter.add(egui::Shape::convex_polygon(
                vec![b, b - dir * 9.0 + n * 4.0, b - dir * 9.0 - n * 4.0],
                fg.gamma_multiply(0.85),
                egui::Stroke::NONE,
            ));
        }
        let mid = a + (b - a) * 0.5;
        let r = egui::Rect::from_center_size(
            egui::pos2(mid.x, mid.y - m.galley.size().y / 2.0 - 2.0),
            m.galley.size() + egui::vec2(6.0, 2.0),
        );
        painter.rect_filled(r, egui::Rounding::same(3.0), panel);
        painter.galley(r.min + egui::vec2(3.0, 1.0), m.galley.clone(), fg);
    }

    // Notes on top.
    for note in &notes {
        let rect = note.rect.translate(origin);
        painter.rect_filled(rect, egui::Rounding::same(4.0), panel);
        for (a, b) in [
            (rect.left_top(), rect.right_top()),
            (rect.right_top(), rect.right_bottom()),
            (rect.right_bottom(), rect.left_bottom()),
            (rect.left_bottom(), rect.left_top()),
        ] {
            painter.line_segment([a, b], line);
        }
        let inner = rect.shrink2(pad);
        painter.galley(
            egui::pos2(inner.left(), inner.center().y - note.galley.size().y / 2.0),
            note.galley.clone(),
            fg,
        );
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
    let status = miao_term_platform::background_command(cmd)
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
        // `parse` is graph-only; `parse_diagram` dispatches on the header.
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
    fn parses_sequence_diagram() {
        let src = "sequenceDiagram\n  participant A as Alice\n  actor B\n  A->>B: Hello\n  B-->>A: Hi\n  A-xB: gone\n  Note over A,B: shared\n  Note right of B: aside";
        let Diagram::Sequence(seq) = parse_diagram(src).expect("parses") else {
            panic!("expected a sequence diagram");
        };
        assert_eq!(seq.actors.len(), 2);
        assert_eq!(seq.actors[0].label, "Alice");
        assert_eq!(seq.actors[1].label, "B");
        assert_eq!(seq.steps.len(), 5);
        match &seq.steps[0] {
            SeqStep::Message {
                from,
                to,
                text,
                dashed,
                open,
            } => {
                assert_eq!((*from, *to), (0, 1));
                assert_eq!(text, "Hello");
                assert!(!dashed && !open);
            }
            other => panic!("expected a message, got {other:?}"),
        }
        match &seq.steps[1] {
            SeqStep::Message { dashed, open, .. } => assert!(*dashed && !*open),
            other => panic!("expected a message, got {other:?}"),
        }
        match &seq.steps[2] {
            SeqStep::Message { open, .. } => assert!(*open, "`-x` has an open head"),
            other => panic!("expected a message, got {other:?}"),
        }
        match &seq.steps[3] {
            SeqStep::Note { over, side, .. } => {
                assert_eq!(over, &[0, 1]);
                assert_eq!(*side, NoteSide::Over);
            }
            other => panic!("expected a note, got {other:?}"),
        }
        match &seq.steps[4] {
            SeqStep::Note { side, .. } => assert_eq!(*side, NoteSide::Right),
            other => panic!("expected a note, got {other:?}"),
        }
    }

    #[test]
    fn sequence_declares_participants_implicitly() {
        let Diagram::Sequence(seq) =
            parse_diagram("sequenceDiagram\n  X->>Y: hi\n  Y->>Z: fwd").unwrap()
        else {
            panic!("expected a sequence diagram");
        };
        assert_eq!(
            seq.actors.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            vec!["X", "Y", "Z"]
        );
    }

    #[test]
    fn sequence_rejects_blocks_we_do_not_model() {
        assert!(parse_diagram("sequenceDiagram\n loop 3 times\n A->>B: hi\n end").is_none());
        assert!(parse_diagram("sequenceDiagram\n A->>B: hi\n alt yes\n end").is_none());
        assert!(parse_diagram("sequenceDiagram\n  hello").is_none());
        // A sequence header with no steps is not renderable.
        assert!(parse_diagram("sequenceDiagram\n participant A").is_none());
    }

    #[test]
    fn shows_sequence_diagram_headless() {
        let Diagram::Sequence(seq) = parse_diagram(
            "sequenceDiagram\n participant A\n participant B\n A->>B: one\n B-->A: two\n Note over A,B: both",
        )
        .unwrap()
        else {
            panic!("expected a sequence diagram");
        };
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
                show_sequence(ui, &seq, egui::Color32::WHITE, egui::Color32::BLACK);
            });
        });
        assert!(
            !out.shapes.is_empty(),
            "mermaid::show_sequence drew nothing"
        );
    }

    #[test]
    fn parses_pie_charts() {
        let Diagram::Pie(pie) =
            parse_diagram("pie title Pets\n  \"Dogs\" : 386\n  Cats : 85").unwrap()
        else {
            panic!("expected a pie chart");
        };
        assert_eq!(pie.title.as_deref(), Some("Pets"));
        assert_eq!(pie.slices.len(), 2);
        assert_eq!(pie.slices[0], ("Dogs".to_string(), 386.0));
        assert_eq!(pie.slices[1], ("Cats".to_string(), 85.0));
        // `pie showData title ...` on the header line works too.
        let Diagram::Pie(p2) = parse_diagram("pie showData title T\n a : 1\n b : 1").unwrap()
        else {
            panic!("expected a pie chart");
        };
        assert_eq!(p2.title.as_deref(), Some("T"));
        // Bad input is refused rather than half-drawn.
        assert!(parse_diagram("pie\n a : nope").is_none());
        assert!(parse_diagram("pie\n a : 0").is_none());
        assert!(parse_diagram("pie title Only").is_none());
    }

    #[test]
    fn shows_pie_chart_headless() {
        let Diagram::Pie(pie) = parse_diagram("pie title P\n one : 3\n two : 1").unwrap() else {
            panic!("expected a pie chart");
        };
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
                show_pie(ui, &pie, egui::Color32::WHITE, egui::Color32::BLACK);
            });
        });
        assert!(!out.shapes.is_empty(), "mermaid::show_pie drew nothing");
    }

    #[test]
    fn parses_state_diagrams() {
        let Diagram::Graph(g) = parse_diagram(
            "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Running: start\n  Running --> [*]",
        )
        .unwrap() else {
            panic!("expected a graph for a state diagram");
        };
        assert_eq!(g.nodes.len(), 3, "two states plus the [*] marker");
        assert_eq!(g.nodes[0].kind, NodeKind::Circle);
        assert_eq!(g.nodes[1].label, "Idle");
        assert_eq!(g.edges.len(), 3);
        assert_eq!(g.edges[1].label.as_deref(), Some("start"));
        // Aliases and rejection of what we do not model.
        let Diagram::Graph(alias) =
            parse_diagram("stateDiagram\n state \"Long name\" as S\n [*] --> S").unwrap()
        else {
            panic!("expected a graph");
        };
        assert_eq!(alias.nodes[0].label, "Long name");
        assert!(parse_diagram("stateDiagram\n note left of A: hi\n [*] --> A").is_none());
    }

    #[test]
    fn parses_class_diagrams() {
        let Diagram::Graph(g) = parse_diagram(
            "classDiagram\n  class Animal {\n    +String name\n    +speak()\n  }\n  Animal <|-- Dog\n  Dog : +bark()",
        )
        .unwrap()
        else {
            panic!("expected a graph for a class diagram");
        };
        assert_eq!(g.nodes.len(), 2);
        let animal = g.nodes.iter().find(|n| n.label == "Animal").unwrap();
        assert_eq!(animal.kind, NodeKind::Class);
        assert_eq!(animal.body, vec!["+String name", "+speak()"]);
        let dog = g.nodes.iter().find(|n| n.label == "Dog").unwrap();
        assert_eq!(dog.body, vec!["+bark()"]);
        assert_eq!(g.edges.len(), 1);
        assert_eq!((g.edges[0].from, g.edges[0].to), (0, 1));
    }

    #[test]
    fn parses_er_diagrams() {
        let Diagram::Graph(g) = parse_diagram(
            "erDiagram\n  CUSTOMER {\n    string name\n  }\n  CUSTOMER ||--o{ ORDER : places",
        )
        .unwrap() else {
            panic!("expected a graph for an ER diagram");
        };
        assert_eq!(g.nodes.len(), 2);
        let customer = g.nodes.iter().find(|n| n.label == "CUSTOMER").unwrap();
        assert_eq!(customer.body, vec!["string name"]);
        let order = g.nodes.iter().find(|n| n.label == "ORDER").unwrap();
        assert!(order.body.is_empty(), "implicitly declared entity");
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].label.as_deref(), Some("places"));
    }

    #[test]
    fn shows_state_and_class_diagrams_headless() {
        for src in [
            "stateDiagram-v2\n [*] --> A\n A --> B: go\n B --> [*]",
            "classDiagram\n class A {\n +x\n }\n A <|-- B",
            "erDiagram\n A {\n int id\n }\n A ||--o{ B : has",
        ] {
            let Diagram::Graph(g) = parse_diagram(src).unwrap() else {
                panic!("expected a graph for {src}");
            };
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
            assert!(!out.shapes.is_empty(), "drew nothing for {src}");
        }
    }

    #[test]
    fn dispatches_graphs_and_sequences() {
        assert!(matches!(
            parse_diagram("graph TD\n A --> B"),
            Some(Diagram::Graph(_))
        ));
        assert!(matches!(
            parse_diagram("sequenceDiagram\n A->>B: hi"),
            Some(Diagram::Sequence(_))
        ));
        assert!(parse_diagram("not a diagram").is_none());
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
