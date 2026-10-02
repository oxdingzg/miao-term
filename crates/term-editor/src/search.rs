//! Find and replace over a rope: literal or regex, optional case and
//! whole-word matching. Results are char ranges.

use regex::{Regex, RegexBuilder};
use ropey::Rope;

use crate::change::{Change, Transaction};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub pattern: String,
    /// Treat `pattern` as a regular expression (otherwise literal text).
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchError {
    /// The pattern is not a valid regular expression; the message says why.
    Invalid(String),
    /// Nothing to search for.
    Empty,
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::Invalid(why) => write!(f, "invalid pattern: {why}"),
            SearchError::Empty => write!(f, "empty pattern"),
        }
    }
}

impl std::error::Error for SearchError {}

impl SearchQuery {
    pub fn literal(pattern: impl Into<String>) -> Self {
        SearchQuery {
            pattern: pattern.into(),
            case_sensitive: true,
            ..Default::default()
        }
    }

    fn compile(&self) -> Result<Regex, SearchError> {
        if self.pattern.is_empty() {
            return Err(SearchError::Empty);
        }
        let body = if self.regex {
            self.pattern.clone()
        } else {
            regex::escape(&self.pattern)
        };
        let body = if self.whole_word {
            format!(r"\b(?:{body})\b")
        } else {
            body
        };
        RegexBuilder::new(&body)
            .case_insensitive(!self.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|e| SearchError::Invalid(e.to_string()))
    }
}

/// Above this size a document is searched in windows of whole lines rather
/// than as one string, so a search never copies all of a large text.
const WHOLE_TEXT_BYTES: usize = 64 << 20;

/// Lines per search window, by size.
const WINDOW_BYTES: usize = 4 << 20;

/// Every non-empty match, in order, as char ranges. In a document over
/// 64 MB a match that spans more than a 4 MB window of lines is not found.
pub fn find_all(rope: &Rope, query: &SearchQuery) -> Result<Vec<(usize, usize)>, SearchError> {
    let re = query.compile()?;
    if rope.len_bytes() <= WHOLE_TEXT_BYTES {
        let haystack = rope.to_string();
        return Ok(re
            .find_iter(&haystack)
            .filter(|m| m.start() < m.end())
            .map(|m| (rope.byte_to_char(m.start()), rope.byte_to_char(m.end())))
            .collect());
    }
    let (len, lines) = (rope.len_bytes(), rope.len_lines());
    let mut out = Vec::new();
    let mut line = 0;
    while line < lines {
        let start = rope.line_to_byte(line);
        // Whole lines, so a match within a line is never split.
        let end_line = (rope.byte_to_line((start + WINDOW_BYTES).min(len)) + 1).min(lines);
        let end = if end_line >= lines {
            len
        } else {
            rope.line_to_byte(end_line)
        };
        let window = rope.byte_slice(start..end).to_string();
        out.extend(
            re.find_iter(&window)
                .filter(|m| m.start() < m.end())
                .map(|m| {
                    (
                        rope.byte_to_char(start + m.start()),
                        rope.byte_to_char(start + m.end()),
                    )
                }),
        );
        line = end_line;
    }
    Ok(out)
}

/// The next match after `from` (or the previous one before it), wrapping
/// around the document.
pub fn find_next(
    rope: &Rope,
    query: &SearchQuery,
    from: usize,
    forward: bool,
) -> Result<Option<(usize, usize)>, SearchError> {
    let all = find_all(rope, query)?;
    let found = if forward {
        all.iter().find(|m| m.0 >= from).or(all.first())
    } else {
        all.iter().rev().find(|m| m.1 <= from).or(all.last())
    };
    Ok(found.copied())
}

/// One transaction replacing every match. In regex mode `replacement` may use
/// `$1` / `${name}` groups; in literal mode it is inserted as written.
pub fn replace_all(
    rope: &Rope,
    query: &SearchQuery,
    replacement: &str,
) -> Result<Transaction, SearchError> {
    let re = query.compile()?;
    let haystack = rope.to_string();
    let mut changes = Vec::new();
    for caps in re.captures_iter(&haystack) {
        let m = caps.get(0).unwrap();
        if m.start() == m.end() {
            continue;
        }
        let text = if query.regex {
            let mut out = String::new();
            caps.expand(replacement, &mut out);
            out
        } else {
            replacement.to_string()
        };
        changes.push(Change::replace(
            rope.byte_to_char(m.start()),
            rope.byte_to_char(m.end()),
            text,
        ));
    }
    Ok(Transaction::new(changes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_documents_are_searched_in_windows_without_missing_matches() {
        let line = "a line of filler text for the window test, needle here\n";
        let text = line.repeat(WHOLE_TEXT_BYTES / line.len() + 1000);
        let rope = Rope::from_str(&text);
        assert!(rope.len_bytes() > WHOLE_TEXT_BYTES);
        let hits = find_all(&rope, &SearchQuery::literal("needle")).unwrap();
        assert_eq!(hits.len(), text.len() / line.len());
        assert!(
            hits.windows(2).all(|w| w[0].1 <= w[1].0),
            "ordered, no repeats"
        );
        let (a, b) = hits[hits.len() / 2];
        assert_eq!(rope.slice(a..b).to_string(), "needle");
    }

    #[test]
    fn literal_case_and_whole_word() {
        let rope = Rope::from_str("Foo foo food (foo)");
        let q = SearchQuery::literal("foo");
        assert_eq!(
            find_all(&rope, &q).unwrap(),
            vec![(4, 7), (8, 11), (14, 17)]
        );
        let q = SearchQuery {
            case_sensitive: false,
            whole_word: true,
            ..SearchQuery::literal("foo")
        };
        assert_eq!(find_all(&rope, &q).unwrap(), vec![(0, 3), (4, 7), (14, 17)]);
        let q = SearchQuery::literal("(foo)");
        assert_eq!(
            find_all(&rope, &q).unwrap(),
            vec![(13, 18)],
            "literal, not regex"
        );
    }

    #[test]
    fn matches_are_char_ranges_in_multibyte_text() {
        let rope = Rope::from_str("中文 abc 中文");
        let q = SearchQuery::literal("中文");
        assert_eq!(find_all(&rope, &q).unwrap(), vec![(0, 2), (7, 9)]);
    }

    #[test]
    fn next_and_previous_wrap_around() {
        let rope = Rope::from_str("x a x b x");
        let q = SearchQuery::literal("x");
        assert_eq!(find_next(&rope, &q, 1, true).unwrap(), Some((4, 5)));
        assert_eq!(find_next(&rope, &q, 9, true).unwrap(), Some((0, 1)));
        assert_eq!(find_next(&rope, &q, 4, false).unwrap(), Some((0, 1)));
        assert_eq!(find_next(&rope, &q, 0, false).unwrap(), Some((8, 9)));
    }

    #[test]
    fn regex_replace_expands_groups_and_lines_anchor() {
        let mut rope = Rope::from_str("let a = 1;\nlet b = 2;\n");
        let q = SearchQuery {
            regex: true,
            case_sensitive: true,
            ..SearchQuery::literal(r"^let (\w) = (\d);$")
        };
        let tx = replace_all(&rope, &q, "const $1: i32 = $2;").unwrap();
        assert_eq!(tx.changes().len(), 2);
        tx.apply(&mut rope);
        assert_eq!(rope.to_string(), "const a: i32 = 1;\nconst b: i32 = 2;\n");
    }

    #[test]
    fn bad_and_empty_patterns_are_errors() {
        let rope = Rope::from_str("abc");
        let q = SearchQuery {
            regex: true,
            ..SearchQuery::literal("(")
        };
        assert!(matches!(find_all(&rope, &q), Err(SearchError::Invalid(_))));
        assert_eq!(
            find_all(&rope, &SearchQuery::literal("")),
            Err(SearchError::Empty)
        );
    }
}
