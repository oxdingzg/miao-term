//! `miao-term-ui` — host-agnostic pieces used by mtty and embedders.
//!
//! Nothing here owns an event loop or a GPU: it is theme, input encoding,
//! selection, row building for the renderer, the split layout model, and the
//! egui chrome widgets. Keeping them here is what lets the chrome be shared
//! instead of duplicated (ADR 0030, step 2/4).

pub mod agentloop;
pub mod chrome;
pub mod forward;
pub mod hints;
pub mod hostkeys;
pub mod hotkey;
pub mod i18n;
pub mod icons;
pub mod input;
pub mod integration;
pub mod launch;
pub mod layout;
pub mod markdown;
pub mod menu;
pub mod mermaid;
pub mod palette;
pub mod selection;
pub mod sftp;
pub mod ssh;
pub mod syntax;
pub mod tasks;
pub mod theme;
pub mod update;
pub mod vim;

use miao_term_core::ATerm;
use miao_term_render::Span;
use theme::{Rgb, Theme};

pub use input::{encode_key, encode_paste, encode_text, EncodeOpts, KeyKind, Modifiers};
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
            let color = if cursor == Some((row, col)) {
                rgb(theme.bg)
            } else if cell.inverse {
                rgb(theme.color(cell.bg, false))
            } else {
                rgb(theme.foreground(cell.fg, cell.bg))
            };
            if width >= 2 {
                if let Some(rc) = run_col.take() {
                    spans.push(Span::new(rc, std::mem::take(&mut run_text), run_color));
                }
                spans.push(Span::new(col, cell.ch.to_string(), color));
                col += 2;
            } else {
                if let Some(rc) = run_col {
                    if run_color == color {
                        run_text.push(cell.ch);
                    } else {
                        spans.push(Span::new(rc, std::mem::take(&mut run_text), run_color));
                        run_col = Some(col);
                        run_color = color;
                        run_text.push(cell.ch);
                    }
                } else {
                    run_col = Some(col);
                    run_color = color;
                    run_text.push(cell.ch);
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

#[cfg(test)]
mod row_tests {
    use super::*;

    #[test]
    fn inverse_video_uses_the_cells_background_for_text() {
        let mut screen = ATerm::new(20, 3, 100);
        screen.process(b"\x1b[31;42;7mX\x1b[0m");
        let theme = Theme::nord();
        let rows = build_rows(&screen, &theme, None);
        assert_eq!(rows[0][0].text, "X");
        assert_eq!(rows[0][0].color, rgb(theme.palette[2]));
    }
}
