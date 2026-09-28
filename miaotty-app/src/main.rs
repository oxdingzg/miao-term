//! miaotty — a cross-platform terminal (engine: `miao-term-core`).
//!
//! R0/R1 bootstrap UI on eframe/egui: multi-tab, left sidebar, selection +
//! copy/paste, scrollback, wide-char (CJK) layout, cursor blink. The terminal
//! grid is drawn with egui for iteration speed; R1 replaces this with the
//! custom wgpu renderer (`term-render`) per the architecture.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use unicode_width::UnicodeWidthStr;

use miao_term_core::vt100;
use miao_term_core::Terminal;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 660.0])
            .with_title("miaotty"),
        ..Default::default()
    };
    eframe::run_native(
        "miaotty",
        options,
        Box::new(|cc| {
            install_fonts(&cc.egui_ctx);
            Ok(Box::new(MiaottyApp::new()))
        }),
    )
}

/// Register a system CJK font as a fallback so Chinese/Japanese glyphs render.
fn install_fonts(ctx: &egui::Context) {
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("cjk".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("cjk".to_owned());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
}

/// Nord palette (matches miaotty's default look).
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
const SELECTION: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x81, 0xa1, 0xc1, 0x55);

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

struct Tab {
    term: Terminal,
    title: String,
}

/// A grid selection, in viewport (row, col) coordinates; end may be before start.
#[derive(Clone, Copy)]
struct Selection {
    start: (u16, u16),
    end: (u16, u16),
}

struct MiaottyApp {
    tabs: Vec<Tab>,
    active: usize,
    font_size: f32,
    selection: Option<Selection>,
    scroll: usize,
    cursor_on: bool,
    last_blink: Instant,
}

impl MiaottyApp {
    fn new() -> Self {
        let tab = Tab {
            term: Terminal::new(None, 100, 30, 10_000).expect("failed to spawn shell"),
            title: "shell".to_owned(),
        };
        Self {
            tabs: vec![tab],
            active: 0,
            font_size: 14.0,
            selection: None,
            scroll: 0,
            cursor_on: true,
            last_blink: Instant::now(),
        }
    }

    fn new_tab(&mut self) {
        let (rows, cols) = self.tabs[self.active].term.size();
        match Terminal::new(None, cols, rows, 10_000) {
            Ok(term) => {
                let n = self.tabs.len() + 1;
                self.tabs.push(Tab {
                    term,
                    title: format!("shell {n}"),
                });
                self.active = self.tabs.len() - 1;
                self.selection = None;
                self.scroll = 0;
            }
            Err(e) => eprintln!("failed to spawn shell: {e}"),
        }
    }

    fn close_active(&mut self) {
        if self.tabs.len() <= 1 {
            return;
        }
        self.tabs.remove(self.active);
        self.active = self.active.min(self.tabs.len() - 1);
    }
}

impl eframe::App for MiaottyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain output for every tab (keeps channels from growing unbounded).
        let mut changed = false;
        for tab in &mut self.tabs {
            if tab.term.process_pending() {
                changed = true;
            }
        }
        if changed {
            // New output jumps to the bottom.
            self.scroll = 0;
            ctx.request_repaint();
        }

        // Cursor blink (only when focused; keeps idle CPU low otherwise).
        if ctx.input(|i| i.focused) {
            if self.last_blink.elapsed() >= Duration::from_millis(530) {
                self.cursor_on = !self.cursor_on;
                self.last_blink = Instant::now();
                ctx.request_repaint();
            }
            ctx.request_repaint_after(Duration::from_millis(530));
        }

        self.sidebar(ctx);
        self.terminal_panel(ctx);
    }
}

impl MiaottyApp {
    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tabs")
            .resizable(true)
            .default_width(190.0)
            .frame(egui::Frame::default().fill(BG).inner_margin(egui::Margin::same(6.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("TABS")
                            .size(11.0)
                            .strong()
                            .color(egui::Color32::from_gray(150)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("+").on_hover_text("New Tab").clicked() {
                            self.new_tab();
                        }
                    });
                });
                ui.separator();

                let mut switch_to = None;
                for (i, tab) in self.tabs.iter().enumerate() {
                    let selected = i == self.active;
                    let label = egui::RichText::new(&tab.title).size(13.0).color(FG);
                    let resp = ui.selectable_label(selected, label);
                    if resp.clicked() {
                        switch_to = Some(i);
                    }
                    if resp.clicked_by(egui::PointerButton::Middle) && self.tabs.len() > 1 {
                        // middle-click closes
                        self.active = i;
                        self.close_active();
                        break;
                    }
                }
                if let Some(i) = switch_to {
                    self.active = i;
                    self.selection = None;
                    self.scroll = 0;
                }
            });
    }

    fn terminal_panel(&mut self, ctx: &egui::Context) {
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

                let idx = self.active;
                let modifiers = ctx.input(|i| i.modifiers);
                let cmd = modifiers.command; // ⌘ on mac, Ctrl elsewhere

                // Window-level shortcuts.
                ctx.input(|i| {
                    for ev in &i.events {
                        if let egui::Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } = ev
                        {
                            if modifiers.command {
                                match key {
                                    egui::Key::T => self.new_tab(),
                                    egui::Key::W => self.close_active(),
                                    egui::Key::Plus | egui::Key::Equals => self.font_size += 1.0,
                                    egui::Key::Minus => self.font_size = (self.font_size - 1.0).max(6.0),
                                    egui::Key::Num0 => self.font_size = 14.0,
                                    _ => {}
                                }
                            }
                        }
                    }
                });

                {
                    let tab = &mut self.tabs[idx];
                    tab.term.resize(rows, cols);

                    // ---- scrolling ----
                    let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
                    if scroll_delta != 0.0 {
                        if scroll_delta > 0.0 {
                            self.scroll = (self.scroll + 3).min(200_000);
                        } else {
                            self.scroll = self.scroll.saturating_sub(3);
                        }
                    }
                    tab.term.screen_mut().set_scrollback(self.scroll);

                    // ---- selection ----
                    let response = ui.interact(
                        rect,
                        ui.id().with("terminal"),
                        egui::Sense::click_and_drag(),
                    );
                    let ptr = response.interact_pointer_pos();
                    let cell_at = |p: egui::Pos2| -> (u16, u16) {
                        let col = (((p.x - rect.left()) / cw).floor() as i64)
                            .clamp(0, cols as i64 - 1) as u16;
                        let row = (((p.y - rect.top()) / ch).floor() as i64)
                            .clamp(0, rows as i64 - 1) as u16;
                        (row, col)
                    };
                    if response.drag_started() {
                        if let Some(p) = ptr {
                            let c = cell_at(p);
                            self.selection = Some(Selection { start: c, end: c });
                        }
                    } else if response.dragged() {
                        if let (Some(p), Some(sel)) = (ptr, self.selection.as_mut()) {
                            sel.end = cell_at(p);
                        }
                    } else if response.clicked() {
                        self.selection = None;
                    }

                    // ---- keyboard / paste ----
                    let app_cursor = tab.term.screen().application_cursor();
                    let bracketed = tab.term.screen().bracketed_paste();
                    let mut out = Vec::new();
                    let has_selection = self.selection.is_some();
                    ctx.input(|i| {
                        for ev in &i.events {
                            encode_input(ev, app_cursor, bracketed, has_selection, &mut out);
                        }
                    });
                    if !out.is_empty() {
                        tab.term.write(&out);
                        ctx.request_repaint();
                    }

                    // ---- copy ----
                    if cmd {
                        ctx.input(|i| {
                            for ev in &i.events {
                                if let egui::Event::Key {
                                    key: egui::Key::C,
                                    pressed: true,
                                    modifiers,
                                    ..
                                } = ev
                                {
                                    if modifiers.command {
                                        if let Some(sel) = self.selection {
                                            let (r1, c1, r2, c2) = ordered(sel);
                                            let text = tab
                                                .term
                                                .screen()
                                                .contents_between(r1, c1, r2, c2);
                                            if !text.is_empty() {
                                                ctx.copy_text(text);
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    }

                    // ---- draw ----
                    let screen = tab.term.screen();
                    draw_screen(
                        ui,
                        screen,
                        &font,
                        cw,
                        ch,
                        rect,
                        self.selection,
                        self.scroll == 0 && self.cursor_on && !screen.hide_cursor(),
                    );
                }
            });
    }
}

fn ordered(sel: Selection) -> (u16, u16, u16, u16) {
    let (sr, sc) = sel.start;
    let (er, ec) = sel.end;
    if (er, ec) < (sr, sc) {
        (er, ec, sr, sc)
    } else {
        (sr, sc, er, ec)
    }
}

fn draw_screen(
    ui: &egui::Ui,
    screen: &vt100::Screen,
    font: &egui::FontId,
    cw: f32,
    ch: f32,
    rect: egui::Rect,
    selection: Option<Selection>,
    draw_cursor: bool,
) {
    let painter = ui.painter_at(rect);
    let (rows, cols) = screen.size();
    let ox = rect.left();
    let oy = rect.top();

    // Selection highlight (linear range).
    if let Some(sel) = selection {
        let (r1, c1, r2, c2) = ordered(sel);
        let start = r1 as usize * cols as usize + c1 as usize;
        let end = r2 as usize * cols as usize + c2 as usize;
        for i in start..=end {
            let r = (i / cols as usize) as u16;
            let c = (i % cols as usize) as u16;
            if r >= rows || c >= cols {
                continue;
            }
            let cell_rect = egui::Rect::from_min_size(
                egui::pos2(ox + c as f32 * cw, oy + r as f32 * ch),
                egui::vec2(cw, ch),
            );
            painter.rect_filled(cell_rect, egui::Rounding::ZERO, SELECTION);
        }
    }

    // Cells (wide chars span two columns).
    let mut row = 0;
    while row < rows {
        let mut col = 0;
        while col < cols {
            let Some(cell) = screen.cell(row, col) else {
                col += 1;
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
            let width = UnicodeWidthStr::width(contents).max(if contents.is_empty() { 0 } else { 1 });
            if !contents.is_empty() && contents != " " {
                let fg = if cell.inverse() {
                    BG
                } else {
                    map_color(cell.fgcolor(), true)
                };
                let span = egui::Rect::from_min_size(cell_rect.min, egui::vec2(cw * width as f32, ch));
                painter.text(span.left_top(), egui::Align2::LEFT_TOP, contents, font.clone(), fg);
            }
            col += width.max(1) as u16;
        }
        row += 1;
    }

    // Cursor.
    if draw_cursor {
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
                    painter.text(cur_rect.left_top(), egui::Align2::LEFT_TOP, c, font.clone(), BG);
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

fn encode_input(
    ev: &egui::Event,
    app_cursor: bool,
    bracketed: bool,
    has_selection: bool,
    out: &mut Vec<u8>,
) {
    match ev {
        egui::Event::Text(t) => {
            out.extend_from_slice(t.as_bytes());
        }
        egui::Event::Paste(text) => {
            let mut body = text.replace('\r', "\r").replace('\n', "\r");
            if bracketed {
                out.extend_from_slice(b"\x1b[200~");
                out.extend_from_slice(body.as_bytes());
                out.extend_from_slice(b"\x1b[201~");
            } else {
                out.extend_from_slice(body.as_bytes());
            }
            body.clear();
        }
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => {
            // Ctrl combos (Unix control bytes). On macOS, Cmd is reserved for copy/paste etc.
            if modifiers.ctrl && !modifiers.mac_cmd {
                if let Some(b) = ctrl_byte(*key) {
                    // Ctrl+C with a selection is a copy (handled by the caller).
                    if *key == egui::Key::C && has_selection {
                        return;
                    }
                    out.push(b);
                    return;
                }
            }
            let mut arrow = |c: u8| {
                if app_cursor {
                    out.extend_from_slice(&[0x1b, b'O', c]);
                } else {
                    out.extend_from_slice(&[0x1b, b'[', c]);
                }
            };
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
                egui::Key::ArrowUp => arrow(b'A'),
                egui::Key::ArrowDown => arrow(b'B'),
                egui::Key::ArrowRight => arrow(b'C'),
                egui::Key::ArrowLeft => arrow(b'D'),
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
