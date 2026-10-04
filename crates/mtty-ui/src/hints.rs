//! Hint-Mode scanning: find visible URLs / absolute paths and label them.

/// A labelled target found in the visible screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub label: String,
    pub target: String,
    pub row: u16,
    pub col: u16,
    /// A filesystem path (open in the editor) rather than a URL (open externally).
    pub is_path: bool,
}

/// Scan `lines` (viewport rows, top to bottom) for whitespace-delimited tokens
/// that are URLs or absolute paths, labelling up to 26 of them `a`..`z`.
pub fn scan(lines: &[String]) -> Vec<Hint> {
    let mut out = Vec::new();
    let mut labels = ('a'..='z').map(|c| c.to_string());
    'outer: for (row, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_whitespace() {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            let tok: String = chars[start..i].iter().collect();
            let t = tok
                .trim_start_matches(['(', '[', '<', '"', '\''])
                .trim_end_matches([',', '.', ';', ')', ']', '"', '\'']);
            let is_url = t.starts_with("http://") || t.starts_with("https://");
            let is_path = t.starts_with('/') && t.len() > 1;
            if is_url || is_path {
                let Some(label) = labels.next() else {
                    break 'outer;
                };
                out.push(Hint {
                    label,
                    target: t.to_string(),
                    row: row as u16,
                    col: start as u16,
                    is_path,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_urls_and_paths_with_labels() {
        let lines = vec![
            "visit https://example.com/a?b=1 now".to_string(),
            "see /usr/local/bin for tools".to_string(),
            "no links here".to_string(),
        ];
        let hints = scan(&lines);
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0].label, "a");
        assert_eq!(hints[0].target, "https://example.com/a?b=1");
        assert!(!hints[0].is_path);
        assert_eq!(hints[0].row, 0);
        assert_eq!(hints[1].label, "b");
        assert_eq!(hints[1].target, "/usr/local/bin");
        assert!(hints[1].is_path);
    }

    #[test]
    fn trims_trailing_punctuation() {
        let hints = scan(&["(https://x.io).".to_string()]);
        // '(' is not whitespace-delimited? it is part of the token, then trimmed.
        assert_eq!(hints[0].target, "https://x.io");
    }
}
