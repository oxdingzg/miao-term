//! `miao-term-config` — configuration and theming.
//!
//! A small TOML config (`~/.config/miaotty/config.toml`) with sensible defaults
//! matching miaotty's look (Nord). ghostty/alacritty import is a later step.

use std::path::PathBuf;

use serde::Deserialize;

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
            "#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0",
            "#e5e9f0", "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead",
            "#8fbcbb", "#eceff4",
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
struct RawConfig {
    #[serde(rename = "font-size")]
    font_size: Option<f32>,
    colors: Option<RawColors>,
}

/// Runtime configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub font_size: f32,
    pub theme: Theme,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_size: default_font_size(),
            theme: Theme::default(),
        }
    }
}

impl Config {
    /// Config file path: `$XDG_CONFIG_HOME/miaotty/config.toml` or `~/.config/miaotty/config.toml`.
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("miaotty").join("config.toml"))
    }

    /// Load from the default path; falls back to defaults on any error.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        Self::from_toml(&text).unwrap_or_default()
    }

    pub fn from_toml(text: &str) -> Option<Self> {
        let raw: RawConfig = toml::from_str(text).ok()?;
        let mut cfg = Self::default();
        if let Some(size) = raw.font_size {
            cfg.font_size = size.clamp(6.0, 40.0);
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
