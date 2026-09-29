//! `miao-term-ui` — host-agnostic pieces shared by both miaotty hosts (the
//! eframe app and the native `winit`+`wgpu` host).
//!
//! Nothing here owns an event loop or a GPU: it is theme, input encoding,
//! selection, row building for the renderer, the split layout model, and the
//! egui chrome widgets. Keeping them here is what lets the chrome be shared
//! instead of duplicated (ADR 0030, step 2/4).

pub mod chrome;
pub mod i18n;
pub mod input;
pub mod layout;
pub mod selection;
pub mod theme;

use miao_term_core::ATerm;
use miao_term_render::Span;
use theme::{Rgb, Theme};

pub use input::{encode, KeyInput, KeyKind, Modifiers};
pub use layout::{Layout, Rect, SplitDir};
pub use selection::Selection;
pub use theme::{CursorStyle, Theme as UiTheme};

/// Build per-row, cell-pinned spans for the grid renderer (wide chars get their
/// own span so every glyph lands on an exact cell).
pub fn build_rows(screen: &ATerm, theme: &Theme, cursor: Option<(u16, u16)>) -> Vec<Vec<Span>> {
    let (rows, cols) = screen.size();
    let mut out = Vec::with_capacity(rows as usize);
    for row in 0..rows {
        let mut spans: Vec<Span> = Vec::new();
        let mut run_col: Option<u16> = None;
        let mut run_color = (0u8, 0u8, 0u8);
        let mut run_text = String::new();
        let mut col = 0u16;
        while col < cols {
            let Some(cell) = screen.cell(row, col) else {
                col += 1;
                continue;
            };
            let width = width_of(cell.ch);
            let mut buf = [0u8; 4];
            let text = cell.ch.encode_utf8(&mut buf).to_string();
            let color = if cell.inverse || cursor == Some((row, col)) {
                rgb(theme.bg)
            } else {
                rgb(theme.color(cell.fg, true))
            };
            if width >= 2 {
                if let Some(rc) = run_col.take() {
                    spans.push(Span::new(rc, std::mem::take(&mut run_text), run_color));
                }
                spans.push(Span::new(col, text, color));
                col += 2;
            } else {
                if let Some(rc) = run_col {
                    if run_color == color {
                        run_text.push_str(&text);
                    } else {
                        spans.push(Span::new(rc, std::mem::take(&mut run_text), run_color));
                        run_col = Some(col);
                        run_color = color;
                        run_text = text;
                    }
                } else {
                    run_col = Some(col);
                    run_color = color;
                    run_text = text;
                }
                col += 1;
            }
        }
        if let Some(rc) = run_col.take() {
            spans.push(Span::new(rc, run_text, run_color));
        }
        out.push(spans);
    }
    out
}

pub fn rgb(c: Rgb) -> (u8, u8, u8) {
    (c.0, c.1, c.2)
}

/// Width in terminal cells (2 for wide CJK/emoji, else 1).
pub fn width_of(c: char) -> u16 {
    let u = c as u32;
    if u >= 0x1100
        && (u <= 0x115f
            || (0x2e80..=0xa4cf).contains(&u)
            || (0xac00..=0xd7a3).contains(&u)
            || (0xf900..=0xfaff).contains(&u)
            || (0xfe30..=0xfe4f).contains(&u)
            || (0xff00..=0xff60).contains(&u)
            || (0xffe0..=0xffe6).contains(&u)
            || (0x1f300..=0x1faff).contains(&u)
            || (0x20000..=0x3fffd).contains(&u))
    {
        2
    } else {
        1
    }
}
