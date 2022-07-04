use std::mem::size_of;

use bytemuck::{Pod, Zeroable};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::*;
use winit::{dpi::PhysicalSize, window::Window};

#[derive(Debug)]
pub struct Render {
    surface: Surface,
    device: Device,
    queue: Queue,
    pipeline: RenderPipeline,
    size: PhysicalSize<u32>,
    config: SurfaceConfiguration,
    vertex_buffer: Buffer,
    index_buffer: Buffer,
    num_indices: u32,
}

impl Render {
    pub async fn new(window: &Window) -> Self {
        let inst = wgpu::Instance::new(Backends::all());
        let surface = unsafe { inst.create_surface(window) };
        let adapter = inst
            .request_adapter(&RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to request adapter");
        let (device, queue) = adapter
            .request_device(
                &DeviceDescriptor {
                    label: None,
                    limits: Limits::downlevel_defaults(),
                    features: Features::default(),
                },
                None,
            )
            .await
            .expect("Failed to request device");

        let size = window.inner_size();
        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface.get_supported_formats(&adapter)[0],
            width: size.width,
            height: size.height,
            present_mode: PresentMode::Fifo,
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(include_wgsl!("./shader.wgsl"));
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("PipelineLayout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("RenderPipeline"),
            layout: Some(&layout),
            vertex: VertexState {
                module: &shader,
                entry_point: "main_vs",
                buffers: &[VertexBufferLayout {
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                    array_stride: size_of::<Vertex>() as BufferAddress,
                }],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: "main_fs",
                targets: &[Some(ColorTargetState {
                    format: config.format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: Some(Face::Back),
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        let (vertex_data, index_data) = create_vertices();
        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&index_data),
            usage: BufferUsages::INDEX,
        });
        let num_indices = index_data.len() as u32;

        Self {
            surface,
            device,
            queue,
            pipeline,
            size,
            config,
            vertex_buffer,
            index_buffer,
            num_indices,
        }
    }

    pub fn size(&self) -> PhysicalSize<u32> {
        self.size
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        assert_ne!(size.width, 0);
        assert_ne!(size.height, 0);

        self.size = size;
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn update(&mut self) {}

    pub fn render(&mut self) -> Result<(), SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Render Command Encoder"),
            });

        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("Render Pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Color {
                        r: 0.1,
                        g: 0.2,
                        b: 0.3,
                        a: 1.0,
                    }),
                    store: true,
                },
            })],
            depth_stencil_attachment: None,
        });
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), IndexFormat::Uint16);
        render_pass.draw_indexed(0..self.num_indices, 0, 0..1);
        drop(render_pass);

        self.queue.submit([encoder.finish()]);
        output.present();

        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    color: [f32; 3],
}

impl From<([f32; 3], [f32; 3])> for Vertex {
    fn from((pos, color): ([f32; 3], [f32; 3])) -> Self {
        Self { pos, color }
    }
}

fn create_vertices() -> (Vec<Vertex>, Vec<u16>) {
    let vertex_data = [
        // top (0, 0, 1)
        ([-1., -1., 1.], [1., 0., 0.]),
        ([1., -1., 1.], [0., 1., 0.]),
        ([1., 1., 1.], [0., 0., 1.]),
        ([-1., 1., 1.], [1., 1., 0.]),
        // bottom (0., 0., -1.)
        ([-1., 1., -1.], [1., 0., 0.]),
        ([1., 1., -1.], [0., 1., 0.]),
        ([1., -1., -1.], [0., 0., 1.]),
        ([-1., -1., -1.], [1., 1., 0.]),
        // right (1., 0., 0.)
        ([1., -1., -1.], [1., 0., 0.]),
        ([1., 1., -1.], [0., 1., 0.]),
        ([1., 1., 1.], [0., 0., 1.]),
        ([1., -1., 1.], [1., 1., 0.]),
        // left (-1., 0., 0.)
        ([-1., -1., 1.], [1., 0., 0.]),
        ([-1., 1., 1.], [0., 1., 0.]),
        ([-1., 1., -1.], [0., 0., 1.]),
        ([-1., -1., -1.], [1., 1., 0.]),
        // front (0., 1., 0.)
        ([1., 1., -1.], [1., 0., 0.]),
        ([-1., 1., -1.], [0., 1., 0.]),
        ([-1., 1., 1.], [0., 0., 1.]),
        ([1., 1., 1.], [1., 1., 0.]),
        // back (0., -1., 0.)
        ([1., -1., 1.], [1., 0., 0.]),
        ([-1., -1., 1.], [0., 1., 0.]),
        ([-1., -1., -1.], [0., 0., 1.]),
        ([1., -1., -1.], [1., 1., 0.]),
    ]
    .into_iter()
    .map(Into::into)
    .collect();

    let index_data = [
        0, 1, 2, 2, 3, 0, // top
        4, 5, 6, 6, 7, 4, // bottom
        8, 9, 10, 10, 11, 8, // right
        12, 13, 14, 14, 15, 12, // left
        16, 17, 18, 18, 19, 16, // front
        20, 21, 22, 22, 23, 20, // back
    ]
    .to_vec();

    (vertex_data, index_data)
}
