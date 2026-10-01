//! Reusable command-palette scoring.

/// Rank `label`/`kind` against `query`. `None` = no match; lower is better.
pub fn score(label: &str, kind: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    // Most palette entries are ASCII. Search the virtual "label kind" string
    // without allocating either the joined string or its lowercase copy.
    if label.is_ascii() && kind.is_ascii() && query.is_ascii() {
        let bytes = || {
            label
                .bytes()
                .chain(std::iter::once(b' '))
                .chain(kind.bytes())
                .map(|b| b.to_ascii_lowercase())
        };
        let first = query.as_bytes()[0];
        let mut hay = bytes();
        let mut pos = 0;
        while let Some(b) = hay.next() {
            if b == first && hay.clone().take(query.len() - 1).eq(query.bytes().skip(1)) {
                return Some(pos);
            }
            pos += 1;
        }
        let mut hay = bytes();
        return query.bytes().all(|c| hay.any(|h| h == c)).then_some(1000);
    }
    let hay = format!("{label} {kind}").to_lowercase();
    if let Some(pos) = hay.find(query) {
        return Some(pos);
    }
    let mut chars = hay.chars();
    for c in query.chars() {
        if !chars.any(|h| h == c) {
            return None;
        }
    }
    Some(1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring_preserves_joined_text_offsets_and_unicode() {
        // Include boundary-spanning substrings, subsequences, repeated prefixes,
        // mixed case, and Unicode lowercase expansion.
        for (label, kind) in [
            ("New Tab", "command"),
            ("aaaaab", "file"),
            ("", "FILE"),
            ("目录 İ", "文件"),
        ] {
            for query in [
                "", "tab c", "b f", "file", "nf", "aaaaac", "N", "目录", "i\u{307}",
            ] {
                let hay = format!("{label} {kind}").to_lowercase();
                let expected = hay.find(query).or_else(|| {
                    let mut chars = hay.chars();
                    query.chars().all(|c| chars.any(|h| h == c)).then_some(1000)
                });
                assert_eq!(
                    score(label, kind, query),
                    expected,
                    "{label:?} {kind:?} {query:?}"
                );
            }
        }
    }

    #[test]
    fn substring_then_subsequence() {
        assert_eq!(score("New Tab", "command", ""), Some(0));
        assert_eq!(score("New Tab", "command", "new"), Some(0));
        assert!(score("New Tab", "command", "nt").is_some());
        assert!(score("New Tab", "command", "xyz").is_none());
    }
}
