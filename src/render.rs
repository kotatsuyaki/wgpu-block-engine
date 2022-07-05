use std::mem::size_of;
use std::num::NonZeroU32;

use bytemuck::{Pod, Zeroable};
use glam::{vec3, Mat4};
use tokio::time::Instant;
use tracing::{error, info};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::*;
use winit::{dpi::PhysicalSize, window::Window};

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

    uniform_buffer: Buffer,
    uniform_bind_group: BindGroup,

    #[allow(dead_code)]
    grass_texture: Texture,
    grass_bind_group: BindGroup,

    last_update: tokio::time::Instant,
    angle: f32,
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

        // Create data buffers
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

        // Create uniform buffer
        let uniform_data = Self::calculate_uniform_data(&config, 0.);
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

            vertex_buffer,
            index_buffer,
            num_indices,

            uniform_buffer,
            uniform_bind_group,

            grass_texture,
            grass_bind_group,

            last_update: Instant::now(),
            angle: 0.,
        }
    }

    fn calculate_uniform_data(config: &SurfaceConfiguration, angle: f32) -> UniformData {
        // let eye = vec3(0.0, -3.0, 2.0);
        let eye = vec3(3.0 * f32::sin(angle), 3.0, 3.0 * f32::cos(angle));
        info!(?eye);
        let center = vec3(0.0, 0.0, 0.0);
        let up = vec3(0.0, 1.0, 0.0);

        let proj = Mat4::perspective_rh(
            std::f32::consts::FRAC_PI_4,
            config.width as f32 / config.height as f32,
            0.1,
            100.0,
        );
        let view = Mat4::look_at_rh(eye, center, up);
        UniformData { trans: proj * view }
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
            bytemuck::cast_slice(&[Self::calculate_uniform_data(&self.config, self.angle)]),
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
            bytemuck::cast_slice(&[Self::calculate_uniform_data(&self.config, self.angle)]),
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
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), IndexFormat::Uint16);
        render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        render_pass.set_bind_group(1, &self.grass_bind_group, &[]);
        render_pass.draw_indexed(0..self.num_indices, 0, 0..1);
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
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UniformData {
    trans: Mat4,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    color: [f32; 3],
    texcoord: [f32; 2],
}

impl From<([f32; 3], [f32; 3], [f32; 2])> for Vertex {
    fn from((pos, color, texcoord): ([f32; 3], [f32; 3], [f32; 2])) -> Self {
        Self {
            pos,
            color,
            texcoord,
        }
    }
}

fn create_vertices() -> (Vec<Vertex>, Vec<u16>) {
    // Coordinate system
    //
    //    (-1,+1,-1)______ (+1,+1,-1)
    //             /     /|               ^ +y
    //            /     / |               |
    // (-1,+1,+1)/_____/(+1,+1,+1)        |
    //    (-1,-1,-1)-  |  /(+1,-1,-1)     ---> +x
    //           |     | /               /
    // (-1,-1,+1)|_____|/(+1,-1,+1)     v +z

    let vertex_data = [
        // top
        ([-1., 1., -1.], [0., 0., 0.], [0., 0.]),
        ([-1., 1., 1.], [0., 0., 0.], [0., 1.]),
        ([1., 1., 1.], [0., 0., 0.], [1., 1.]),
        ([1., 1., -1.], [0., 0., 0.], [1., 0.]),
        // bottom
        ([-1., -1., 1.], [0., 0., 0.], [0., 0.]),
        ([-1., -1., -1.], [0., 0., 0.], [0., 1.]),
        ([1., -1., -1.], [0., 0., 0.], [1., 1.]),
        ([1., -1., 1.], [0., 0., 0.], [1., 0.]),
        // right
        ([1., 1., 1.], [0., 0., 0.], [0., 0.]),
        ([1., -1., 1.], [0., 0., 0.], [0., 1.]),
        ([1., -1., -1.], [0., 0., 0.], [1., 1.]),
        ([1., 1., -1.], [0., 0., 0.], [1., 0.]),
        // left
        ([-1., 1., -1.], [0., 0., 0.], [0., 0.]),
        ([-1., -1., -1.], [0., 0., 0.], [0., 1.]),
        ([-1., -1., 1.], [0., 0., 0.], [1., 1.]),
        ([-1., 1., 1.], [0., 0., 0.], [1., 0.]),
        // front
        ([-1., 1., 1.], [0., 0., 0.], [0., 0.]),
        ([-1., -1., 1.], [0., 0., 0.], [0., 1.]),
        ([1., -1., 1.], [0., 0., 0.], [1., 1.]),
        ([1., 1., 1.], [0., 0., 0.], [1., 0.]),
        // rear
        ([1., 1., -1.], [0., 0., 0.], [0., 0.]),
        ([1., -1., -1.], [0., 0., 0.], [0., 1.]),
        ([-1., -1., -1.], [0., 0., 0.], [1., 1.]),
        ([-1., 1., -1.], [0., 0., 0.], [1., 0.]),
    ]
    .into_iter()
    .map(Into::into)
    .collect();

    let index_data = [
        0, 1, 2, 2, 3, 0, //
        4, 5, 6, 6, 7, 4, //
        8, 9, 10, 10, 11, 8, //
        12, 13, 14, 14, 15, 12, //
        16, 17, 18, 18, 19, 16, //
        20, 21, 22, 22, 23, 20, //
    ]
    .to_vec();

    (vertex_data, index_data)
}

mod assets {
    pub const GRASSTOP: &[u8] = include_bytes!("../assets/grass-top-arrow.png");
}
