//! Terminal theme: default colors, indexed color mapping, cursor style.

use miao_term_core::aterm::{Color as TermColor, NamedColor};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}

#[derive(Clone)]
pub struct Theme {
    pub bg: Rgb,
    pub fg: Rgb,
    pub palette: [Rgb; 16],
    pub selection: Rgb,
    pub cursor: CursorStyle,
}

impl Theme {
    pub fn nord() -> Self {
        const P: [Rgb; 16] = [
            Rgb(0x3b, 0x42, 0x52),
            Rgb(0xbf, 0x61, 0x6a),
            Rgb(0xa3, 0xbe, 0x8c),
            Rgb(0xeb, 0xcb, 0x8b),
            Rgb(0x81, 0xa1, 0xc1),
            Rgb(0xb4, 0x8e, 0xad),
            Rgb(0x88, 0xc0, 0xd0),
            Rgb(0xe5, 0xe9, 0xf0),
            Rgb(0x4c, 0x56, 0x6a),
            Rgb(0xbf, 0x61, 0x6a),
            Rgb(0xa3, 0xbe, 0x8c),
            Rgb(0xeb, 0xcb, 0x8b),
            Rgb(0x81, 0xa1, 0xc1),
            Rgb(0xb4, 0x8e, 0xad),
            Rgb(0x8f, 0xbc, 0xbb),
            Rgb(0xec, 0xef, 0xf4),
        ];
        Self {
            bg: Rgb(0x2e, 0x34, 0x40),
            fg: Rgb(0xd8, 0xde, 0xe9),
            palette: P,
            selection: Rgb(0x43, 0x4c, 0x5e),
            cursor: CursorStyle::Block,
        }
    }

    /// Resolve an alacritty color to RGB.
    pub fn color(&self, c: TermColor, foreground: bool) -> Rgb {
        match c {
            TermColor::Spec(s) => Rgb(s.r, s.g, s.b),
            TermColor::Indexed(i) => self.indexed(i),
            TermColor::Named(n) => match n {
                NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => {
                    self.fg
                }
                NamedColor::Background => self.bg,
                NamedColor::DimForeground => Rgb(0x7a, 0x82, 0x8e),
                NamedColor::Black => self.palette[0],
                NamedColor::Red => self.palette[1],
                NamedColor::Green => self.palette[2],
                NamedColor::Yellow => self.palette[3],
                NamedColor::Blue => self.palette[4],
                NamedColor::Magenta => self.palette[5],
                NamedColor::Cyan => self.palette[6],
                NamedColor::White => self.palette[7],
                NamedColor::BrightBlack => self.palette[8],
                NamedColor::BrightRed => self.palette[9],
                NamedColor::BrightGreen => self.palette[10],
                NamedColor::BrightYellow => self.palette[11],
                NamedColor::BrightBlue => self.palette[12],
                NamedColor::BrightMagenta => self.palette[13],
                NamedColor::BrightCyan => self.palette[14],
                NamedColor::BrightWhite => self.palette[15],
                _ => {
                    if foreground {
                        self.fg
                    } else {
                        self.bg
                    }
                }
            },
        }
    }

    pub fn indexed(&self, i: u8) -> Rgb {
        match i {
            0..=15 => self.palette[i as usize],
            16..=231 => {
                let i = i as u16 - 16;
                let f = |x: u16| (x as u8) * 40 + if x > 0 { 55 } else { 0 };
                Rgb(f(i / 36), f((i / 6) % 6), f(i % 6))
            }
            _ => {
                let v = ((i as u16 - 232) * 10 + 8) as u8;
                Rgb(v, v, v)
            }
        }
    }
}

impl Theme {
    pub fn dracula() -> Self {
        const P: [Rgb; 16] = [
            Rgb(0x21, 0x22, 0x2c),
            Rgb(0xff, 0x55, 0x55),
            Rgb(0x50, 0xfa, 0x7b),
            Rgb(0xf1, 0xfa, 0x8c),
            Rgb(0xbd, 0x93, 0xf9),
            Rgb(0xff, 0x79, 0xc6),
            Rgb(0x8b, 0xe9, 0xfd),
            Rgb(0xf8, 0xf8, 0xf2),
            Rgb(0x62, 0x72, 0xa4),
            Rgb(0xff, 0x6e, 0x6e),
            Rgb(0x69, 0xff, 0x94),
            Rgb(0xff, 0xff, 0xa5),
            Rgb(0xd6, 0xac, 0xff),
            Rgb(0xff, 0x92, 0xd0),
            Rgb(0xa4, 0xff, 0xff),
            Rgb(0xff, 0xff, 0xff),
        ];
        Self {
            bg: Rgb(0x28, 0x2a, 0x36),
            fg: Rgb(0xf8, 0xf8, 0xf2),
            palette: P,
            selection: Rgb(0x44, 0x47, 0x5a),
            cursor: CursorStyle::Block,
        }
    }

    pub fn gruvbox() -> Self {
        const P: [Rgb; 16] = [
            Rgb(0x28, 0x28, 0x28),
            Rgb(0xcc, 0x24, 0x1d),
            Rgb(0x98, 0x97, 0x1a),
            Rgb(0xd7, 0x99, 0x21),
            Rgb(0x45, 0x85, 0x88),
            Rgb(0xb1, 0x62, 0x86),
            Rgb(0x68, 0x9d, 0x6a),
            Rgb(0xa8, 0x99, 0x84),
            Rgb(0x92, 0x83, 0x74),
            Rgb(0xfb, 0x49, 0x34),
            Rgb(0xb8, 0xbb, 0x26),
            Rgb(0xfa, 0xbd, 0x2f),
            Rgb(0x83, 0xa5, 0x98),
            Rgb(0xd3, 0x86, 0x9b),
            Rgb(0x8e, 0xc0, 0x7c),
            Rgb(0xeb, 0xdb, 0xb2),
        ];
        Self {
            bg: Rgb(0x28, 0x28, 0x28),
            fg: Rgb(0xeb, 0xdb, 0xb2),
            palette: P,
            selection: Rgb(0x50, 0x49, 0x45),
            cursor: CursorStyle::Block,
        }
    }

    /// Look up a built-in theme by display name.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "Nord" => Some(Self::nord()),
            "Dracula" => Some(Self::dracula()),
            "Gruvbox" => Some(Self::gruvbox()),
            _ => None,
        }
    }

    pub const NAMES: [&'static str; 3] = ["Nord", "Dracula", "Gruvbox"];
}

/// Colours for the surrounding chrome (menu/tabs/sidebar/details/status).
/// A neutral dark palette in the reference app's style, kept separate from the
/// terminal theme so the UI reads cleanly regardless of the terminal colours.
#[derive(Clone, Copy)]
pub struct Chrome {
    pub bg: Rgb,
    pub card: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub hover: Rgb,
    pub active: Rgb,
    pub accent: Rgb,
    /// Left session list, a shade lighter than the terminal card.
    pub sidebar: Rgb,
    /// Right details inspector, a shade darker than the terminal card.
    pub details: Rgb,
}

impl Chrome {
    pub fn dark() -> Self {
        Self {
            bg: Rgb(0x1c, 0x1c, 0x1e),
            card: Rgb(0x23, 0x23, 0x25),
            text: Rgb(0xd1, 0xd1, 0xd1),
            muted: Rgb(0x8a, 0x8a, 0x8a),
            hover: Rgb(0x2c, 0x2c, 0x2e),
            active: Rgb(0x3a, 0x3a, 0x3c),
            accent: Rgb(0x0a, 0x84, 0xff),
            sidebar: Rgb(0x23, 0x23, 0x25),
            details: Rgb(0x14, 0x14, 0x16),
        }
    }
}

impl Theme {
    /// The chrome palette (neutral dark, not tied to the terminal colours).
    pub fn chrome(&self) -> Chrome {
        Chrome::dark()
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::nord()
    }
}

impl Theme {
    /// Build a UI theme from a `term-config` theme (and cursor style), so both
    /// hosts share one mapping.
    pub fn from_config(
        cfg: &miao_term_config::Theme,
        cursor: miao_term_config::CursorStyle,
    ) -> Self {
        let rgb = |c: miao_term_config::Rgb| Rgb(c.0, c.1, c.2);
        Self {
            bg: rgb(cfg.background),
            fg: rgb(cfg.foreground),
            palette: cfg.palette.map(rgb),
            selection: Rgb(0x43, 0x4c, 0x5e),
            cursor: match cursor {
                miao_term_config::CursorStyle::Block => CursorStyle::Block,
                miao_term_config::CursorStyle::Bar => CursorStyle::Bar,
                miao_term_config::CursorStyle::Underline => CursorStyle::Underline,
            },
        }
    }
}
