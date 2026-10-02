//! Fallback highlighting with syntect's built-in Sublime syntaxes, for files
//! no tree-sitter grammar claims (ADR 0034, E3): OCaml, LaTeX, D, Pascal,
//! Tcl, Graphviz and the rest of syntect's default set.
//!
//! Sublime syntaxes are line-based and need the parser state from the top of
//! the file, so a checkpoint of that state is kept every [`STRIDE`] lines.
//! An edit drops the checkpoints after it; drawing parses from the nearest
//! checkpoint to the bottom of the screen, never the whole file.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;

use ropey::Rope;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use crate::syntax::Highlight;

/// Lines between parser checkpoints.
const STRIDE: usize = 64;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// What a TextMate scope paints, by its most specific known prefix.
fn classify(scope: &str) -> Option<Highlight> {
    const RULES: &[(&str, Highlight)] = &[
        // Quote and comment delimiters take the colour of what they delimit.
        ("punctuation.definition.string", Highlight::String),
        ("punctuation.definition.comment", Highlight::Comment),
        ("comment", Highlight::Comment),
        ("constant.character.escape", Highlight::Escape),
        ("string", Highlight::String),
        ("constant.numeric", Highlight::Number),
        ("constant.language", Highlight::Number),
        ("constant", Highlight::Constant),
        ("keyword.operator", Highlight::Operator),
        ("keyword", Highlight::Keyword),
        ("storage.type", Highlight::Keyword),
        ("storage", Highlight::Keyword),
        ("entity.name.function", Highlight::Function),
        ("support.function", Highlight::Function),
        ("meta.function-call", Highlight::Function),
        ("entity.name.type", Highlight::Type),
        ("entity.name.class", Highlight::Type),
        ("entity.other.inherited-class", Highlight::Type),
        ("support.type", Highlight::Type),
        ("support.class", Highlight::Type),
        ("entity.name.tag", Highlight::Tag),
        ("entity.other.attribute-name", Highlight::Attribute),
        ("variable.parameter", Highlight::Variable),
        ("variable.language", Highlight::Variable),
        ("variable.other.member", Highlight::Property),
        ("markup.heading", Highlight::Heading),
        ("markup.underline.link", Highlight::Link),
        ("markup.raw", Highlight::String),
        ("punctuation", Highlight::Punctuation),
    ];
    RULES
        .iter()
        .find(|(prefix, _)| {
            scope == *prefix
                || (scope.starts_with(prefix) && scope.as_bytes().get(prefix.len()) == Some(&b'.'))
        })
        .map(|(_, h)| *h)
}

/// A file's Sublime syntax, when syntect has one for its extension, name or
/// first line (and it is not plain text).
fn syntax_for(path: &Path, first_line: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let by_ext = path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|e| set.find_syntax_by_extension(e));
    let by_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| set.find_syntax_by_extension(n));
    by_ext
        .or(by_name)
        .or_else(|| set.find_syntax_by_first_line(first_line))
        .filter(|s| s.name != "Plain Text")
}

struct State {
    /// `checkpoints[k]`: the parser state at the start of line `k * STRIDE`.
    checkpoints: Vec<(ParseState, ScopeStack)>,
    /// What each scope seen so far paints (scope names are built lazily).
    kinds: HashMap<Scope, Option<Highlight>>,
}

/// Fallback highlighting for one document.
pub struct Fallback {
    syntax: &'static SyntaxReference,
    state: RefCell<State>,
}

impl Fallback {
    pub fn for_file(path: &Path, first_line: &str) -> Option<Fallback> {
        let syntax = syntax_for(path, first_line)?;
        Some(Fallback {
            syntax,
            state: RefCell::new(State {
                checkpoints: vec![(ParseState::new(syntax), ScopeStack::new())],
                kinds: HashMap::new(),
            }),
        })
    }

    pub fn name(&self) -> &'static str {
        &self.syntax.name
    }

    /// Text changed from `line` on: later checkpoints are stale.
    pub fn invalidate_from(&self, line: usize) {
        let keep = line / STRIDE + 1;
        self.state.borrow_mut().checkpoints.truncate(keep.max(1));
    }

    /// Highlighted byte ranges within `range` (whole lines are parsed).
    pub fn highlights(&self, rope: &Rope, range: Range<usize>) -> Vec<(Range<usize>, Highlight)> {
        if rope.len_bytes() == 0 || range.start >= range.end {
            return Vec::new();
        }
        let first = rope.byte_to_line(range.start.min(rope.len_bytes()));
        let last = rope.byte_to_line(range.end.saturating_sub(1).min(rope.len_bytes()));
        let mut state = self.state.borrow_mut();
        let state = &mut *state;
        // Resume from the last checkpoint at or before `first`.
        let k = (first / STRIDE).min(state.checkpoints.len() - 1);
        let (mut parse, mut stack) = state.checkpoints[k].clone();
        let mut out: Vec<(Range<usize>, Highlight)> = Vec::new();
        let mut line_buf = String::new();
        for line in k * STRIDE..=last.min(rope.len_lines().saturating_sub(1)) {
            if line % STRIDE == 0 && line / STRIDE == state.checkpoints.len() {
                state.checkpoints.push((parse.clone(), stack.clone()));
            }
            line_buf.clear();
            line_buf.extend(rope.line(line).chunks());
            let Ok(ops) = parse.parse_line(&line_buf, syntaxes()) else {
                // A syntax error in the definition: leave the rest plain.
                break;
            };
            let start = rope.line_to_byte(line);
            let mut at = 0;
            let mut paint = |from: usize, to: usize, stack: &ScopeStack| {
                if line < first || from >= to {
                    return;
                }
                let kind = stack.as_slice().iter().rev().find_map(|scope| {
                    *state
                        .kinds
                        .entry(*scope)
                        .or_insert_with(|| classify(&scope.build_string()))
                });
                let (a, b) = (start + from, start + to);
                if let Some(kind) = kind {
                    let (a, b) = (a.max(range.start), b.min(range.end));
                    if a < b {
                        match out.last_mut() {
                            Some((r, k)) if *k == kind && r.end == a => r.end = b,
                            _ => out.push((a..b, kind)),
                        }
                    }
                }
            };
            for (offset, op) in ops {
                paint(at, offset, &stack);
                at = offset;
                if stack.apply(&op).is_err() {
                    break;
                }
            }
            paint(at, line_buf.len(), &stack);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds_at(f: &Fallback, rope: &Rope, needle: &str) -> Vec<Highlight> {
        let at = rope.to_string().find(needle).unwrap();
        f.highlights(rope, 0..rope.len_bytes())
            .into_iter()
            .filter(|(r, _)| r.start <= at && at < r.end)
            .map(|(_, k)| k)
            .collect()
    }

    #[test]
    fn ocaml_and_latex_come_from_the_default_set() {
        let rope = Rope::from_str("(* note *)\nlet x = \"s\" in 42\n");
        let f = Fallback::for_file(Path::new("a.ml"), "").unwrap();
        assert_eq!(f.name(), "OCaml");
        assert_eq!(kinds_at(&f, &rope, "(* note"), vec![Highlight::Comment]);
        assert_eq!(kinds_at(&f, &rope, "let"), vec![Highlight::Keyword]);
        assert_eq!(kinds_at(&f, &rope, "\"s\""), vec![Highlight::String]);
        assert_eq!(kinds_at(&f, &rope, "42"), vec![Highlight::Number]);
        let tex = Fallback::for_file(Path::new("paper.tex"), "").unwrap();
        assert_eq!(tex.name(), "LaTeX");
        assert!(Fallback::for_file(Path::new("notes.txt"), "").is_none());
    }

    #[test]
    fn checkpoints_resume_mid_file_and_follow_edits() {
        // A block comment opened at the top colours lines far below it, so
        // a screen in the middle must start from the right parser state.
        let mut text = String::from("(*\n");
        for i in 0..300 {
            text.push_str(&format!("line {i}\n"));
        }
        text.push_str("*)\nlet x = 1\n");
        let rope = Rope::from_str(&text);
        let f = Fallback::for_file(Path::new("a.ml"), "").unwrap();
        let mid = rope.line_to_byte(200)..rope.line_to_byte(201);
        assert_eq!(f.highlights(&rope, mid.clone())[0].1, Highlight::Comment);
        // Close the comment early: the middle is code now.
        let edited = Rope::from_str(&text.replacen("(*\n", "(**)\n", 1));
        f.invalidate_from(0);
        let mid = edited.line_to_byte(200)..edited.line_to_byte(201);
        assert!(f
            .highlights(&edited, mid)
            .iter()
            .all(|(_, k)| *k != Highlight::Comment));
    }

    #[test]
    fn scopes_map_by_dotted_prefix() {
        assert_eq!(
            classify("comment.line.double-slash"),
            Some(Highlight::Comment)
        );
        assert_eq!(
            classify("keyword.operator.arithmetic"),
            Some(Highlight::Operator)
        );
        assert_eq!(
            classify("constant.character.escape.ocaml"),
            Some(Highlight::Escape)
        );
        assert_eq!(classify("stringish"), None, "a prefix must end at a dot");
        assert_eq!(classify("source.ocaml"), None);
        assert_eq!(
            classify("punctuation.definition.string.begin.ocaml"),
            Some(Highlight::String)
        );
    }
}
