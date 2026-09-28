//! `miao-term-config` — configuration and theming.
//!
//! A small TOML config (`~/.config/miaotty/config.toml`) with sensible defaults
//! matching miaotty's look (Nord). ghostty/alacritty import is a later step.

use std::path::PathBuf;

use serde::Deserialize;

pub mod view;

/// An RGB color parsed from `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse(s: &str) -> Option<Self> {
        let h = s.trim().trim_start_matches('#');
        if h.len() != 6 {
            return None;
        }
        let n = u32::from_str_radix(h, 16).ok()?;
        Some(Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
    }
}

fn default_font_size() -> f32 {
    14.0
}

/// Theme colors.
#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Rgb,
    pub foreground: Rgb,
    pub palette: [Rgb; 16],
}

impl Default for Theme {
    fn default() -> Self {
        // Nord
        let p = [
            "#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0",
            "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4",
        ];
        let mut palette = [Rgb(0, 0, 0); 16];
        for (i, hex) in p.iter().enumerate() {
            palette[i] = Rgb::parse(hex).unwrap();
        }
        Self {
            background: Rgb::parse("#2e3440").unwrap(),
            foreground: Rgb::parse("#d8dee9").unwrap(),
            palette,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawColors {
    background: Option<String>,
    foreground: Option<String>,
    palette: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RawBadges {
    processing: Option<bool>,
    idle: Option<bool>,
    awaiting: Option<bool>,
    error: Option<bool>,
}

/// How the text cursor is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

impl CursorStyle {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "block" => Self::Block,
            "bar" | "beam" => Self::Bar,
            "underline" => Self::Underline,
            _ => return None,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(rename = "font-size")]
    font_size: Option<f32>,
    #[serde(rename = "font-family")]
    font_family: Option<String>,
    #[serde(rename = "line-height")]
    line_height: Option<f32>,
    #[serde(rename = "cursor-style")]
    cursor_style: Option<String>,
    #[serde(rename = "background-opacity")]
    background_opacity: Option<f32>,
    notifications: Option<bool>,
    #[serde(rename = "prevent-sleep")]
    prevent_sleep: Option<bool>,
    badges: Option<RawBadges>,
    language: Option<String>,
    #[serde(rename = "update-check-url")]
    update_check_url: Option<String>,
    editor: Option<String>,
    theme: Option<String>,
    colors: Option<RawColors>,
}

fn theme_from(bg: &str, fg: &str, palette: &[&str; 16]) -> Theme {
    let mut p = [Rgb(0, 0, 0); 16];
    for (i, hex) in palette.iter().enumerate() {
        p[i] = Rgb::parse(hex).unwrap_or(Rgb(0, 0, 0));
    }
    Theme {
        background: Rgb::parse(bg).unwrap_or(Theme::default().background),
        foreground: Rgb::parse(fg).unwrap_or(Theme::default().foreground),
        palette: p,
    }
}

/// A built-in named theme.
pub fn theme_by_name(name: &str) -> Option<Theme> {
    let t = match name.to_ascii_lowercase().as_str() {
        "nord" => Theme::default(),
        "dracula" => theme_from(
            "#282a36",
            "#f8f8f2",
            &[
                "#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd",
                "#f8f8f2", "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df",
                "#a4ffff", "#ffffff",
            ],
        ),
        "gruvbox" | "gruvbox-dark" => theme_from(
            "#282828",
            "#ebdbb2",
            &[
                "#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a",
                "#a89984", "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b",
                "#8ec07c", "#ebdbb2",
            ],
        ),
        "solarized" | "solarized-dark" => theme_from(
            "#002b36",
            "#839496",
            &[
                "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198",
                "#eee8d5", "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4",
                "#93a1a1", "#fdf6e3",
            ],
        ),
        "tokyo-night" | "tokyonight" => theme_from(
            "#1a1b26",
            "#c0caf5",
            &[
                "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff",
                "#a9b1d6", "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7",
                "#7dcfff", "#c0caf5",
            ],
        ),
        _ => return None,
    };
    Some(t)
}

/// Runtime configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub font_size: f32,
    pub font_family: Option<String>,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    pub cursor_style: CursorStyle,
    /// Window/terminal background opacity, 0.0–1.0.
    pub background_opacity: f32,
    /// Post a system notification when an agent needs attention.
    pub notifications: bool,
    /// Keep the machine awake while an agent is processing.
    pub prevent_sleep: bool,
    /// Which agent states show a tab badge.
    pub badges: Badges,
    /// UI language tag (`en`, `zh`, …); `None` detects from `$LANG`.
    pub language: Option<String>,
    /// Optional URL checked for a newer version (see ADR 0013).
    pub update_check_url: Option<String>,
    /// External editor command used by "Edit in Tab" (see ADR 0017).
    pub editor: Option<String>,
    pub theme: Theme,
}

/// Per-state tab badge switches (`settings.agents.badge_*`).
#[derive(Debug, Clone, Copy)]
pub struct Badges {
    pub processing: bool,
    pub idle: bool,
    pub awaiting: bool,
    pub error: bool,
}

impl Default for Badges {
    fn default() -> Self {
        Self {
            processing: true,
            idle: true,
            awaiting: true,
            error: true,
        }
    }
}

impl Badges {
    pub fn enabled(&self, state: &str) -> bool {
        match state {
            "processing" => self.processing,
            "idle" => self.idle,
            "awaiting" => self.awaiting,
            "error" => self.error,
            _ => false,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_size: default_font_size(),
            font_family: None,
            line_height: 1.25,
            cursor_style: CursorStyle::Block,
            background_opacity: 1.0,
            notifications: true,
            prevent_sleep: true,
            badges: Badges::default(),
            language: None,
            update_check_url: None,
            editor: None,
            theme: Theme::default(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct AlacrittyFontNormal {
    family: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlacrittyFont {
    size: Option<f32>,
    normal: Option<AlacrittyFontNormal>,
}

#[derive(Debug, Deserialize)]
struct AlacrittyPrimary {
    background: Option<String>,
    foreground: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlacrittyPalette {
    black: Option<String>,
    red: Option<String>,
    green: Option<String>,
    yellow: Option<String>,
    blue: Option<String>,
    magenta: Option<String>,
    cyan: Option<String>,
    white: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlacrittyColors {
    primary: Option<AlacrittyPrimary>,
    normal: Option<AlacrittyPalette>,
    bright: Option<AlacrittyPalette>,
}

#[derive(Debug, Deserialize)]
struct AlacrittyConfig {
    font: Option<AlacrittyFont>,
    colors: Option<AlacrittyColors>,
}

fn alacritty_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("alacritty").join("alacritty.toml"))
}

fn ghostty_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("ghostty").join("config"))
}

impl Config {
    /// Config file path: `$XDG_CONFIG_HOME/miaotty/config.toml` or `~/.config/miaotty/config.toml`.
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("miaotty").join("config.toml"))
    }

    /// Load from the default path. If there is no miaotty config, fall back to
    /// importing a ghostty config, then to defaults.
    pub fn load() -> Self {
        if let Some(path) = Self::path() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Some(cfg) = Self::from_toml(&text) {
                    return cfg;
                }
            }
        }
        if let Some(ghostty) = ghostty_config_path() {
            if let Ok(text) = std::fs::read_to_string(ghostty) {
                return Self::from_ghostty_text(&text);
            }
        }
        if let Some(alacritty) = alacritty_config_path() {
            if let Ok(text) = std::fs::read_to_string(alacritty) {
                if let Some(cfg) = Self::from_alacritty_text(&text) {
                    return cfg;
                }
            }
        }
        Self::default()
    }

    /// Import the subset of an alacritty TOML config we understand
    /// (`font.size`, `colors.primary`, `colors.normal`/`bright`).
    pub fn from_alacritty_text(text: &str) -> Option<Self> {
        let raw: AlacrittyConfig = toml::from_str(text).ok()?;
        let mut cfg = Self::default();
        if let Some(font) = raw.font {
            if let Some(size) = font.size {
                cfg.font_size = size.clamp(6.0, 40.0);
            }
            if let Some(family) = font.normal.and_then(|n| n.family) {
                if !family.trim().is_empty() {
                    cfg.font_family = Some(family.trim().to_string());
                }
            }
        }
        if let Some(colors) = raw.colors {
            if let Some(primary) = colors.primary {
                if let Some(c) = primary.background.as_deref().and_then(Rgb::parse) {
                    cfg.theme.background = c;
                }
                if let Some(c) = primary.foreground.as_deref().and_then(Rgb::parse) {
                    cfg.theme.foreground = c;
                }
            }
            let apply = |palette: AlacrittyPalette, base: usize, cfg: &mut Config| {
                let slots = [
                    palette.black,
                    palette.red,
                    palette.green,
                    palette.yellow,
                    palette.blue,
                    palette.magenta,
                    palette.cyan,
                    palette.white,
                ];
                for (i, slot) in slots.into_iter().enumerate() {
                    if let Some(c) = slot.as_deref().and_then(Rgb::parse) {
                        cfg.theme.palette[base + i] = c;
                    }
                }
            };
            if let Some(normal) = colors.normal {
                apply(normal, 0, &mut cfg);
            }
            if let Some(bright) = colors.bright {
                apply(bright, 8, &mut cfg);
            }
        }
        Some(cfg)
    }

    /// Import the subset of a ghostty `config` file we understand
    /// (`background`, `foreground`, `font-size`, `palette = N=#hex`).
    pub fn from_ghostty_text(text: &str) -> Self {
        let mut cfg = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();
            match key {
                "background" => {
                    if let Some(c) = Rgb::parse(value) {
                        cfg.theme.background = c;
                    }
                }
                "foreground" => {
                    if let Some(c) = Rgb::parse(value) {
                        cfg.theme.foreground = c;
                    }
                }
                "font-size" => {
                    if let Ok(size) = value.parse::<f32>() {
                        cfg.font_size = size.clamp(6.0, 40.0);
                    }
                }
                "font-family" => {
                    if !value.is_empty() {
                        cfg.font_family = Some(value.to_string());
                    }
                }
                "theme" => {
                    if let Some(theme) = theme_by_name(value) {
                        cfg.theme = theme;
                    }
                }
                "line-height" => {
                    if let Ok(ratio) = value.parse::<f32>() {
                        cfg.line_height = ratio.clamp(0.8, 3.0);
                    }
                }
                "cursor-style" => {
                    if let Some(style) = CursorStyle::parse(value) {
                        cfg.cursor_style = style;
                    }
                }
                "background-opacity" => {
                    if let Ok(opacity) = value.parse::<f32>() {
                        cfg.background_opacity = opacity.clamp(0.1, 1.0);
                    }
                }
                "notifications" => {
                    if let Ok(v) = value.parse::<bool>() {
                        cfg.notifications = v;
                    }
                }
                "prevent-sleep" => {
                    if let Ok(v) = value.parse::<bool>() {
                        cfg.prevent_sleep = v;
                    }
                }
                "palette" => {
                    if let Some((idx, hex)) = value.split_once('=') {
                        if let (Ok(i), Some(c)) = (idx.trim().parse::<usize>(), Rgb::parse(hex)) {
                            if i < 16 {
                                cfg.theme.palette[i] = c;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        cfg
    }

    pub fn from_toml(text: &str) -> Option<Self> {
        let raw: RawConfig = toml::from_str(text).ok()?;
        let mut cfg = Self::default();
        if let Some(size) = raw.font_size {
            cfg.font_size = size.clamp(6.0, 40.0);
        }
        if let Some(family) = raw.font_family {
            let family = family.trim();
            if !family.is_empty() {
                cfg.font_family = Some(family.to_string());
            }
        }
        if let Some(lh) = raw.line_height {
            cfg.line_height = lh.clamp(0.8, 3.0);
        }
        if let Some(style) = raw.cursor_style.as_deref().and_then(CursorStyle::parse) {
            cfg.cursor_style = style;
        }
        if let Some(opacity) = raw.background_opacity {
            cfg.background_opacity = opacity.clamp(0.1, 1.0);
        }
        if let Some(v) = raw.notifications {
            cfg.notifications = v;
        }
        if let Some(v) = raw.prevent_sleep {
            cfg.prevent_sleep = v;
        }
        if let Some(lang) = raw.language {
            let lang = lang.trim();
            if !lang.is_empty() {
                cfg.language = Some(lang.to_string());
            }
        }
        if let Some(url) = raw.update_check_url {
            let url = url.trim();
            if !url.is_empty() {
                cfg.update_check_url = Some(url.to_string());
            }
        }
        if let Some(editor) = raw.editor {
            let editor = editor.trim();
            if !editor.is_empty() {
                cfg.editor = Some(editor.to_string());
            }
        }
        if let Some(b) = raw.badges {
            if let Some(v) = b.processing {
                cfg.badges.processing = v;
            }
            if let Some(v) = b.idle {
                cfg.badges.idle = v;
            }
            if let Some(v) = b.awaiting {
                cfg.badges.awaiting = v;
            }
            if let Some(v) = b.error {
                cfg.badges.error = v;
            }
        }
        if let Some(name) = raw.theme {
            if let Some(theme) = theme_by_name(&name) {
                cfg.theme = theme;
            }
        }
        if let Some(colors) = raw.colors {
            if let Some(bg) = colors.background.as_deref().and_then(Rgb::parse) {
                cfg.theme.background = bg;
            }
            if let Some(fg) = colors.foreground.as_deref().and_then(Rgb::parse) {
                cfg.theme.foreground = fg;
            }
            if let Some(palette) = colors.palette {
                for (i, hex) in palette.iter().take(16).enumerate() {
                    if let Some(c) = Rgb::parse(hex) {
                        cfg.theme.palette[i] = c;
                    }
                }
            }
        }
        Some(cfg)
    }
}

/// Config crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex() {
        assert_eq!(Rgb::parse("#2e3440"), Some(Rgb(0x2e, 0x34, 0x40)));
        assert_eq!(Rgb::parse("2e3440"), Some(Rgb(0x2e, 0x34, 0x40)));
        assert_eq!(Rgb::parse("nope"), None);
    }

    #[test]
    fn defaults_to_nord() {
        let cfg = Config::default();
        assert_eq!(cfg.theme.background, Rgb(0x2e, 0x34, 0x40));
        assert_eq!(cfg.font_size, 14.0);
    }

    #[test]
    fn imports_alacritty() {
        let cfg = Config::from_alacritty_text(
            r##"
            [font]
            size = 15
            [colors.primary]
            background = "#000000"
            foreground = "#cccccc"
            [colors.normal]
            red = "#ff0000"
            [colors.bright]
            red = "#ff5555"
            "##,
        )
        .unwrap();
        assert_eq!(cfg.font_size, 15.0);
        assert_eq!(cfg.theme.background, Rgb(0, 0, 0));
        assert_eq!(cfg.theme.foreground, Rgb(0xcc, 0xcc, 0xcc));
        assert_eq!(cfg.theme.palette[1], Rgb(0xff, 0, 0));
        assert_eq!(cfg.theme.palette[9], Rgb(0xff, 0x55, 0x55));
    }

    #[test]
    fn named_theme_and_font_family() {
        let cfg = Config::from_toml(
            r##"
            theme = "Dracula"
            font-family = "JetBrains Mono"
            "##,
        )
        .unwrap();
        assert_eq!(cfg.theme.background, Rgb(0x28, 0x2a, 0x36));
        assert_eq!(cfg.font_family.as_deref(), Some("JetBrains Mono"));
    }

    #[test]
    fn imports_ghostty() {
        let cfg = Config::from_ghostty_text(
            "# comment\nbackground = #212733\nforeground = #e5e5e5\nfont-size = 16\npalette = 4=#6d95b4\n",
        );
        assert_eq!(cfg.theme.background, Rgb(0x21, 0x27, 0x33));
        assert_eq!(cfg.theme.foreground, Rgb(0xe5, 0xe5, 0xe5));
        assert_eq!(cfg.font_size, 16.0);
        assert_eq!(cfg.theme.palette[4], Rgb(0x6d, 0x95, 0xb4));
    }

    #[test]
    fn overrides_from_toml() {
        let cfg = Config::from_toml(
            r##"
            font-size = 16
            [colors]
            background = "#000000"
            foreground = "#ffffff"
            "##,
        )
        .unwrap();
        assert_eq!(cfg.font_size, 16.0);
        assert_eq!(cfg.theme.background, Rgb(0, 0, 0));
        assert_eq!(cfg.theme.foreground, Rgb(255, 255, 255));
    }
}
