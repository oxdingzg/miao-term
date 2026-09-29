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

/// A solid-colour rectangle in physical pixels (background / selection / cursor).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Quad {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub color: [f32; 4],
    /// Corner radius in physical pixels (0 = square).
    pub radius: f32,
}

impl Quad {
    pub fn new(min: (f32, f32), max: (f32, f32), color: (u8, u8, u8, u8)) -> Self {
        Self {
            min: [min.0, min.1],
            max: [max.0, max.1],
            color: [
                color.0 as f32 / 255.0,
                color.1 as f32 / 255.0,
                color.2 as f32 / 255.0,
                color.3 as f32 / 255.0,
            ],
            radius: 0.0,
        }
    }

    pub fn rounded(min: (f32, f32), max: (f32, f32), color: (u8, u8, u8, u8), radius: f32) -> Self {
        let mut q = Self::new(min, max, color);
        q.radius = radius;
        q
    }
}

const QUAD_WGSL: &str = r#"
struct Globals { resolution: vec2<f32>, _pad: vec2<f32> };
@group(0) @binding(0) var<uniform> globals: Globals;

struct VsIn {
    @location(0) min: vec2<f32>,
    @location(1) max: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) radius: f32,
    @builtin(vertex_index) vi: u32,
};
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) rect_min: vec2<f32>,
    @location(2) rect_max: vec2<f32>,
    @location(3) radius: f32,
};

@vertex
fn vs(in: VsIn) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[in.vi];
    let p = mix(in.min, in.max, c);
    let ndc = vec2<f32>(
        p.x / globals.resolution.x * 2.0 - 1.0,
        p.y / globals.resolution.y * 2.0 - 1.0,
    );
    var o: VsOut;
    o.pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    o.color = in.color;
    o.rect_min = in.min;
    o.rect_max = in.max;
    o.radius = in.radius;
    return o;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    var alpha = in.color.a;
    if (in.radius > 0.5) {
        let center = (in.rect_min + in.rect_max) * 0.5;
        let half = (in.rect_max - in.rect_min) * 0.5;
        let b = max(half - vec2<f32>(in.radius), vec2<f32>(0.0));
        let q = abs(in.pos.xy - center) - b;
        let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - in.radius;
        alpha = in.color.a * (1.0 - smoothstep(-1.0, 1.0, d));
    }
    return vec4<f32>(in.color.rgb, alpha);
}
"#;

/// Instanced renderer for solid-colour quads, drawn beneath the glyphs.
pub struct QuadRenderer {
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    capacity: usize,
    count: u32,
}

impl QuadRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad shader"),
            source: wgpu::ShaderSource::Wgsl(QUAD_WGSL.into()),
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quad pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Quad>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let capacity = 4096;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad instances"),
            size: (capacity * std::mem::size_of::<Quad>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            globals,
            bind_group,
            instances,
            capacity,
            count: 0,
        }
    }

    /// Upload this frame's quads. `resolution` is the surface size in pixels.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resolution: (u32, u32),
        quads: &[Quad],
    ) {
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck_cast(&[resolution.0 as f32, resolution.1 as f32, 0.0, 0.0]),
        );
        if quads.len() > self.capacity {
            self.capacity = quads.len().next_power_of_two();
            self.instances = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("quad instances"),
                size: (self.capacity * std::mem::size_of::<Quad>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !quads.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck_cast(quads));
        }
        self.count = quads.len() as u32;
    }

    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

/// Reinterpret a slice of `#[repr(C)]` plain-old-data as bytes (no bytemuck dep).
fn bytemuck_cast<T: Copy>(data: &[T]) -> &[u8] {
    // SAFETY: `T` is `#[repr(C)]`/POD here (f32 / Quad of f32) with no padding
    // bits that matter, so viewing it as bytes is sound.
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) }
}

/// Renderer crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod gpu_tests {
    use super::*;

    /// Headless smoke test: render a quad + a glyph row offscreen and assert the
    /// pipeline produces output. Skips itself when no GPU adapter is available
    /// (e.g. a CI runner without a driver), so it is safe to run in CI.
    #[test]
    fn offscreen_render_smoke() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let Some(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            assert!(
                std::env::var_os("MIAO_REQUIRE_GPU").is_none(),
                "MIAO_REQUIRE_GPU is set but no wgpu adapter is available"
            );
            return;
        };
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
        else {
            assert!(
                std::env::var_os("MIAO_REQUIRE_GPU").is_none(),
                "MIAO_REQUIRE_GPU is set but no wgpu device could be created"
            );
            return;
        };

        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let (w, h) = (64u32, 40u32);
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());

        let mut quads = QuadRenderer::new(&device, format);
        quads.prepare(
            &device,
            &queue,
            (w, h),
            &[Quad::rounded(
                (2.0, 2.0),
                (30.0, 20.0),
                (0xff, 0xff, 0xff, 255),
                4.0,
            )],
        );
        let mut glyphs = TermRenderer::new(&device, &queue, format);
        let rows = vec![vec![Span::new(0, "Hi", (255, 255, 255))]];
        glyphs.prepare(
            &device,
            &queue,
            (w, h),
            1.0,
            12.0,
            14.0,
            7.0,
            0.0,
            0.0,
            (255, 255, 255),
            Some("JetBrains Mono"),
            &rows,
        );

        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                })
                .forget_lifetime();
            quads.render(&mut pass);
            glyphs.render(&mut pass);
        }
        let bpr = (w * 4) as usize;
        let padded = (bpr + 255) & !255;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (padded * h as usize) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &buf,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(enc.finish()));
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::Maintain::Wait);
        let data = slice.get_mapped_range();
        assert!(data.iter().any(|&b| b > 0), "renderer produced no output");
    }
}
