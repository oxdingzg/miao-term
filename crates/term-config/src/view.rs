//! View rule engine (ADR 0007).
//!
//! Maps a pane's context — working directory, foreground command, agent, host,
//! file — to an alias, icon, tab title and badge. The rule set is plain JSON at
//! `~/.config/miaotty/views.json`, so it is user-editable and round-trippable.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Rgb;

/// The context a rule is evaluated against. All fields are optional; a matcher
/// only fires when the context supplies the value it tests.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub cwd: Option<String>,
    pub command: Option<String>,
    pub agent: Option<String>,
    pub host: Option<String>,
    pub file: Option<String>,
    pub user: Option<String>,
    pub shell: Option<String>,
    pub branch: Option<String>,
    pub osc_title: Option<String>,
    pub index: Option<usize>,
}

/// The match clauses of a rule. An empty (all-`None`) match is the `any`
/// catch-all.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Match {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl Match {
    /// True when the rule has no clauses and therefore matches anything.
    pub fn is_any(&self) -> bool {
        self.path.is_none()
            && self.command.is_none()
            && self.agent.is_none()
            && self.host.is_none()
            && self.file.is_none()
    }

    fn matches(&self, ctx: &Context) -> bool {
        glob_opt(&self.path, ctx.cwd.as_deref(), true)
            && glob_opt(&self.command, ctx.command.as_deref(), false)
            && exact_opt(&self.agent, ctx.agent.as_deref())
            && glob_opt(&self.host, ctx.host.as_deref(), false)
            && glob_opt(&self.file, ctx.file.as_deref(), true)
    }
}

/// An icon descriptor: a built-in name, an emoji, and/or a color.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Icon {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl Icon {
    /// The color as RGB, if it parses.
    pub fn rgb(&self) -> Option<Rgb> {
        self.color.as_deref().and_then(Rgb::parse)
    }
}

/// One rule: a match plus the appearance it produces.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub r#match: Match,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
}

/// A path → alias mapping for `{alias}` and project grouping.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub path: String,
    pub alias: String,
}

/// The whole rule set. Rules are evaluated in order; the first match wins.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub worktree_suffix: bool,
}

/// The resolved appearance of a pane.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    pub alias: Option<String>,
    pub icon: Option<Icon>,
    pub title: String,
    pub badge: Option<String>,
}

impl RuleSet {
    /// `~/.config/miaotty/views.json` (respecting `XDG_CONFIG_HOME`).
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("miaotty").join("views.json"))
    }

    /// Load from the default path; a missing or malformed file yields an empty
    /// rule set (the plain terminal title), never an error.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| Self::from_json(&t))
            .unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Write to the default path, creating the parent directory.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_json())
    }

    /// Move a rule earlier in the priority order.
    pub fn move_up(&mut self, index: usize) {
        if index > 0 && index < self.rules.len() {
            self.rules.swap(index - 1, index);
        }
    }

    /// Move a rule later in the priority order.
    pub fn move_down(&mut self, index: usize) {
        if index + 1 < self.rules.len() {
            self.rules.swap(index, index + 1);
        }
    }

    /// The alias of the project that owns `cwd` (longest path prefix), with a
    /// worktree suffix when enabled.
    pub fn project_alias(&self, cwd: &str) -> Option<String> {
        let cwd_abs = expand_home(cwd);
        let mut best: Option<(&Project, usize)> = None;
        for project in &self.projects {
            let base = expand_home(&project.path);
            let better = match best {
                None => true,
                Some((_, len)) => base.len() > len,
            };
            if better && is_path_prefix(&cwd_abs, &base) {
                best = Some((project, base.len()));
            }
        }
        let (project, base_len) = best?;
        let mut alias = project.alias.clone();
        if self.worktree_suffix {
            if let Some(suffix) = worktree_suffix(&cwd_abs[base_len..]) {
                alias = format!("{alias}-{suffix}");
            }
        }
        Some(alias)
    }

    /// Evaluate the rule set for a context. Returns the first matching rule's
    /// appearance, the project alias as a fallback `{alias}`, or `None` when
    /// nothing matches and there is no project.
    pub fn evaluate(&self, ctx: &Context) -> Option<Resolved> {
        let rule = self.rules.iter().find(|r| r.r#match.matches(ctx));
        let alias = rule
            .and_then(|r| r.alias.clone())
            .or_else(|| ctx.cwd.as_deref().and_then(|c| self.project_alias(c)));
        if rule.is_none() && alias.is_none() {
            return None;
        }
        let title = rule
            .and_then(|r| r.title.clone())
            .map(|t| render_title(&t, alias.as_deref(), ctx))
            .unwrap_or_else(|| default_title(alias.as_deref(), ctx));
        Some(Resolved {
            alias,
            icon: rule.and_then(|r| r.icon.clone()),
            title,
            badge: rule.and_then(|r| r.badge.clone()),
        })
    }
}

fn default_title(alias: Option<&str>, ctx: &Context) -> String {
    if let Some(alias) = alias {
        return alias.to_string();
    }
    ctx.osc_title.clone().unwrap_or_default()
}

/// Expand the `{...}` variables in a title template.
fn render_title(template: &str, alias: Option<&str>, ctx: &Context) -> String {
    let folder = ctx
        .cwd
        .as_deref()
        .map(folder_of)
        .unwrap_or_default()
        .to_string();
    let cwd = ctx.cwd.as_deref().map(shorten_home).unwrap_or_default();
    let index = ctx.index.map(|i| i.to_string()).unwrap_or_default();
    let file = ctx
        .file
        .as_deref()
        .map(|f| f.rsplit('/').next().unwrap_or(f))
        .unwrap_or_default();
    let osc = ctx.osc_title.as_deref().unwrap_or_default();
    let vars: [(&str, &str); 12] = [
        ("alias", alias.unwrap_or_default()),
        ("cwd", &cwd),
        ("folder", &folder),
        ("user", ctx.user.as_deref().unwrap_or_default()),
        ("host", ctx.host.as_deref().unwrap_or_default()),
        ("agent", ctx.agent.as_deref().unwrap_or_default()),
        ("branch", ctx.branch.as_deref().unwrap_or_default()),
        ("command", ctx.command.as_deref().unwrap_or_default()),
        ("title", osc),
        ("osc_title", osc),
        ("shell", ctx.shell.as_deref().unwrap_or_default()),
        ("index", &index),
    ];
    let mut out = template.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out = out.replace("{file}", file);
    out.trim().to_string()
}

/// The last path component (ignoring a trailing slash).
fn folder_of(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

fn home_dir() -> Option<String> {
    std::env::var("HOME").ok().filter(|h| !h.is_empty())
}

/// Replace a leading `~` with the home directory.
fn expand_home(path: &str) -> String {
    if path == "~" {
        return home_dir().unwrap_or_else(|| "~".into());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}

/// Abbreviate the home directory back to `~` for display.
fn shorten_home(path: &str) -> String {
    if let Some(home) = home_dir() {
        if path == home {
            return "~".into();
        }
        if let Some(rest) = path.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    path.to_string()
}

/// Whether `cwd` is inside `base` on a path-component boundary (equal counts).
fn is_path_prefix(cwd: &str, base: &str) -> bool {
    let base = base.trim_end_matches('/');
    cwd == base || cwd.strip_prefix(base).is_some_and(|r| r.starts_with('/'))
}

/// `.worktrees/<name>/...` → `<name>`.
fn worktree_suffix(rest: &str) -> Option<String> {
    let rest = rest.trim_start_matches('/');
    let mut parts = rest.split('/');
    if parts.next() == Some(".worktrees") {
        return parts.next().filter(|s| !s.is_empty()).map(str::to_string);
    }
    None
}

fn glob_opt(pattern: &Option<String>, value: Option<&str>, expand: bool) -> bool {
    let Some(pattern) = pattern else {
        return true;
    };
    let Some(value) = value else {
        return false;
    };
    let pattern = if expand {
        expand_home(pattern)
    } else {
        pattern.clone()
    };
    let value = if expand {
        expand_home(value)
    } else {
        value.to_string()
    };
    glob_match(&pattern, &value)
}

fn exact_opt(pattern: &Option<String>, value: Option<&str>) -> bool {
    match pattern {
        None => true,
        Some(p) => value.is_some_and(|v| v == p),
    }
}

/// Glob matcher: `*` matches any run within a path segment, `**` crosses `/`,
/// `?` matches one non-`/` character. Case-sensitive.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    glob(pattern.as_bytes(), text.as_bytes())
}

fn glob(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        b'*' if pattern.get(1) == Some(&b'*') => {
            let rest = &pattern[2..];
            (0..=text.len()).any(|i| glob(rest, &text[i..]))
        }
        b'*' => {
            let rest = &pattern[1..];
            for i in 0..=text.len() {
                if i > 0 && text[i - 1] == b'/' {
                    break;
                }
                if glob(rest, &text[i..]) {
                    return true;
                }
            }
            false
        }
        b'?' => !text.is_empty() && text[0] != b'/' && glob(&pattern[1..], &text[1..]),
        c => !text.is_empty() && text[0] == c && glob(&pattern[1..], &text[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(cwd: &str) -> Context {
        Context {
            cwd: Some(cwd.into()),
            ..Default::default()
        }
    }

    #[test]
    fn glob_star_does_not_cross_separator() {
        assert!(glob_match("~/work/*", "~/work/repo"));
        assert!(!glob_match("~/work/*", "~/work/a/repo"));
        assert!(glob_match("~/work/**", "~/work/a/repo"));
        assert!(glob_match("/x/?.rs", "/x/a.rs"));
        assert!(!glob_match("/x/?.rs", "/x/ab.rs"));
    }

    #[test]
    fn first_matching_rule_wins() {
        let set = RuleSet::from_json(
            r#"{"rules":[
                {"match":{"path":"/x/work/**"},"alias":"work","title":"{alias}:{folder}"},
                {"match":{"agent":"claude"},"icon":{"name":"claude"}},
                {"match":{},"title":"{osc_title}"}
            ]}"#,
        )
        .unwrap();
        let mut c = ctx("/x/work/miao");
        c.osc_title = Some("zsh".into());
        let view = set.evaluate(&c).unwrap();
        assert_eq!(view.alias.as_deref(), Some("work"));
        assert_eq!(view.title, "work:miao");
    }

    #[test]
    fn agent_matches_exactly() {
        let set =
            RuleSet::from_json(r#"{"rules":[{"match":{"agent":"claude"},"badge":"ai"}]}"#).unwrap();
        let mut c = ctx("/tmp");
        c.agent = Some("claude".into());
        assert_eq!(set.evaluate(&c).unwrap().badge.as_deref(), Some("ai"));
        c.agent = Some("claude-code".into());
        assert!(set.evaluate(&c).is_none());
    }

    #[test]
    fn any_fallback_renders_variables() {
        let set = RuleSet::from_json(
            r#"{"rules":[{"match":{},"title":"{folder} · {user} · {osc_title}"}]}"#,
        )
        .unwrap();
        let mut c = ctx("/tmp/demo");
        c.user = Some("me".into());
        c.osc_title = Some("vim".into());
        assert_eq!(set.evaluate(&c).unwrap().title, "demo · me · vim");
    }

    #[test]
    fn project_longest_prefix_and_worktree_suffix() {
        let set = RuleSet::from_json(
            r#"{"projects":[
                {"path":"/x/work","alias":"work"},
                {"path":"/x/work/miao","alias":"miao"}
            ],"worktree_suffix":true}"#,
        )
        .unwrap();
        assert_eq!(set.project_alias("/x/work/other").as_deref(), Some("work"));
        assert_eq!(
            set.project_alias("/x/work/miao/src").as_deref(),
            Some("miao")
        );
        assert_eq!(
            set.project_alias("/x/work/miao/.worktrees/feat/x")
                .as_deref(),
            Some("miao-feat")
        );
    }

    #[test]
    fn reorder_changes_priority() {
        let mut set =
            RuleSet::from_json(r#"{"rules":[{"name":"a","match":{}},{"name":"b","match":{}}]}"#)
                .unwrap();
        set.move_up(1);
        assert_eq!(set.rules[0].name.as_deref(), Some("b"));
        set.move_down(0);
        assert_eq!(set.rules[0].name.as_deref(), Some("a"));
    }

    #[test]
    fn json_round_trip_and_malformed_fallback() {
        let json = r#"{"rules":[{"match":{"command":"cargo*"},"icon":{"emoji":"🦀"}}]}"#;
        let set = RuleSet::from_json(json).unwrap();
        let again = RuleSet::from_json(&set.to_json()).unwrap();
        assert_eq!(set, again);
        assert!(RuleSet::from_json("{ not json").is_none());
    }

    #[test]
    fn icon_color_parses() {
        let icon = Icon {
            color: Some("#81a1c1".into()),
            ..Default::default()
        };
        assert_eq!(icon.rgb(), Some(Rgb(0x81, 0xa1, 0xc1)));
    }
}
