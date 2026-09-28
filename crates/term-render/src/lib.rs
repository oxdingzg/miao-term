//! `miao-term-render` — GPU glyph-grid renderer (wgpu + glyphon).
//!
//! The app builds row runs from the terminal screen and hands them here; this
//! crate owns the font system, glyph atlas, and the glyphon text pipeline. It
//! draws into the same `wgpu` device/queue/surface as egui.

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};

/// A run of same-colored text within a row.
pub struct Span {
    pub text: String,
    pub color: (u8, u8, u8),
}

impl Span {
    pub fn new(text: impl Into<String>, color: (u8, u8, u8)) -> Self {
        Self {
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
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let cache = Cache::new(device);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let viewport = Viewport::new(device, &cache);
        let mut font_system = FontSystem::new();
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
        left: f32,
        top: f32,
        default_color: (u8, u8, u8),
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
        while self.buffers.len() < rows.len() {
            self.buffers
                .push(Buffer::new(&mut self.font_system, metrics));
        }

        for (i, row) in rows.iter().enumerate() {
            let buffer = &mut self.buffers[i];
            buffer.set_metrics(&mut self.font_system, metrics);
            buffer.set_size(&mut self.font_system, None, None);
            let owned: Vec<(String, Attrs)> = row
                .iter()
                .map(|span| {
                    let attrs = Attrs::new()
                        .family(Family::Monospace)
                        .color(Color::rgb(span.color.0, span.color.1, span.color.2));
                    (span.text.clone(), attrs)
                })
                .collect();
            buffer.set_rich_text(
                &mut self.font_system,
                owned.iter().map(|(text, attrs)| (text.as_str(), *attrs)),
                Attrs::new().family(Family::Monospace),
                Shaping::Basic,
            );
        }

        let bounds = TextBounds {
            left: (left * scale) as i32,
            top: (top * scale) as i32,
            right: pixels.0 as i32,
            bottom: pixels.1 as i32,
        };
        let areas: Vec<TextArea> = self
            .buffers
            .iter()
            .take(rows.len())
            .enumerate()
            .map(|(i, buffer)| TextArea {
                buffer,
                left,
                top: top + i as f32 * line_height,
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
        load_system_cjk(&mut font_system);
        let buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
        Self {
            font_system,
            buffer,
        }
    }

    /// Returns `(cell_width, line_height)` for the given font size.
    pub fn cell(&mut self, font_size: f32, line_height: f32) -> (f32, f32) {
        let metrics = Metrics::new(font_size, line_height);
        self.buffer
            .set_metrics(&mut self.font_system, metrics);
        self.buffer.set_size(&mut self.font_system, None, None);
        self.buffer.set_text(
            &mut self.font_system,
            "M",
            Attrs::new().family(Family::Monospace),
            Shaping::Basic,
        );
        self.buffer
            .shape_until_scroll(&mut self.font_system, false);
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
