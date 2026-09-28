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
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            cache,
            atlas,
            renderer,
            viewport,
            buffers: Vec::new(),
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

/// Renderer crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
