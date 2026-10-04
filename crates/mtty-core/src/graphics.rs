//! Per-terminal inline-graphics layer: decoded images anchored to buffer lines.
//!
//! Images are not part of the alacritty grid, so we keep a side table. Each
//! image is anchored to an absolute `Line` (the same coordinate `ATerm` uses),
//! and every time the buffer grows we shift anchors so images scroll with the
//! text. (Beyond the scrollback cap the grid rotates without growing, so
//! anchoring is best-effort there.)

use std::collections::HashMap;
use std::sync::Arc;

use mtty_graphics as gfx;

/// Decoded-pixel cap per image (also bounds total memory per image).
pub const DEFAULT_MAX_PIXELS: usize = 16_000_000;
/// Total retained decoded RGBA bytes, including animation frames, per terminal.
pub const DEFAULT_MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;
const MAX_PARTIAL_TRANSFERS: usize = 16;
const MAX_ANIMATION_FRAMES: usize = 256;

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
pub struct GraphicsLayer {
    pub images: Vec<PlacedImage>,
    next_id: u64,
    partial: HashMap<u64, Partial>,
    pub max_pixels: usize,
    pub max_image_bytes: usize,
    pub max_partial_bytes: usize,
    last_total: usize,
}

impl Default for GraphicsLayer {
    fn default() -> Self {
        Self {
            images: Vec::new(),
            next_id: 0,
            partial: HashMap::new(),
            max_pixels: DEFAULT_MAX_PIXELS,
            max_image_bytes: DEFAULT_MAX_IMAGE_BYTES,
            max_partial_bytes: gfx::MAX_SEQUENCE,
            last_total: 0,
        }
    }
}

impl GraphicsLayer {
    pub fn new() -> Self {
        Self::default()
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
        self.trim_images();
        id
    }

    /// Retained pixel-buffer capacity (frame 0 and `image` share one Arc).
    pub fn decoded_bytes(&self) -> usize {
        self.images.iter().map(placed_bytes).sum()
    }

    /// Allocated capacity of incomplete Kitty transfers, across all ids.
    pub fn pending_bytes(&self) -> usize {
        self.partial.values().map(|p| p.data.capacity()).sum()
    }

    fn trim_images(&mut self) {
        let mut bytes = self.decoded_bytes();
        let mut drop = 0;
        while self.images.len() - drop > 256 || bytes > self.max_image_bytes {
            bytes -= placed_bytes(&self.images[drop]);
            drop += 1;
        }
        // Evict whole oldest placements: dropping individual animation frames
        // would silently renumber the protocol's frame sequence.
        self.images.drain(..drop);
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
        if cmd.action == 'f'
            && self
                .images
                .iter()
                .any(|image| image.kitty_id == cmd.id && image.frames.len() >= MAX_ANIMATION_FRAMES)
        {
            self.partial.remove(&key);
            return false;
        }
        // Bound the aggregate capacity, not just one sender's payload. Many
        // image ids and animation frames must not multiply a per-image cap.
        if !self.partial.contains_key(&key) && self.partial.len() >= MAX_PARTIAL_TRANSFERS {
            return false;
        }
        let other_bytes: usize = self
            .partial
            .iter()
            .filter(|(id, _)| **id != key)
            .map(|(_, p)| p.data.capacity())
            .sum();
        let limit = self.max_partial_bytes.saturating_sub(other_bytes);
        let p = self.partial.entry(key).or_default();
        if p.data.is_empty() || p.action != cmd.action {
            if p.action != cmd.action {
                p.data = Vec::new();
            }
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
        let needed = p.data.len().saturating_add(cmd.data.len());
        if needed > limit {
            self.partial.remove(&key);
            return false;
        }
        if needed > p.data.capacity() {
            let capacity = needed.max(p.data.capacity().saturating_mul(2)).min(limit);
            p.data.reserve_exact(capacity - p.data.len());
        }
        p.data.extend_from_slice(&cmd.data);
        if cmd.more {
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
                    self.trim_images();
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

fn placed_bytes(image: &PlacedImage) -> usize {
    if image.frames.is_empty() {
        image.image.rgba.capacity()
    } else {
        image.frames.iter().map(|frame| frame.rgba.capacity()).sum()
    }
}

/// Which side of the text grid a Kitty placement draws on.
///
/// Kitty's `z=` gives the vertical stacking order: a negative z-index places the
/// image behind the text (so glyphs render on top of it), while the default
/// layer (`z=0`) and positive values place it above the text.
pub fn draws_behind_text(z: i32) -> bool {
    z < 0
}

/// Stable cache key for one rendered image: pane + image id + animation frame.
pub fn image_key(pane: &str, id: u64, frame: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    pane.hash(&mut h);
    id.hash(&mut h);
    frame.hash(&mut h);
    h.finish()
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
    fn decoded_budget_evicts_oldest_placements_and_counts_frames_once() {
        let mut layer = GraphicsLayer::new();
        layer.max_image_bytes = 12;
        let oldest = layer.place(img(1, 1), 0, 0, None, None, 0, 0, 0);
        let released = Arc::downgrade(&layer.images[0].image);
        layer.kitty(gfx::kitty::parse(b"a=T,f=24,s=1x1,i=9;AAAA"), 0, 0, 0);
        layer.kitty(gfx::kitty::parse(b"a=f,f=24,s=1x1,i=9;AAAA"), 0, 0, 0);
        assert_eq!(
            layer.decoded_bytes(),
            12,
            "frame 0 shares the image allocation"
        );
        layer.place(img(1, 1), 0, 0, None, None, 0, 0, 0);
        assert_eq!(layer.decoded_bytes(), 12);
        assert!(layer.images.iter().all(|im| im.id != oldest));
        assert!(
            released.upgrade().is_none(),
            "eviction must actually release pixels"
        );
        assert_eq!(
            layer.images[0].frames.len(),
            2,
            "do not renumber retained frames"
        );
        layer.clear();
        assert_eq!(layer.decoded_bytes(), 0);
    }

    #[test]
    fn decoded_budget_counts_unused_capacity_and_frame_metadata_is_bounded() {
        let mut layer = GraphicsLayer::new();
        layer.max_image_bytes = 12;
        let mut pixels = Vec::with_capacity(1024);
        pixels.extend_from_slice(&[0, 0, 0, 255]);
        layer.place(
            gfx::Image {
                width: 1,
                height: 1,
                rgba: pixels,
            },
            0,
            0,
            None,
            None,
            0,
            0,
            0,
        );
        assert!(
            layer.images.is_empty(),
            "unused pixel capacity must count toward the budget"
        );
        layer.max_image_bytes = DEFAULT_MAX_IMAGE_BYTES;
        layer.kitty(gfx::kitty::parse(b"a=T,f=24,s=1x1,i=1;AAAA"), 0, 0, 0);
        for _ in 0..MAX_ANIMATION_FRAMES + 10 {
            layer.kitty(gfx::kitty::parse(b"a=f,f=24,s=1x1,i=1;AAAA"), 0, 0, 0);
        }
        assert_eq!(layer.images[0].frames.len(), MAX_ANIMATION_FRAMES);
        assert_eq!(layer.pending_bytes(), 0);
    }

    #[test]
    fn one_animation_cannot_bypass_the_total_decoded_budget() {
        let mut layer = GraphicsLayer::new();
        layer.max_image_bytes = 12;
        layer.kitty(gfx::kitty::parse(b"a=T,f=24,s=1x1,i=1;AAAA"), 0, 0, 0);
        for _ in 0..3 {
            layer.kitty(gfx::kitty::parse(b"a=f,f=24,s=1x1,i=1;AAAA"), 0, 0, 0);
            assert!(layer.decoded_bytes() <= 12);
        }
        assert!(
            layer.images.is_empty(),
            "an oversized animation is evicted whole"
        );
    }

    #[test]
    fn partial_budget_covers_all_ids_and_allocated_capacity() {
        let mut layer = GraphicsLayer::new();
        layer.max_partial_bytes = 24;
        for command in [
            b"a=T,i=1,m=1;AAAAAAAAAA".as_slice(),
            b"a=T,i=2,m=1;AAAAAAAAAA",
            b"a=T,i=1,m=1;AAAA",
        ] {
            assert!(!layer.kitty(gfx::kitty::parse(command), 0, 0, 0));
            assert!(layer.pending_bytes() <= 24);
        }
        assert_eq!(
            layer.partial[&1].data.len(),
            14,
            "m=1 continuation must append"
        );
        layer.kitty(gfx::kitty::parse(b"a=T,i=1,m=1;A"), 0, 0, 0);
        assert!(
            !layer.partial.contains_key(&1),
            "over-budget transfer is discarded"
        );
        assert_eq!(layer.pending_bytes(), 10);
        layer.clear();
        assert_eq!(layer.pending_bytes(), 0);
        for id in 0..32 {
            layer.kitty(
                gfx::kitty::parse(format!("a=T,i={id},m=1;A").as_bytes()),
                0,
                0,
                0,
            );
        }
        assert_eq!(layer.partial.len(), MAX_PARTIAL_TRANSFERS);
    }

    #[test]
    fn three_or_more_kitty_chunks_preserve_every_payload_fragment() {
        let mut layer = GraphicsLayer::new();
        for command in [b"a=T,f=24,s=1x1,m=1;A".as_slice(), b"a=T,m=1;A"] {
            assert!(!layer.kitty(gfx::kitty::parse(command), 0, 0, 0));
        }
        assert!(layer.kitty(gfx::kitty::parse(b"a=T,m=0;AA"), 0, 0, 0));
        assert_eq!(layer.images[0].image.rgba, [0, 0, 0, 255]);
        assert_eq!(layer.pending_bytes(), 0);
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
    fn kitty_rgba_transmit_and_display_places() {
        let mut l = GraphicsLayer::new();
        // 2x2 RGBA (f=32): four opaque red pixels.
        let placed = l.kitty(
            gfx::kitty::parse(b"a=T,f=32,s=2x2,i=1,c=8,r=8;/wAA//8AAP//AAD//wAA/w=="),
            0,
            0,
            0,
        );
        assert!(placed, "decodes");
        assert_eq!(l.images.len(), 1);
        let im = &l.images[0];
        assert_eq!((im.image.width, im.image.height), (2, 2));
        assert_eq!(&im.image.rgba[..4], &[255, 0, 0, 255]);
        assert_eq!((im.cols, im.rows), (Some(8), Some(8)));
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

    #[test]
    fn negative_z_index_draws_behind_text() {
        assert!(draws_behind_text(-1));
        assert!(draws_behind_text(-4));
        assert!(draws_behind_text(i32::MIN));
        assert!(!draws_behind_text(0), "the default layer is above text");
        assert!(!draws_behind_text(1));
        assert!(!draws_behind_text(i32::MAX));
    }

    #[test]
    fn kitty_placement_keeps_its_z_index() {
        let mut l = GraphicsLayer::new();
        l.kitty(gfx::kitty::parse(b"a=T,f=24,s=1x1,i=1,z=-4;AAAA"), 0, 0, 0);
        assert_eq!(l.images[0].z, -4);
        assert!(draws_behind_text(l.images[0].z));
        l.kitty(gfx::kitty::parse(b"a=T,f=24,s=1x1,i=2,z=3;AAAA"), 0, 0, 0);
        assert_eq!(l.images[1].z, 3);
        assert!(!draws_behind_text(l.images[1].z));
    }
}
