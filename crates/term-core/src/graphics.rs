//! Per-terminal inline-graphics layer: decoded images anchored to buffer lines.
//!
//! Images are not part of the alacritty grid, so we keep a side table. Each
//! image is anchored to an absolute `Line` (the same coordinate `ATerm` uses),
//! and every time the buffer grows we shift anchors so images scroll with the
//! text. (Beyond the scrollback cap the grid rotates without growing, so
//! anchoring is best-effort there.)

use std::collections::HashMap;
use std::sync::Arc;

use miao_term_graphics as gfx;

/// Decoded-pixel cap per image (also bounds total memory per image).
pub const DEFAULT_MAX_PIXELS: usize = 16_000_000;

#[derive(Clone)]
pub struct PlacedImage {
    pub id: u64,
    /// Absolute buffer line (`Line.0`) where the image's top-left sits.
    pub anchor: i32,
    pub col: u16,
    /// Explicit cell footprint (Kitty `c=`/`r=`); the host derives one when
    /// this is `None`.
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub image: Arc<gfx::Image>,
    pub z: i32,
}

#[derive(Default)]
struct Partial {
    id: Option<u64>,
    cols: Option<u16>,
    rows: Option<u16>,
    move_cursor: bool,
    z: i32,
    format: u32,
    compressed: bool,
    size: Option<(u32, u32)>,
    data: Vec<u8>,
}

/// The graphics state for one terminal.
#[derive(Default)]
pub struct GraphicsLayer {
    pub images: Vec<PlacedImage>,
    next_id: u64,
    partial: HashMap<u64, Partial>,
    pub max_pixels: usize,
    last_total: usize,
}

impl GraphicsLayer {
    pub fn new() -> Self {
        Self {
            max_pixels: DEFAULT_MAX_PIXELS,
            ..Default::default()
        }
    }

    pub fn place(
        &mut self,
        image: gfx::Image,
        anchor: i32,
        col: u16,
        cols: Option<u16>,
        rows: Option<u16>,
        z: i32,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.images.push(PlacedImage {
            id,
            anchor,
            col,
            cols,
            rows,
            image: Arc::new(image),
            z,
        });
        // Keep the most recent images only, to bound memory.
        if self.images.len() > 256 {
            let drop = self.images.len() - 256;
            self.images.drain(..drop);
        }
        id
    }

    pub fn delete_id(&mut self, id: u64) {
        self.images.retain(|i| i.id != id);
    }

    pub fn clear(&mut self) {
        self.images.clear();
        self.partial.clear();
    }

    /// Keep anchors aligned with buffer growth (call with `total_lines()`).
    pub fn sync_total(&mut self, total: usize) {
        let d = total as i64 - self.last_total as i64;
        if d > 0 {
            let d = d.min(i32::MAX as i64) as i32;
            for img in &mut self.images {
                img.anchor -= d;
            }
        }
        self.last_total = total;
    }

    /// Handle a Kitty command. `anchor`/`col` are the current cursor position.
    /// Returns true if the visible content changed.
    pub fn kitty(&mut self, cmd: gfx::KittyCmd, anchor: i32, col: u16) -> bool {
        let key = cmd.id.unwrap_or(0);
        if cmd.action == 'd' {
            match cmd.id {
                Some(id) => self.delete_id(id),
                None => self.images.clear(),
            }
            return true;
        }
        // Accumulate chunks (`m=1` means more follow).
        let p = self.partial.entry(key).or_default();
        if cmd.more || p.data.is_empty() {
            p.id = cmd.id;
            p.cols = cmd.cols.or(p.cols);
            p.rows = cmd.rows.or(p.rows);
            p.move_cursor = cmd.move_cursor;
            p.z = cmd.z;
            p.format = cmd.format;
            p.compressed = cmd.compressed;
            p.size = cmd.size.or(p.size);
        }
        p.data.extend_from_slice(&cmd.data);
        if cmd.more {
            // Bound a partial in case the sender never finishes.
            if p.data.len() > gfx::MAX_SEQUENCE {
                self.partial.remove(&key);
            }
            return false;
        }
        let Some(p) = self.partial.remove(&key) else {
            return false;
        };
        if !matches!(cmd.action, 'T' | 'p') {
            return false; // transfer-only / query: nothing to show
        }
        let full = gfx::KittyCmd {
            data: p.data,
            format: p.format,
            compressed: p.compressed,
            size: p.size,
            ..cmd
        };
        match gfx::kitty::decode(&full, self.max_pixels) {
            Some(img) => {
                self.place(img, anchor, col, p.cols, p.rows, p.z);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32) -> gfx::Image {
        gfx::Image {
            width: w,
            height: h,
            rgba: vec![255u8; (w * h * 4) as usize],
        }
    }

    #[test]
    fn anchors_shift_with_buffer_growth() {
        let mut l = GraphicsLayer::new();
        l.sync_total(10); // last_total aligns with the buffer at placement
        l.place(img(2, 2), 5, 0, None, None, 0);
        l.sync_total(10);
        assert_eq!(l.images[0].anchor, 5);
        l.sync_total(13); // three new lines
        assert_eq!(l.images[0].anchor, 2);
    }

    #[test]
    fn delete_by_id_and_all() {
        let mut l = GraphicsLayer::new();
        let a = l.place(img(1, 1), 0, 0, None, None, 0);
        l.place(img(1, 1), 0, 0, None, None, 0);
        l.delete_id(a);
        assert_eq!(l.images.len(), 1);
        l.images.clear();
        assert!(l.images.is_empty());
    }
}
