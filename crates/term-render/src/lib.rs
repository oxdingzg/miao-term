//! `miao-term-render` — GPU glyph-grid renderer (wgpu + glyphon).
//!
//! The app builds row runs from the terminal screen and hands them here; this
//! crate owns the font system, glyph atlas, and the glyphon text pipeline. It
//! draws into the same `wgpu` device/queue/surface as egui.

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};

/// A run of same-colored text within a row, pinned to a starting cell column.
///
/// Runs may only contain single-cell characters; a wide (2-cell) character must
/// be its own span so it can be positioned at an exact cell.
pub struct Span {
    pub col: u16,
    pub text: String,
    pub color: (u8, u8, u8),
}

impl Span {
    pub fn new(col: u16, text: impl Into<String>, color: (u8, u8, u8)) -> Self {
        Self {
            col,
            text: text.into(),
            color,
        }
    }
}

/// Owns glyphon state for drawing a grid of text.
pub struct TermRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    #[allow(dead_code)]
    cache: Cache,
    atlas: TextAtlas,
    renderer: TextRenderer,
    viewport: Viewport,
    buffers: Vec<Buffer>,
    frames: u64,
}

/// Candidate system CJK fonts (macOS / Linux / Windows).
const CJK_FONTS: &[&str] = &[
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/STHeiti Light.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "C:/Windows/Fonts/msyh.ttc",
    "C:/Windows/Fonts/simhei.ttf",
];

/// Fonts bundled with the app (embedded at compile time; see `assets/fonts/`).
const BUNDLED_FONTS: &[&[u8]] = &[
    include_bytes!("../../../assets/fonts/JetBrainsMono.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../../../assets/fonts/SymbolsNerdFontMono-Regular.ttf"),
];

/// Load the default monospace font plus the Nerd Font symbol fallback.
fn load_bundled(font_system: &mut FontSystem) {
    for bytes in BUNDLED_FONTS {
        font_system.db_mut().load_font_data(bytes.to_vec());
    }
}

/// Load a system CJK font into the font database so CJK glyphs render.
fn load_system_cjk(font_system: &mut FontSystem) {
    for path in CJK_FONTS {
        if let Ok(bytes) = std::fs::read(path) {
            font_system.db_mut().load_font_data(bytes);
            return;
        }
    }
}

impl TermRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let cache = Cache::new(device);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let viewport = Viewport::new(device, &cache);
        let mut font_system = FontSystem::new();
        load_bundled(&mut font_system);
        load_system_cjk(&mut font_system);
        Self {
            font_system,
            swash_cache: SwashCache::new(),
            cache,
            atlas,
            renderer,
            viewport,
            buffers: Vec::new(),
            frames: 0,
        }
    }

    /// Prepare the glyphs for this frame.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pixels: (u32, u32),
        scale: f32,
        font_size: f32,
        line_height: f32,
        cell_width: f32,
        left: f32,
        top: f32,
        default_color: (u8, u8, u8),
        family: Option<&str>,
        rows: &[Vec<Span>],
    ) {
        self.viewport.update(
            queue,
            Resolution {
                width: pixels.0.max(1),
                height: pixels.1.max(1),
            },
        );

        // Periodically repack the glyph atlas to reclaim space from old glyphs.
        self.frames = self.frames.wrapping_add(1);
        if self.frames % 300 == 0 {
            self.atlas.trim();
        }

        let metrics = Metrics::new(font_size, line_height);
        let fam = match family {
            Some(name) => Family::Name(name),
            None => Family::Monospace,
        };

        // `left`/`top`/`line_height`/`cell_width` are logical points relative to
        // the target viewport's origin; glyphon positions in *physical* pixels
        // and already scales per-glyph advances by `scale`. `pixels` must be the
        // viewport's pixel size (egui-wgpu sets the render viewport to the
        // callback's rect), not the whole surface.
        let left_px = left * scale;
        let top_px = top * scale;
        let line_px = line_height * scale;
        let cell_px = cell_width * scale;
        let bounds = TextBounds {
            left: left_px as i32,
            top: top_px as i32,
            right: pixels.0 as i32,
            bottom: pixels.1 as i32,
        };

        // One buffer per span, positioned at an exact cell column, so wide (CJK)
        // glyphs whose advance isn't exactly 2 cells can't drift the row.
        let total: usize = rows.iter().map(|r| r.len()).sum();
        while self.buffers.len() < total {
            self.buffers
                .push(Buffer::new(&mut self.font_system, metrics));
        }

        let mut positions: Vec<(f32, f32)> = Vec::with_capacity(total);
        let mut idx = 0usize;
        for (row_idx, row) in rows.iter().enumerate() {
            for span in row {
                let buffer = &mut self.buffers[idx];
                idx += 1;
                buffer.set_metrics(&mut self.font_system, metrics);
                buffer.set_size(&mut self.font_system, None, None);
                let attrs = Attrs::new().family(fam).color(Color::rgb(
                    span.color.0,
                    span.color.1,
                    span.color.2,
                ));
                // `Shaping::Advanced` is required for font fallback: `Basic`
                // renders glyphs missing from the primary font as tofu.
                buffer.set_rich_text(
                    &mut self.font_system,
                    std::iter::once((span.text.as_str(), attrs)),
                    Attrs::new().family(fam),
                    Shaping::Advanced,
                );
                positions.push((
                    left_px + span.col as f32 * cell_px,
                    top_px + row_idx as f32 * line_px,
                ));
            }
        }

        let areas: Vec<TextArea> = self
            .buffers
            .iter()
            .take(total)
            .zip(positions.iter())
            .map(|(buffer, (x, y))| TextArea {
                buffer,
                left: *x,
                top: *y,
                scale,
                bounds,
                default_color: Color::rgb(default_color.0, default_color.1, default_color.2),
                custom_glyphs: &[],
            })
            .collect();

        if let Err(e) = self.renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash_cache,
        ) {
            eprintln!("miaotty: text prepare error: {e}");
        }
    }

    /// Draw prepared glyphs into the pass.
    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Err(e) = self.renderer.render(&self.atlas, &self.viewport, pass) {
            eprintln!("miaotty: text render error: {e}");
        }
    }
}

/// Measures the cell size from the actual font, so grid layout matches the
/// glyphs the renderer draws.
pub struct MetricsProbe {
    font_system: FontSystem,
    buffer: Buffer,
}

impl Default for MetricsProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsProbe {
    pub fn new() -> Self {
        let mut font_system = FontSystem::new();
        load_bundled(&mut font_system);
        load_system_cjk(&mut font_system);
        let buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
        Self {
            font_system,
            buffer,
        }
    }

    /// Returns `(cell_width, line_height)` for the given font size.
    pub fn cell(&mut self, font_size: f32, line_height: f32, family: Option<&str>) -> (f32, f32) {
        let metrics = Metrics::new(font_size, line_height);
        let fam = match family {
            Some(name) => Family::Name(name),
            None => Family::Monospace,
        };
        self.buffer.set_metrics(&mut self.font_system, metrics);
        self.buffer.set_size(&mut self.font_system, None, None);
        self.buffer.set_text(
            &mut self.font_system,
            "M",
            Attrs::new().family(fam),
            Shaping::Basic,
        );
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        let mut width = font_size * 0.6;
        for run in self.buffer.layout_runs() {
            if run.line_w > 0.0 {
                width = run.line_w;
                break;
            }
        }
        (width, line_height)
    }
}

/// Renderer crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
