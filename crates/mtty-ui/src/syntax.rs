//! A tiny, dependency-free syntax highlighter for the built-in editor.
//!
//! It is intentionally lexical (comments, strings, numbers, keywords, calls,
//! types) rather than a full parser: good enough to make code readable, cheap
//! enough to run on every keystroke, and it never needs extra crates.

use std::ops::Range;

/// The languages we have a keyword set for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Rust,
    Python,
    Shell,
    Json,
    Toml,
    JavaScript,
    Plain,
}

/// Guess the language from a path or file name.
pub fn detect(path: &str) -> Lang {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Lang::Rust,
        "py" | "pyi" => Lang::Python,
        "sh" | "bash" | "zsh" | "fish" => Lang::Shell,
        "json" => Lang::Json,
        "toml" => Lang::Toml,
        "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" => Lang::JavaScript,
        _ => match name {
            ".bashrc" | ".zshrc" | ".profile" => Lang::Shell,
            _ => Lang::Plain,
        },
    }
}

/// A token kind; the layouter maps these to colors.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Plain,
    Keyword,
    Str,
    Comment,
    Number,
    Func,
    Type,
}

fn line_comment(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust | Lang::JavaScript => "//",
        Lang::Python | Lang::Shell | Lang::Toml => "#",
        _ => "",
    }
}

fn keywords(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::Rust => &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while",
        ],
        Lang::Python => &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
            "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import",
            "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return",
            "True", "try", "while", "with", "yield",
        ],
        Lang::Shell => &[
            "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if",
            "in", "local", "return", "then", "until", "while",
        ],
        Lang::JavaScript => &[
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "default",
            "delete",
            "do",
            "else",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "function",
            "if",
            "import",
            "in",
            "instanceof",
            "let",
            "new",
            "null",
            "return",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "undefined",
            "var",
            "void",
            "while",
            "yield",
        ],
        Lang::Toml | Lang::Json | Lang::Plain => &[],
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}
fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Split `text` into colored ranges. Ranges are byte offsets into `text`.
pub fn tokens(text: &str, lang: Lang) -> Vec<(Range<usize>, Tok)> {
    let mut out: Vec<(Range<usize>, Tok)> = Vec::new();
    let bytes = text.as_bytes();
    let comment = line_comment(lang);
    let kws = keywords(lang);
    let mut i = 0usize;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        // Line comment.
        if !comment.is_empty() && text[i..].starts_with(comment) {
            let end = text[i..].find('\n').map(|p| i + p).unwrap_or(text.len());
            out.push((i..end, Tok::Comment));
            i = end;
            continue;
        }
        // String literal.
        if c == '"' || c == '\'' {
            let mut j = i + c.len_utf8();
            while j < text.len() {
                let d = text[j..].chars().next().unwrap();
                if d == '\\' {
                    j += 1;
                    if j < text.len() {
                        j += text[j..].chars().next().unwrap().len_utf8();
                    }
                    continue;
                }
                j += d.len_utf8();
                if d == c {
                    break;
                }
            }
            out.push((i..j.min(text.len()), Tok::Str));
            i = j;
            continue;
        }
        // Number.
        if c.is_ascii_digit() {
            let mut j = i + 1;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric()
                    || bytes[j] == b'.'
                    || bytes[j] == b'_'
                    || bytes[j] == b'x')
            {
                j += 1;
            }
            out.push((i..j, Tok::Number));
            i = j;
            continue;
        }
        // Identifier / keyword.
        if is_ident_start(c) {
            let mut j = i + c.len_utf8();
            while j < text.len() {
                let d = text[j..].chars().next().unwrap();
                if !is_ident(d) {
                    break;
                }
                j += d.len_utf8();
            }
            let word = &text[i..j];
            let tok = if kws.contains(&word) {
                Tok::Keyword
            } else if word.chars().next().map(is_ident_start).unwrap_or(false)
                && word.chars().next().unwrap().is_uppercase()
                && lang != Lang::Shell
            {
                Tok::Type
            } else if text[j..].trim_start().starts_with('(') {
                Tok::Func
            } else {
                Tok::Plain
            };
            out.push((i..j, tok));
            i = j;
            continue;
        }
        // Everything else, one char at a time.
        let j = i + c.len_utf8();
        out.push((i..j, Tok::Plain));
        i = j;
    }
    out
}

fn tok_color(tok: Tok, plain: egui::Color32) -> egui::Color32 {
    let c = |r, g, b| egui::Color32::from_rgb(r, g, b);
    match tok {
        Tok::Plain => plain,
        Tok::Keyword => c(0xc6, 0x78, 0xdd),
        Tok::Str => c(0x98, 0xc3, 0x79),
        Tok::Comment => c(0x5c, 0x63, 0x70),
        Tok::Number => c(0xd1, 0x9a, 0x66),
        Tok::Func => c(0x61, 0xaf, 0xef),
        Tok::Type => c(0xe5, 0xc0, 0x7b),
    }
}

/// An egui `TextEdit` layouter that colors `text` for `lang`.
pub fn layouter(
    lang: Lang,
    plain: egui::Color32,
    size: f32,
) -> impl FnMut(&egui::Ui, &str, f32) -> std::sync::Arc<egui::Galley> {
    move |ui: &egui::Ui, text: &str, wrap_width: f32| {
        let font = egui::FontId::monospace(size);
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = wrap_width;
        for (range, tok) in tokens(text, lang) {
            job.append(
                &text[range],
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: tok_color(tok, plain),
                    ..Default::default()
                },
            );
        }
        ui.fonts(|f| f.layout_job(job))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension() {
        assert_eq!(detect("a/b/main.rs"), Lang::Rust);
        assert_eq!(detect("x.py"), Lang::Python);
        assert_eq!(detect(".zshrc"), Lang::Shell);
        assert_eq!(detect("README"), Lang::Plain);
    }

    #[test]
    fn classifies_tokens() {
        let text = "fn main() { let x = \"hi\"; } // done";
        let toks = tokens(text, Lang::Rust);
        let kw = |w: &str| {
            let at = text.find(w).unwrap();
            toks.iter()
                .any(|(r, t)| r.start == at && *t == Tok::Keyword)
        };
        assert!(kw("fn"));
        assert!(kw("let"));
        assert!(toks.iter().any(|(_, t)| *t == Tok::Str));
        assert!(toks.iter().any(|(_, t)| *t == Tok::Comment));
        // `main` is a call.
        let at = text.find("main").unwrap();
        assert!(toks.iter().any(|(r, t)| r.start == at && *t == Tok::Func));
    }

    #[test]
    fn python_comment_and_string() {
        let text = "def f():  # greet\n    return 'x'";
        let toks = tokens(text, Lang::Python);
        assert!(toks
            .iter()
            .any(|(r, t)| *t == Tok::Comment && &text[r.clone()] == "# greet"));
        assert!(toks
            .iter()
            .any(|(r, t)| *t == Tok::Str && &text[r.clone()] == "'x'"));
    }
}
