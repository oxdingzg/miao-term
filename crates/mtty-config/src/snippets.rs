//! Command snippets (B3.5) in `~/.config/mtty/snippets.toml`.
//!
//! ```toml
//! [[snippet]]
//! name = "disk usage"
//! command = "df -h"
//! tags = ["ops"]
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Snippet {
    /// Whether every word of `query` appears in the name, command or tags.
    pub fn matches(&self, query: &str) -> bool {
        let hay = format!("{} {} {}", self.name, self.command, self.tags.join(" ")).to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|word| hay.contains(word))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetBook {
    #[serde(default, rename = "snippet")]
    pub snippets: Vec<Snippet>,
}

impl SnippetBook {
    pub fn path() -> Option<PathBuf> {
        Some(crate::config_dir()?.join("snippets.toml"))
    }

    /// A missing file is an empty book; a malformed one is an error, so a
    /// save never overwrites what could not be read.
    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| {
                let first = e.to_string();
                format!(
                    "snippets.toml: {}",
                    first.lines().next().unwrap_or_default()
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no config directory")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    /// Add or replace (by name).
    pub fn upsert(&mut self, snippet: Snippet) {
        match self.snippets.iter_mut().find(|s| s.name == snippet.name) {
            Some(existing) => *existing = snippet,
            None => self.snippets.push(snippet),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_round_trip_match_and_upsert() {
        let mut book = SnippetBook::default();
        book.upsert(Snippet {
            name: "disk usage".into(),
            command: "df -h".into(),
            tags: vec!["ops".into()],
        });
        book.upsert(Snippet {
            name: "disk usage".into(),
            command: "df -hT".into(),
            tags: vec![],
        });
        assert_eq!(book.snippets.len(), 1, "same name replaces");
        let text = toml::to_string_pretty(&book).unwrap();
        assert!(text.contains("[[snippet]]"), "{text}");
        assert_eq!(toml::from_str::<SnippetBook>(&text).unwrap(), book);
        let s = &book.snippets[0];
        assert!(s.matches("disk"));
        assert!(s.matches("DF usage"));
        assert!(!s.matches("memory"));
        assert!(s.matches(""));
    }
}
