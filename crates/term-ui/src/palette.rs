//! Reusable command-palette scoring.

/// Rank `label`/`kind` against `query`. `None` = no match; lower is better.
pub fn score(label: &str, kind: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
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
    fn substring_then_subsequence() {
        assert_eq!(score("New Tab", "command", ""), Some(0));
        assert_eq!(score("New Tab", "command", "new"), Some(0));
        assert!(score("New Tab", "command", "nt").is_some());
        assert!(score("New Tab", "command", "xyz").is_none());
    }
}
