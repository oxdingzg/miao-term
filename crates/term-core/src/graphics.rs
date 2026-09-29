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
    /// Pixel offsets from the cursor cell (Kitty `X`/`Y`).
    pub x_off: i32,
    pub y_off: i32,
    pub image: Arc<gfx::Image>,
    pub z: i32,
    /// Kitty image id this was placed as (for frames / animation / delete).
    pub kitty_id: Option<u64>,
    /// Animation frames (frame 0 is `image`); empty/1 for a still image.
    pub frames: Vec<Arc<gfx::Image>>,
    pub animating: bool,
    pub anim_start: std::time::Instant,
    pub loops: Option<u32>,
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
    x: i32,
    y: i32,
    action: char,
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

    #[allow(clippy::too_many_arguments)]
    pub fn place(
        &mut self,
        image: gfx::Image,
        anchor: i32,
        col: u16,
        cols: Option<u16>,
        rows: Option<u16>,
        x_off: i32,
        y_off: i32,
        z: i32,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let image = Arc::new(image);
        self.images.push(PlacedImage {
            id,
            anchor,
            col,
            cols,
            rows,
            x_off,
            y_off,
            image: image.clone(),
            z,
            kitty_id: None,
            frames: vec![image],
            animating: false,
            anim_start: std::time::Instant::now(),
            loops: None,
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

    /// Shift every anchor up by `lines` (content scrolled past the ring cap,
    /// where `total_lines()` no longer grows).
    pub fn shift(&mut self, lines: usize) {
        let d = lines.min(i32::MAX as usize) as i32;
        for img in &mut self.images {
            img.anchor -= d;
        }
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

    /// Handle a Kitty command. `anchor`/`col` are the current cursor position,
    /// `view_offset` the scrollback offset (so viewport rows can be computed).
    /// Returns true if the visible content changed.
    pub fn kitty(&mut self, cmd: gfx::KittyCmd, anchor: i32, col: u16, view_offset: i32) -> bool {
        let key = cmd.id.unwrap_or(0);
        // Animation control: start/stop; no payload.
        if cmd.action == 'a' {
            if let Some(pl) = self.images.iter_mut().find(|i| i.kitty_id == cmd.id) {
                let s = cmd.state.unwrap_or(1);
                pl.animating = s == 1;
                pl.loops = cmd.loops;
                pl.anim_start = std::time::Instant::now();
            }
            return true;
        }
        if cmd.action == 'd' {
            let row_of = |im: &PlacedImage| im.anchor + view_offset;
            match cmd
                .delete
                .unwrap_or(if cmd.id.is_some() { 'i' } else { 'a' })
            {
                'i' => match cmd.id {
                    Some(id) => self.delete_id(id),
                    None => self.images.clear(),
                },
                'p' => {
                    let (cx, cy) = (cmd.cell_x.unwrap_or(0), cmd.cell_y.unwrap_or(0));
                    self.images
                        .retain(|im| !(im.col == cx && row_of(im) == cy as i32));
                }
                'c' => {
                    let cx = cmd.cell_x.unwrap_or(0);
                    self.images.retain(|im| im.col != cx);
                }
                'r' => {
                    let cy = cmd.cell_y.unwrap_or(0) as i32;
                    self.images.retain(|im| row_of(im) != cy);
                }
                'z' => self.images.retain(|im| im.z != cmd.z),
                _ => self.images.clear(),
            }
            return true;
        }
        // Accumulate chunks (`m=1` means more follow). A new action restarts.
        let p = self.partial.entry(key).or_default();
        if cmd.more || p.data.is_empty() || p.action != cmd.action {
            p.data.clear();
            p.action = cmd.action;
            p.id = cmd.id;
            p.cols = cmd.cols.or(p.cols);
            p.rows = cmd.rows.or(p.rows);
            p.move_cursor = cmd.move_cursor;
            p.z = cmd.z;
            p.format = cmd.format;
            p.compressed = cmd.compressed;
            p.size = cmd.size.or(p.size);
            p.x = cmd.x;
            p.y = cmd.y;
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
        if !matches!(cmd.action, 'T' | 'p' | 'f') {
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
                if cmd.action == 'f' {
                    // Append a frame to the image with this kitty id.
                    if let Some(pl) = self.images.iter_mut().find(|i| i.kitty_id == cmd.id) {
                        pl.frames.push(Arc::new(img));
                        pl.anim_start = std::time::Instant::now();
                    }
                } else {
                    let our = self.place(img, anchor, col, p.cols, p.rows, p.x, p.y, p.z);
                    if let Some(pl) = self.images.iter_mut().find(|i| i.id == our) {
                        pl.kitty_id = cmd.id;
                    }
                }
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
        l.place(img(2, 2), 5, 0, None, None, 0, 0, 0);
        l.sync_total(10);
        assert_eq!(l.images[0].anchor, 5);
        l.sync_total(13); // three new lines
        assert_eq!(l.images[0].anchor, 2);
    }

    #[test]
    fn kitty_frames_and_animation() {
        let mut l = GraphicsLayer::new();
        l.kitty(gfx::kitty::parse(b"a=T,f=24,s=1x1,i=1;AAAA"), 0, 0, 0);
        assert_eq!(l.images[0].frames.len(), 1);
        l.kitty(gfx::kitty::parse(b"a=f,f=24,s=1x1,i=1;AAAA"), 0, 0, 0);
        assert_eq!(l.images[0].frames.len(), 2);
        l.kitty(gfx::kitty::parse(b"a=a,i=1,s=1,c=2"), 0, 0, 0);
        assert!(l.images[0].animating);
        assert_eq!(l.images[0].loops, Some(2));
        l.kitty(gfx::kitty::parse(b"a=a,i=1,s=2"), 0, 0, 0);
        assert!(!l.images[0].animating);
    }

    #[test]
    fn kitty_chunks_reassemble() {
        let mut l = GraphicsLayer::new();
        // 1x1 raw RGB (base64 "AAAA"), sent as two `m=` chunks.
        let c1 = gfx::kitty::parse(b"a=T,f=24,s=1x1,m=1;AA");
        let c2 = gfx::kitty::parse(b"a=T,f=24,s=1x1,m=0;AA");
        assert!(!l.kitty(c1, 0, 0, 0), "first chunk is held");
        assert!(l.kitty(c2, 0, 0, 0), "second chunk decodes");
        assert_eq!(l.images.len(), 1);
        assert_eq!((l.images[0].image.width, l.images[0].image.height), (1, 1));
    }

    #[test]
    fn region_deletes() {
        let mut l = GraphicsLayer::new();
        l.place(img(2, 2), 3, 1, None, None, 0, 0, 0); // column 1, line 3
        l.place(img(2, 2), 5, 2, None, None, 0, 0, 0); // column 2, line 5
                                                       // d=p at (col 1, row 3) deletes only the first.
        let mut cmd = gfx::kitty::parse(b"a=d,d=p,x=1,y=3");
        cmd.id = None;
        l.kitty(cmd, 0, 0, 0);
        assert_eq!(l.images.len(), 1);
        assert_eq!(l.images[0].col, 2);
        // d=c at column 2 removes the rest.
        let mut cmd = gfx::kitty::parse(b"a=d,d=c,x=2");
        cmd.id = None;
        l.kitty(cmd, 0, 0, 0);
        assert!(l.images.is_empty());
    }

    #[test]
    fn shift_moves_all_anchors() {
        let mut l = GraphicsLayer::new();
        l.place(img(1, 1), 3, 0, None, None, 0, 0, 0);
        l.shift(5);
        assert_eq!(l.images[0].anchor, -2);
    }

    #[test]
    fn delete_by_id_and_all() {
        let mut l = GraphicsLayer::new();
        let a = l.place(img(1, 1), 0, 0, None, None, 0, 0, 0);
        l.place(img(1, 1), 0, 0, None, None, 0, 0, 0);
        l.delete_id(a);
        assert_eq!(l.images.len(), 1);
        l.images.clear();
        assert!(l.images.is_empty());
    }
}
