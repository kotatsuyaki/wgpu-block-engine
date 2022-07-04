use anyhow::Result;
use render::Render;
use tokio::runtime::Handle;
use tracing::warn;
use wgpu::SurfaceError;
use winit::{event::WindowEvent, event_loop::ControlFlow};

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

    let event_loop = winit::event_loop::EventLoop::new();
    let window = winit::window::Window::new(&event_loop).expect("Failed to create window");

    let mut render = handle.block_on(Render::new(&window));
    event_loop.run(move |event, _, control_flow| match event {
        Event::WindowEvent { event, .. } => match event {
            WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
            WindowEvent::Resized(size) => render.resize(size),
            WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                render.resize(*new_inner_size)
            }
            _ => {}
        },
        Event::RedrawRequested(window_id) if window_id == window.id() => {
            render.update();
            let render_result = handle.block_on(render.render());
            match render_result {
                Ok(_) => {}
                Err(SurfaceError::Lost | SurfaceError::Outdated) => render.resize(render.size()),
                Err(SurfaceError::OutOfMemory) => *control_flow = ControlFlow::Exit,
                Err(SurfaceError::Timeout) => warn!("Surface timeout"),
            }
        }
        Event::RedrawEventsCleared => {
            window.request_redraw();
        }
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
