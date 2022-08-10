use glam::{vec3, Mat4, Vec3};
use itertools::iproduct;
use render::Render;
use tokio::{runtime::Handle, sync::mpsc};
use tracing::{info, warn};
use wgpu::SurfaceError;
use winit::{
    event::{ElementState, VirtualKeyCode, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::Window,
};

use wgpu_block_shared::protocol::{ClientMessage, ServerMessage};

mod chunk;
mod render;

use chunk::{ChunkCollection, MaybeLoadedBlock};
use render::Vertex;

pub fn run(
    handle: Handle,
    (out_tx, in_rx): (
        mpsc::UnboundedSender<ClientMessage>,
        crossbeam_channel::Receiver<ServerMessage>,
    ),
) {
    use winit::event::Event;

    let mut chunk_collection = ChunkCollection::new();

    let event_loop = EventLoop::new();
    let window = Window::new(&event_loop).expect("Failed to create window");

    let mut render = handle.block_on(Render::new(&window));
    let mut spec = Spectator::new((40.0, 40.0, 40.0), 0.4, 0.4);
    let mut is_cursor_grabbed = false;

    use WindowEvent::*;
    let mut frame = 0;
    event_loop.run(move |event, _, control_flow| match event {
        Event::WindowEvent { event, .. } => match event {
            CloseRequested => *control_flow = ControlFlow::Exit,
            Resized(size) => render.resize(size),
            ScaleFactorChanged { new_inner_size, .. } => render.resize(*new_inner_size),
            KeyboardInput { input, .. } => {
                if input.state != ElementState::Pressed {
                    return;
                }
                if input.virtual_keycode.is_none() {
                    return;
                }

                info!(?input);
                let keycode = input.virtual_keycode.unwrap();

                use VirtualKeyCode::*;
                match keycode {
                    Space => {
                        spec.update_eye((0.0, 0.05, 0.0));
                    }
                    LShift => {
                        spec.update_eye((0.0, -0.05, 0.0));
                    }
                    G => {
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
            frame += 1;

            // consume [`ServerMessage`]s
            for server_msg in in_rx.try_iter() {
                use ServerMessage::*;
                match server_msg {
                    SetClientInfo { uuid } => {
                        info!("Login success with client uuid {uuid}");
                    }
                    Pong { data } => {
                        info!("Received pong from server: {data}");
                    }
                    LoadChunk { cx, cz, chunk } => {
                        chunk_collection.load_chunk((cx, cz), chunk);
                        info!("Loaded chunk at ({cx}, {cz})");
                    }
                    UnloadChunk { cx, cz } => {
                        chunk_collection.unload_chunk((cx, cz));
                        info!("Unloaded chunk at ({cx}, {cz})");
                    }
                    UpdateBlock { x, y, z, block } => {
                        chunk_collection.set_block((x, y, z), block);
                        info!("Updated block at ({x}, {y}, {z}) to {block:?}");
                    }
                    server_msg => {
                        warn!("Unknown server message {server_msg:?}")
                    }
                }
            }

            // tell the server my position
            if frame % 60 == 0 {
                out_tx
                    .send(ClientMessage::SetPlayerPos {
                        eye: (spec.eye.x, spec.eye.y, spec.eye.z),
                        pitch: spec.pitch,
                        yaw: spec.yaw,
                    })
                    .expect("Failed to send message to out_tx");
            }

            // re-render dirty subchunks
            re_render_chunks(&mut chunk_collection, &mut render);

            render.set_view_matrix(spec.view_matrix());
            render.update();

            // render the frame on screen
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

/// Re-render all chunks that are **loaded** and **dirty**.
fn re_render_chunks(chunk_collection: &mut ChunkCollection, render: &mut render::Render) {
    let coords = chunk_collection.loaded_chunk_coordinates();
    for (cx, cz) in coords {
        for s in 0..16 {
            let is_dirty = chunk_collection.get_chunk((cx, cz)).is_subchunk_dirty(s);
            if is_dirty == false {
                continue;
            }

            re_render_subchunk(chunk_collection, render, (cx, cz), s);

            chunk_collection
                .get_chunk_mut((cx, cz))
                .unmark_subchunk_dirty(s);
        }
    }
}

/// Re-render subchunks at chunk `(cx, s, cz)`.
fn re_render_subchunk(
    chunk_collection: &mut ChunkCollection,
    render: &mut render::Render,
    (cx, cz): (i64, i64),
    s: usize,
) {
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
        use chunk::MaybeLoadedBlock::*;

        let block = match chunk_collection.get_block((x, y, z)) {
            Loaded(block) => block,
            Unloaded => continue,
        };
        if block.is_opaque() == false {
            continue;
        }

        let sx = x.rem_euclid(16);
        let sy = y.rem_euclid(16);
        let sz = z.rem_euclid(16);

        // Storage for the blocks nearby
        let nearbys = NearbyBlocks::new((x, y, z), chunk_collection);
        let opaque_count_of_face = |face: [Vertex; 4]| {
            face.map(Vertex::pos_i64)
                .map(|(vx, vy, vz)| nearbys.opaque_count((vx, vy, vz)))
        };

        if let Loaded(block) = nearbys.at((0, 1, 0)) {
            if block.is_opaque() == false {
                let opaque_counts = opaque_count_of_face(render::TOP_FACE);
                buffer._push_face(render::TOP_FACE, opaque_counts, (sx, sy, sz));
            }
        }

        if let Loaded(below_block) = nearbys.at((0, -1, 0)) {
            if below_block.is_opaque() == false {
                let opaque_counts = opaque_count_of_face(render::BOTTOM_FACE);
                buffer._push_face(render::BOTTOM_FACE, opaque_counts, (sx, sy, sz));
            }
        }

        if let Loaded(right_block) = nearbys.at((1, 0, 0)) {
            if right_block.is_opaque() == false {
                let opaque_counts = opaque_count_of_face(render::RIGHT_FACE);
                buffer._push_face(render::RIGHT_FACE, opaque_counts, (sx, sy, sz));
            }
        }

        if let Loaded(left_block) = nearbys.at((-1, 0, 0)) {
            if left_block.is_opaque() == false {
                let opaque_counts = opaque_count_of_face(render::LEFT_FACE);
                buffer._push_face(render::LEFT_FACE, opaque_counts, (sx, sy, sz));
            }
        }

        if let Loaded(front_block) = nearbys.at((0, 0, 1)) {
            if front_block.is_opaque() == false {
                let opaque_counts = opaque_count_of_face(render::FRONT_FACE);
                buffer._push_face(render::FRONT_FACE, opaque_counts, (sx, sy, sz));
            }
        }

        if let Loaded(rear_block) = nearbys.at((0, 0, -1)) {
            if rear_block.is_opaque() == false {
                let opaque_counts = opaque_count_of_face(render::REAR_FACE);
                buffer._push_face(render::REAR_FACE, opaque_counts, (sx, sy, sz));
            }
        }
    }

    render.insert_rendered((cx, s as i64, cz), buffer);
}

/// Blocks within a 3x3x3 region around a center block.
struct NearbyBlocks {
    blocks: [[[MaybeLoadedBlock; 3]; 3]; 3],
    opaques: [[[bool; 3]; 3]; 3],
}

impl NearbyBlocks {
    fn new((x, y, z): (i64, i64, i64), chunk_collection: &ChunkCollection) -> Self {
        let mut blocks = [[[MaybeLoadedBlock::Unloaded; 3]; 3]; 3];
        for (dx, dy, dz) in iproduct!(-1..=1, -1..=1, -1..=1) {
            blocks[(dx + 1) as usize][(dy + 1) as usize][(dz + 1) as usize] =
                chunk_collection.get_block((x + dx, y + dy, z + dz));
        }

        let opaques = blocks.map(|b| {
            b.map(|c| {
                c.map(|block| match block {
                    MaybeLoadedBlock::Loaded(block) => block.is_opaque(),
                    MaybeLoadedBlock::Unloaded => false,
                })
            })
        });
        Self { blocks, opaques }
    }

    fn at(&self, (dx, dy, dz): (i64, i64, i64)) -> MaybeLoadedBlock {
        self.blocks[(dx + 1) as usize][(dy + 1) as usize][(dz + 1) as usize]
    }

    /// Get the number of opaque blocks at the corner `(vx, vy, vz)`, specified in vertex
    /// coordinates on the centeral unit block.
    fn opaque_count(&self, (vx, vy, vz): (i64, i64, i64)) -> u8 {
        // The filter (i.e. the unit block) is 2x2x2, while the input (i.e. the nearbys) is 3x3x3.
        // This is like a 3d convolution with a 2x2x2 filter of all 1's.
        iproduct!(vx..=(vx + 1), vy..=(vy + 1), vz..=(vz + 1))
            .map(|(dx, dy, dz)| self.opaques[dx as usize][dy as usize][dz as usize])
            .filter(|b| *b)
            .count() as u8
    }
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
        let look_direction = vec3(f32::cos(self.yaw), f32::sin(self.pitch), f32::sin(self.yaw));
        let look_point = self.eye + look_direction;

        const UP: Vec3 = vec3(0.0, 1.0, 0.0);
        Mat4::look_at_rh(self.eye, look_point, UP)
    }
}
