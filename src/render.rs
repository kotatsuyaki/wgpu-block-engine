//! Rendering primitives for rendering to host buffer and for rendering the result via the graphics API.
//!
//! # Coordinate system
//!
//! ```
//!    (-1,+1,-1)______ (+1,+1,-1)
//!             /     /|               ^ +y
//!            /     / |               |
//! (-1,+1,+1)/_____/(+1,+1,+1)        |
//!    (-1,-1,-1)-  |  /(+1,-1,-1)     ---> +x
//!           |     | /               /
//! (-1,-1,+1)|_____|/(+1,-1,+1)     v +z
//! ```

use std::mem::size_of;
use std::num::NonZeroU32;

use bytemuck::{Pod, Zeroable};
use glam::{vec3, vec4, Mat4, Vec4};
use hashbrown::HashMap;
use tokio::time::Instant;
use tracing::error;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::*;
use winit::{dpi::PhysicalSize, window::Window};

/// A collection of objects needed for rendering and presenting.
pub struct Render {
    surface: Surface,
    device: Device,
    queue: Queue,
    pipeline: RenderPipeline,
    size: PhysicalSize<u32>,
    config: SurfaceConfiguration,

    uniform_buffer: Buffer,
    uniform_bind_group: BindGroup,

    #[allow(dead_code)]
    grass_texture: Texture,
    grass_bind_group: BindGroup,

    last_update: tokio::time::Instant,
    angle: f32,

    rendered: RenderedBufferCollection,
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
                    limits: Limits {
                        max_push_constant_size: size_of::<PushConstants>() as u32,
                        ..Default::default()
                    },
                    features: Features::default()
                        .union(Features::TEXTURE_BINDING_ARRAY)
                        .union(Features::PUSH_CONSTANTS),
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

        // Create shader and layouts
        let shader = device.create_shader_module(include_wgsl!("./shader.wgsl"));
        let uniform_data_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Uniform Data Bind Group Layout"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let grass_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("Grass Texture Bind Group Layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("PipelineLayout"),
            bind_group_layouts: &[&uniform_data_layout, &grass_bind_group_layout],
            push_constant_ranges: &[PushConstantRange {
                range: 0..16,
                stages: ShaderStages::VERTEX,
            }],
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("RenderPipeline"),
            layout: Some(&layout),
            vertex: VertexState {
                module: &shader,
                entry_point: "main_vs",
                buffers: &[VertexBufferLayout {
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
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

        // Create uniform buffer
        let uniform_data = Self::calculate_uniforms(&config, 0.);
        let uniform_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniform_data]),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
        let uniform_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Uniform Bind Group"),
            layout: &uniform_data_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // Load texture
        let grass_top_img = image::load_from_memory(assets::GRASSTOP)
            .unwrap()
            .to_rgba8();
        let grass_top_size = Extent3d {
            width: grass_top_img.width(),
            height: grass_top_img.height(),
            depth_or_array_layers: 1,
        };
        let grass_texture = device.create_texture(&TextureDescriptor {
            label: Some("Grass Texture"),
            size: grass_top_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        });
        queue.write_texture(
            ImageCopyTexture {
                texture: &grass_texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &grass_top_img,
            ImageDataLayout {
                offset: 0,
                bytes_per_row: NonZeroU32::new(4 * grass_top_img.width()),
                rows_per_image: NonZeroU32::new(grass_top_img.height()),
            },
            grass_top_size,
        );

        let grass_texture_view = grass_texture.create_view(&TextureViewDescriptor::default());
        let grass_texture_sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("Grass Texture Sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Nearest,
            ..Default::default()
        });
        let grass_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("Grass Texture Bind Group"),
            layout: &grass_bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&grass_texture_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&grass_texture_sampler),
                },
            ],
        });

        Self {
            surface,
            device,
            queue,
            pipeline,
            size,
            config,

            uniform_buffer,
            uniform_bind_group,

            grass_texture,
            grass_bind_group,

            last_update: Instant::now(),
            angle: 0.,

            rendered: RenderedBufferCollection::new(),
        }
    }

    fn calculate_uniforms(config: &SurfaceConfiguration, _angle: f32) -> Uniforms {
        let eye = vec3(40.0, 40.0, 40.0);
        let center = vec3(0.0, 20.0, 0.0);
        let up = vec3(0.0, 1.0, 0.0);

        let proj = Mat4::perspective_rh(
            std::f32::consts::FRAC_PI_4,
            config.width as f32 / config.height as f32,
            0.1,
            100.0,
        );
        let view = Mat4::look_at_rh(eye, center, up);
        Uniforms { trans: proj * view }
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

        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[Self::calculate_uniforms(&self.config, self.angle)]),
        );
    }

    pub fn update(&mut self) {
        let elapsed = self.last_update.elapsed().as_micros() as f32;
        self.last_update = Instant::now();

        // 0.1 rad / s = 0.000_000_1 rad / us
        self.angle += elapsed * 0.000_000_1;

        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[Self::calculate_uniforms(&self.config, self.angle)]),
        );
    }

    pub async fn render(&mut self) -> Result<(), SurfaceError> {
        self.device.push_error_scope(ErrorFilter::Validation);

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
        for (&(cx, cy, cz), buffer) in self.rendered.buffers.iter_mut() {
            let RenderedBufferEntry {
                host_buffer,
                dirty,
                vertex_buffer,
                index_buffer,
            } = buffer;

            if host_buffer.indices.is_empty() {
                continue;
            }

            if *dirty {
                self.queue
                    .write_buffer(vertex_buffer, 0, host_buffer.vertices.as_u8_slice());
                self.queue
                    .write_buffer(index_buffer, 0, host_buffer.indices.as_u8_slice());
                *dirty = false;
            }

            let push_constants = PushConstants::new((cx, cy, cz));

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_index_buffer(index_buffer.slice(..), IndexFormat::Uint16);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            render_pass.set_bind_group(1, &self.grass_bind_group, &[]);
            render_pass.set_push_constants(ShaderStages::VERTEX, 0, push_constants.as_u8_slice());

            let num_indices = host_buffer.indices.len() as u32;
            render_pass.draw_indexed(0..num_indices, 0, 0..1);
        }

        drop(render_pass);
        self.queue.submit([encoder.finish()]);

        // report on error
        let err_scope = self.device.pop_error_scope();
        tokio::spawn(async {
            let out = err_scope.await;
            if let Some(err) = out {
                error!(?err);
            }
        });

        output.present();

        Ok(())
    }

    pub fn insert_rendered(&mut self, key: RenderedBufferKey, host_buffer: RenderedBuffer) {
        let vertex_data: &[u8] = bytemuck::cast_slice(&host_buffer.vertices);
        let index_data: &[u8] = bytemuck::cast_slice(&host_buffer.indices);

        let vertex_buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some("Vertex Buffer"),
            size: vertex_data.len() as BufferAddress,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some("Index Buffer"),
            size: index_data.len() as BufferAddress,
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.rendered.buffers.insert(
            key,
            RenderedBufferEntry {
                host_buffer,
                vertex_buffer,
                index_buffer,
                dirty: true,
            },
        );
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    trans: Mat4,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PushConstants {
    shift: Vec4,
}

impl PushConstants {
    fn new((cx, cy, cz): (i64, i64, i64)) -> Self {
        Self {
            shift: vec4(cx as f32 * 16., cy as f32 * 16., cz as f32 * 16., 0.0),
        }
    }
}

/// A host-side rendered buffer containing vertices and indices.
#[derive(Clone)]
pub struct RenderedBuffer {
    vertices: Vec<Vertex>,
    indices: Vec<u16>,
    max_index: Option<u16>,
}

impl RenderedBuffer {
    pub fn new() -> Self {
        Self {
            vertices: vec![],
            indices: vec![],
            max_index: None,
        }
    }

    pub fn push_face(&mut self, base_face: [Vertex; 4], (sx, sy, sz): (i64, i64, i64)) {
        self.vertices
            .extend_from_slice(&shift_face(base_face, (sx as f32, sy as f32, sz as f32)));

        let index_start = self.max_index.map(|i| i + 1).unwrap_or(0);
        self.max_index = Some(index_start + 3);

        self.indices
            .extend_from_slice(&shift_indices(FACE_INDICES, index_start));
    }
}

pub struct RenderedBufferCollection {
    buffers: HashMap<RenderedBufferKey, RenderedBufferEntry>,
}

struct RenderedBufferEntry {
    host_buffer: RenderedBuffer,
    vertex_buffer: Buffer,
    index_buffer: Buffer,
    dirty: bool,
}

pub type RenderedBufferKey = (i64, i64, i64);

impl RenderedBufferCollection {
    fn new() -> Self {
        Self {
            buffers: HashMap::new(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pos: [f32; 3],
    color: [f32; 3],
    texcoord: [f32; 2],
}

pub const TOP_FACE: [Vertex; 4] = [
    Vertex {
        pos: [0., 1., 0.],
        color: [0., 0., 0.],
        texcoord: [0., 0.],
    },
    Vertex {
        pos: [0., 1., 1.],
        color: [0., 0., 0.],
        texcoord: [0., 1.],
    },
    Vertex {
        pos: [1., 1., 1.],
        color: [0., 0., 0.],
        texcoord: [1., 1.],
    },
    Vertex {
        pos: [1., 1., 0.],
        color: [0., 0., 0.],
        texcoord: [1., 0.],
    },
];

pub const BOTTOM_FACE: [Vertex; 4] = [
    Vertex {
        pos: [-1., -1., 1.],
        color: [0., 0., 0.],
        texcoord: [0., 0.],
    },
    Vertex {
        pos: [-1., -1., -1.],
        color: [0., 0., 0.],
        texcoord: [0., 1.],
    },
    Vertex {
        pos: [1., -1., -1.],
        color: [0., 0., 0.],
        texcoord: [1., 1.],
    },
    Vertex {
        pos: [1., -1., 1.],
        color: [0., 0., 0.],
        texcoord: [1., 0.],
    },
];

pub const RIGHT_FACE: [Vertex; 4] = [
    Vertex {
        pos: [1., 1., 1.],
        color: [0., 0., 0.],
        texcoord: [0., 0.],
    },
    Vertex {
        pos: [1., 0., 1.],
        color: [0., 0., 0.],
        texcoord: [0., 1.],
    },
    Vertex {
        pos: [1., 0., 0.],
        color: [0., 0., 0.],
        texcoord: [1., 1.],
    },
    Vertex {
        pos: [1., 1., 0.],
        color: [0., 0., 0.],
        texcoord: [1., 0.],
    },
];

pub const LEFT_FACE: [Vertex; 4] = [
    Vertex {
        pos: [0., 1., 0.],
        color: [0., 0., 0.],
        texcoord: [0., 0.],
    },
    Vertex {
        pos: [0., 0., 0.],
        color: [0., 0., 0.],
        texcoord: [0., 1.],
    },
    Vertex {
        pos: [0., 0., 1.],
        color: [0., 0., 0.],
        texcoord: [1., 1.],
    },
    Vertex {
        pos: [0., 1., 1.],
        color: [0., 0., 0.],
        texcoord: [1., 0.],
    },
];

pub const FRONT_FACE: [Vertex; 4] = [
    Vertex {
        pos: [0., 1., 1.],
        color: [0., 0., 0.],
        texcoord: [0., 0.],
    },
    Vertex {
        pos: [0., 0., 1.],
        color: [0., 0., 0.],
        texcoord: [0., 1.],
    },
    Vertex {
        pos: [1., 0., 1.],
        color: [0., 0., 0.],
        texcoord: [1., 1.],
    },
    Vertex {
        pos: [1., 1., 1.],
        color: [0., 0., 0.],
        texcoord: [1., 0.],
    },
];

pub const REAR_FACE: [Vertex; 4] = [
    Vertex {
        pos: [1., 1., 0.],
        color: [0., 0., 0.],
        texcoord: [0., 0.],
    },
    Vertex {
        pos: [1., 0., 0.],
        color: [0., 0., 0.],
        texcoord: [0., 1.],
    },
    Vertex {
        pos: [0., 0., 0.],
        color: [0., 0., 0.],
        texcoord: [1., 1.],
    },
    Vertex {
        pos: [0., 1., 0.],
        color: [0., 0., 0.],
        texcoord: [1., 0.],
    },
];

pub fn shift_face(base_face: [Vertex; 4], (dx, dy, dz): (f32, f32, f32)) -> [Vertex; 4] {
    base_face.map(|mut v| {
        v.pos = [v.pos[0] + dx, v.pos[1] + dy, v.pos[2] + dz];
        v
    })
}

pub const FACE_INDICES: [u16; 6] = [0, 1, 2, 2, 3, 0];

pub fn shift_indices(base_indices: [u16; 6], start_index: u16) -> [u16; 6] {
    base_indices.map(|i| i + start_index)
}

mod assets {
    pub const GRASSTOP: &[u8] = include_bytes!("../assets/grass-top-arrow.png");
}

trait AsU8Slice<'a> {
    fn as_u8_slice(self) -> &'a [u8];
}

impl<'a, T> AsU8Slice<'a> for &'a [T]
where
    T: bytemuck::Pod,
{
    fn as_u8_slice(self) -> &'a [u8] {
        bytemuck::cast_slice(self)
    }
}

impl<'a, T> AsU8Slice<'a> for &'a T
where
    T: bytemuck::Pod,
{
    fn as_u8_slice(self) -> &'a [u8] {
        bytemuck::bytes_of(self)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_push_constants_data_size() {
        assert_eq!(size_of::<PushConstants>(), 4 * 4);
    }
}
