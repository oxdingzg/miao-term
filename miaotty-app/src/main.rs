//! miaotty R0 — a usable cross-platform terminal (eframe/egui + vt100 + portable-pty).
//!
//! Bootstrap of the `miaotty` app on the `miao-term` engine. Renders the grid
//! with egui for speed of iteration; R1 replaces this with the custom wgpu
//! glyph-grid renderer (`term-render`) per the architecture.

use eframe::egui;

use miao_term_core::vt100;
use miao_term_core::Terminal;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 640.0])
            .with_title("miaotty"),
        ..Default::default()
    };
    eframe::run_native(
        "miaotty",
        options,
        Box::new(|_cc| Ok(Box::new(MiaottyApp::new()))),
    )
}

/// Nord palette, matching miaotty's default look.
const NORD: [egui::Color32; 16] = [
    egui::Color32::from_rgb(0x3b, 0x42, 0x52),
    egui::Color32::from_rgb(0xbf, 0x61, 0x6a),
    egui::Color32::from_rgb(0xa3, 0xbe, 0x8c),
    egui::Color32::from_rgb(0xeb, 0xcb, 0x8b),
    egui::Color32::from_rgb(0x81, 0xa1, 0xc1),
    egui::Color32::from_rgb(0xb4, 0x8e, 0xad),
    egui::Color32::from_rgb(0x88, 0xc0, 0xd0),
    egui::Color32::from_rgb(0xe5, 0xe9, 0xf0),
    egui::Color32::from_rgb(0x4c, 0x56, 0x6a),
    egui::Color32::from_rgb(0xbf, 0x61, 0x6a),
    egui::Color32::from_rgb(0xa3, 0xbe, 0x8c),
    egui::Color32::from_rgb(0xeb, 0xcb, 0x8b),
    egui::Color32::from_rgb(0x81, 0xa1, 0xc1),
    egui::Color32::from_rgb(0xb4, 0x8e, 0xad),
    egui::Color32::from_rgb(0x8f, 0xbc, 0xbb),
    egui::Color32::from_rgb(0xec, 0xef, 0xf4),
];

const BG: egui::Color32 = egui::Color32::from_rgb(0x2e, 0x34, 0x40);
const FG: egui::Color32 = egui::Color32::from_rgb(0xd8, 0xde, 0xe9);

fn map_color(color: vt100::Color, foreground: bool) -> egui::Color32 {
    match color {
        vt100::Color::Default => {
            if foreground {
                FG
            } else {
                BG
            }
        }
        vt100::Color::Idx(i) => NORD[(i as usize) & 0x0f],
        vt100::Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
    }
}

struct MiaottyApp {
    term: Terminal,
    font_size: f32,
}

impl MiaottyApp {
    fn new() -> Self {
        let term = Terminal::new(None, 100, 30, 10_000).expect("failed to spawn shell");
        Self {
            term,
            font_size: 14.0,
        }
    }
}

impl eframe::App for MiaottyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.term.process_pending() {
            ctx.request_repaint();
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(BG))
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                let font = egui::FontId::monospace(self.font_size);
                let (cw, ch) = ctx.fonts(|f| (f.glyph_width(&font, ' '), f.row_height(&font)));
                if cw <= 0.0 || ch <= 0.0 {
                    return;
                }

                let cols = ((rect.width() / cw).floor() as i64).clamp(1, 1000) as u16;
                let rows = ((rect.height() / ch).floor() as i64).clamp(1, 1000) as u16;
                self.term.resize(rows, cols);

                // Keyboard / paste → PTY.
                let mut out = Vec::new();
                ctx.input(|i| {
                    for ev in &i.events {
                        encode_input(ev, &mut out);
                    }
                });
                if !out.is_empty() {
                    self.term.write(&out);
                    ctx.request_repaint();
                }

                draw_screen(ui, self.term.screen(), &font, cw, ch, rect);
            });
    }
}

fn draw_screen(
    ui: &egui::Ui,
    screen: &vt100::Screen,
    font: &egui::FontId,
    cw: f32,
    ch: f32,
    rect: egui::Rect,
) {
    let painter = ui.painter_at(rect);
    let (rows, cols) = screen.size();
    let ox = rect.left();
    let oy = rect.top();

    for row in 0..rows {
        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };

            let cell_rect = egui::Rect::from_min_size(
                egui::pos2(ox + col as f32 * cw, oy + row as f32 * ch),
                egui::vec2(cw, ch),
            );

            let bg = map_color(cell.bgcolor(), false);
            if bg != BG {
                painter.rect_filled(cell_rect, egui::Rounding::ZERO, bg);
            }

            let contents = cell.contents();
            if !contents.is_empty() && contents != " " {
                let fg = if cell.inverse() {
                    BG
                } else {
                    map_color(cell.fgcolor(), true)
                };
                painter.text(
                    cell_rect.left_top(),
                    egui::Align2::LEFT_TOP,
                    contents,
                    font.clone(),
                    fg,
                );
            }
        }
    }

    // Cursor.
    {
        let (crow, ccol) = screen.cursor_position();
        if crow < rows && ccol < cols {
            let cur_rect = egui::Rect::from_min_size(
                egui::pos2(ox + ccol as f32 * cw, oy + crow as f32 * ch),
                egui::vec2(cw, ch),
            );
            painter.rect_filled(cur_rect, egui::Rounding::ZERO, FG);
            if let Some(cell) = screen.cell(crow, ccol) {
                let c = cell.contents();
                if !c.is_empty() && c != " " {
                    painter.text(
                        cur_rect.left_top(),
                        egui::Align2::LEFT_TOP,
                        c,
                        font.clone(),
                        BG,
                    );
                }
            }
        }
    }
}

fn ctrl_byte(key: egui::Key) -> Option<u8> {
    use egui::Key::*;
    Some(match key {
        A => 0x01, B => 0x02, C => 0x03, D => 0x04, E => 0x05, F => 0x06,
        G => 0x07, H => 0x08, I => 0x09, J => 0x0a, K => 0x0b, L => 0x0c,
        M => 0x0d, N => 0x0e, O => 0x0f, P => 0x10, Q => 0x11, R => 0x12,
        S => 0x13, T => 0x14, U => 0x15, V => 0x16, W => 0x17, X => 0x18,
        Y => 0x19, Z => 0x1a,
        _ => return None,
    })
}

fn encode_input(ev: &egui::Event, out: &mut Vec<u8>) {
    match ev {
        egui::Event::Text(t) => {
            out.extend_from_slice(t.as_bytes());
        }
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => {
            if modifiers.ctrl || modifiers.mac_cmd {
                if let Some(b) = ctrl_byte(*key) {
                    out.push(b);
                    return;
                }
            }
            match key {
                egui::Key::Enter => out.push(b'\r'),
                egui::Key::Backspace => out.push(0x7f),
                egui::Key::Tab => {
                    if modifiers.shift {
                        out.extend_from_slice(b"\x1b[Z");
                    } else {
                        out.push(b'\t');
                    }
                }
                egui::Key::Escape => out.push(0x1b),
                egui::Key::ArrowUp => out.extend_from_slice(b"\x1b[A"),
                egui::Key::ArrowDown => out.extend_from_slice(b"\x1b[B"),
                egui::Key::ArrowRight => out.extend_from_slice(b"\x1b[C"),
                egui::Key::ArrowLeft => out.extend_from_slice(b"\x1b[D"),
                egui::Key::Home => out.extend_from_slice(b"\x1b[H"),
                egui::Key::End => out.extend_from_slice(b"\x1b[F"),
                egui::Key::Delete => out.extend_from_slice(b"\x1b[3~"),
                egui::Key::PageUp => out.extend_from_slice(b"\x1b[5~"),
                egui::Key::PageDown => out.extend_from_slice(b"\x1b[6~"),
                _ => {}
            }
        }
        _ => {}
    }
}
