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

impl Default for Theme {
    fn default() -> Self {
        Self::nord()
    }
}
