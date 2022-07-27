use anyhow::Result;
use glam::{vec3, Mat4, Vec3};
use itertools::iproduct;
use render::Render;
use tokio::runtime::Handle;
use tracing::{info, warn};
use wgpu::SurfaceError;
use winit::{
    event::{ElementState, VirtualKeyCode, WindowEvent},
    event_loop::ControlFlow,
};

mod chunk;
mod render;

fn main() -> Result<()> {
    init_tracing();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    run(runtime.handle().clone());

    Ok(())
}

fn run(handle: Handle) {
    use winit::event::Event;

    let mut chunk_collection = chunk::ChunkCollection::new();

    let event_loop = winit::event_loop::EventLoop::new();
    let window = winit::window::Window::new(&event_loop).expect("Failed to create window");

    let mut render = handle.block_on(Render::new(&window));
    let mut spec = Spectator::new((40.0, 40.0, 40.0), 0.4, 0.4);
    let mut is_cursor_grabbed = false;
    event_loop.run(move |event, _, control_flow| match event {
        Event::WindowEvent { event, .. } => match event {
            WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
            WindowEvent::Resized(size) => render.resize(size),
            WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                render.resize(*new_inner_size)
            }
            WindowEvent::KeyboardInput { input, .. } => {
                if input.state != ElementState::Pressed {
                    return;
                }
                if input.virtual_keycode.is_none() {
                    return;
                }

                info!(?input);
                let keycode = input.virtual_keycode.unwrap();
                match keycode {
                    VirtualKeyCode::Space => {
                        spec.update_eye((0.0, 0.05, 0.0));
                    }
                    VirtualKeyCode::LShift => {
                        spec.update_eye((0.0, -0.05, 0.0));
                    }
                    VirtualKeyCode::G => {
                        window.set_cursor_visible(is_cursor_grabbed);
                        window.set_cursor_grab(!is_cursor_grabbed).unwrap();
                        is_cursor_grabbed = !is_cursor_grabbed;
                    }
                    _ => {}
                }
            }
            _ => {}
        },
        Event::MainEventsCleared => {
            // re-render dirty subchunks
            re_render_chunks(&mut chunk_collection, &mut render);

            render.set_view_matrix(spec.view_matrix());
            render.update();

            info!("Rendering frame");
            let render_result = handle.block_on(render.render());
            match render_result {
                Ok(_) => {}
                Err(SurfaceError::Lost | SurfaceError::Outdated) => render.resize(render.size()),
                Err(SurfaceError::OutOfMemory) => *control_flow = ControlFlow::Exit,
                Err(SurfaceError::Timeout) => warn!("Surface timeout"),
            }
        }
        Event::DeviceEvent { event, .. } => match event {
            winit::event::DeviceEvent::MouseMotion { delta: (x, y) } => {
                spec.update_yaw(x as f32 * 0.01);
                spec.update_pitch(y as f32 * -0.01);
            }
            _ => {}
        },
        _ => {}
    });
}

fn init_tracing() {
    use std::str::FromStr;
    use tracing_subscriber::*;

    const PKG_NAME: &str = env!("CARGO_PKG_NAME");
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            let pkg_name = PKG_NAME.replace("-", "_");
            EnvFilter::from_str(&format!("warn,{pkg_name}=info"))
                .expect("Failed to parse env-filter string")
        }))
        .init();
}

fn re_render_chunks(chunk_collection: &mut chunk::ChunkCollection, render: &mut render::Render) {
    let coords = chunk_collection.loaded_chunk_coordinates();
    for (cx, cz) in coords {
        for s in 0..16 {
            re_render_subchunk(chunk_collection, render, (cx, cz), s);
        }
    }
}

fn re_render_subchunk(
    chunk_collection: &mut chunk::ChunkCollection,
    render: &mut render::Render,
    (cx, cz): (i64, i64),
    s: usize,
) {
    let is_dirty = chunk_collection.get_chunk((cx, cz)).is_subchunk_dirty(s);
    if is_dirty == false {
        return;
    }
    chunk_collection
        .get_chunk_mut((cx, cz))
        .unmark_subchunk_dirty(s);
    info!("Re-rendering chunk at (cx = {cx}, cz = {cz})");

    // redraw the subchunk at (cx, s, cz)
    let mut buffer = render::RenderedBuffer::new();

    let x_start = cx * 16;
    let y_start = s as i64 * 16;
    let z_start = cz * 16;

    let x_end = x_start + 16;
    let y_end = y_start + 16;
    let z_end = z_start + 16;

    for (x, y, z) in iproduct!(x_start..x_end, y_start..y_end, z_start..z_end) {
        let block = match chunk_collection.get_block((x, y, z)) {
            chunk::GetBlockOutput::Loaded(block) => block,
            chunk::GetBlockOutput::Unloaded => continue,
        };
        if block.is_opaque() == false {
            continue;
        }

        let sx = x.rem_euclid(16);
        let sy = y.rem_euclid(16);
        let sz = z.rem_euclid(16);

        let above_block = chunk_collection.get_block((x, y + 1, z));
        if let chunk::GetBlockOutput::Loaded(above_block) = above_block {
            if above_block.is_opaque() == false {
                buffer.push_face(render::TOP_FACE, (sx, sy, sz));
            }
        }

        let below_block = chunk_collection.get_block((x, y - 1, z));
        if let chunk::GetBlockOutput::Loaded(below_block) = below_block {
            if below_block.is_opaque() == false {
                buffer.push_face(render::BOTTOM_FACE, (sx, sy, sz));
            }
        }

        let right_block = chunk_collection.get_block((x + 1, y, z));
        if let chunk::GetBlockOutput::Loaded(right_block) = right_block {
            if right_block.is_opaque() == false {
                buffer.push_face(render::RIGHT_FACE, (sx, sy, sz));
            }
        }

        let left_block = chunk_collection.get_block((x - 1, y, z));
        if let chunk::GetBlockOutput::Loaded(left_block) = left_block {
            if left_block.is_opaque() == false {
                buffer.push_face(render::LEFT_FACE, (sx, sy, sz));
            }
        }

        let front_block = chunk_collection.get_block((x, y, z + 1));
        if let chunk::GetBlockOutput::Loaded(front_block) = front_block {
            if front_block.is_opaque() == false {
                buffer.push_face(render::FRONT_FACE, (sx, sy, sz));
            }
        }

        let rear_block = chunk_collection.get_block((x, y, z - 1));
        if let chunk::GetBlockOutput::Loaded(rear_block) = rear_block {
            if rear_block.is_opaque() == false {
                buffer.push_face(render::REAR_FACE, (sx, sy, sz));
            }
        }
    }

    render.insert_rendered((cx, s as i64, cz), buffer);
}

#[derive(Debug)]
struct Spectator {
    /// The view position.
    eye: Vec3,
    /// Pitch (up-down rotation axis of head), `0` at the eye level, positive down, in radians.
    pitch: f32,
    /// Yaw (horizontal rotation axis of head), `0` towards east, clockwise.
    yaw: f32,
}

impl Spectator {
    fn new(eye: impl Into<Vec3>, pitch: f32, yaw: f32) -> Self {
        Self {
            eye: eye.into(),
            pitch,
            yaw,
        }
    }

    fn update_pitch(&mut self, delta: f32) {
        self.pitch += delta;
        self.pitch = self
            .pitch
            .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
    }

    fn update_yaw(&mut self, delta: f32) {
        self.yaw += delta;
        self.yaw = self.yaw.rem_euclid(std::f32::consts::PI * 2.0);
    }

    fn update_eye(&mut self, delta: impl Into<Vec3>) {
        self.eye += delta.into();
    }

    fn view_matrix(&self) -> Mat4 {
        info!(?self);

        let look_direction = vec3(f32::cos(self.yaw), f32::sin(self.pitch), f32::sin(self.yaw));
        let look_point = self.eye + look_direction;

        const UP: Vec3 = vec3(0.0, 1.0, 0.0);
        Mat4::look_at_rh(self.eye, look_point, UP)
    }
}
