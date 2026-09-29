//! Offscreen smoke test: draw quads + glyphs with the native pipelines and
//! write a PPM, so the renderer can be verified without a window.

use miao_term_render::{Quad, QuadRenderer, Span, TermRenderer};

const W: u32 = 640;
const H: u32 = 220;
const SCALE: f32 = 2.0;
const FONT: f32 = 13.0;
const LINE: f32 = 17.0;
const CW: f32 = 7.8;

fn main() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .unwrap();

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut glyphs = TermRenderer::new(&device, &queue, format);
    let mut quads = QuadRenderer::new(&device, format);

    // A couple of background quads (a "selection" band + a cursor block).
    let quads_data = vec![
        Quad::new(
            (0.0, 0.0),
            (W as f32, LINE * SCALE),
            (0x43, 0x4c, 0x5e, 255),
        ),
        Quad::new(
            (CW * SCALE * 10.0, 0.0),
            (CW * SCALE * 11.0, LINE * SCALE),
            (0xd8, 0xde, 0xe9, 255),
        ),
    ];
    quads.prepare(&device, &queue, (W, H), &quads_data);

    let white = (0xd8, 0xde, 0xe9);
    let rows = vec![
        vec![Span::new(0, "ASCII cell-pinned: 0123456789", white)],
        vec![
            Span::new(0, "CJK: ", white),
            Span::new(5, "你好世界", white),
            Span::new(13, " ok", white),
        ],
    ];
    glyphs.prepare(
        &device,
        &queue,
        (W, H),
        SCALE,
        FONT,
        LINE,
        CW,
        0.0,
        0.0,
        white,
        Some("JetBrains Mono"),
        &rows,
    );

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
        });
        quads.render(&mut pass);
        glyphs.render(&mut pass);
    }

    let bytes_per_row = (W * 4) as usize;
    let padded = (bytes_per_row + 255) & !255;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (padded * H as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: &buffer,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();

    let mut ppm = format!("P6\n{W} {H}\n255\n").into_bytes();
    for y in 0..H as usize {
        let row = &data[y * padded..y * padded + bytes_per_row];
        for x in 0..W as usize {
            ppm.push(row[x * 4]);
            ppm.push(row[x * 4 + 1]);
            ppm.push(row[x * 4 + 2]);
        }
    }
    std::fs::write("/tmp/pipeline.ppm", ppm).unwrap();
    println!("wrote /tmp/pipeline.ppm");
}
