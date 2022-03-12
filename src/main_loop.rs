//! Main (platform-specific) main loop which handles:
//! * Input (Mouse/Keyboard)
//! * Platform Events like suspend/resume
//! * Render a new frame

use log::{error, info, trace};
use winit::event::{ElementState, Event, KeyboardInput, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};

use crate::input::{InputController, UpdateState};

use crate::io::scheduler::IOScheduler;
use crate::platform::Instant;
use crate::render::render_state::RenderState;

pub async fn setup(
    window: winit::window::Window,
    event_loop: EventLoop<()>,
    mut workflow: Box<IOScheduler>,
) {
    info!("== mapr ==");

    let mut input = InputController::new(0.2, 100.0, 0.1);
    let mut maybe_state: Option<RenderState> = if cfg!(target_os = "android") {
        None
    } else {
        Some(RenderState::new(&window).await)
    };

    let mut last_render_time = Instant::now();

    event_loop.run(move |event, _, control_flow| {
        /* FIXME:   On Android we need to initialize the surface on Event::Resumed. On desktop this
                    event is not fired and we can do surface initialization anytime. Clean this up.
        */
        #[cfg(target_os = "android")]
        if maybe_state.is_none() && event == Event::Resumed {
            use tokio::runtime::Handle;
            use tokio::task;

            let state = task::block_in_place(|| {
                Handle::current().block_on(async { RenderState::new(&window).await })
            });
            maybe_state = Some(state);
            return;
        }

        if let Some(state) = maybe_state.as_mut() {
            match event {
                Event::DeviceEvent {
                    ref event,
                    .. // We're not using device_id currently
                } => {
                    trace!("{:?}", event);
                    input.device_input(event);
                }

                Event::WindowEvent {
                    ref event,
                    window_id,
                } if window_id == window.id() => {
                    if !input.window_input(event, state) {
                        match event {
                            WindowEvent::CloseRequested
                            | WindowEvent::KeyboardInput {
                                input:
                                KeyboardInput {
                                    state: ElementState::Pressed,
                                    virtual_keycode: Some(VirtualKeyCode::Escape),
                                    ..
                                },
                                ..
                            } => *control_flow = ControlFlow::Exit,
                            WindowEvent::Resized(physical_size) => {
                                state.resize(*physical_size);
                            }
                            WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                                // new_inner_size is &mut so w have to dereference it twice
                                state.resize(**new_inner_size);
                            }
                            _ => {}
                        }
                    }
                }
                Event::RedrawRequested(_) => {
                    let now = Instant::now();
                    let dt = now - last_render_time;
                    last_render_time = now;

                    workflow.populate_cache();

                    input.update_state(state, dt);
                    state.upload_tile_geometry(&mut workflow);
                    match state.render() {
                        Ok(_) => {}
                        Err(wgpu::SurfaceError::Lost) => {
                            error!("Surface Lost");
                        },
                        // The system is out of memory, we should probably quit
                        Err(wgpu::SurfaceError::OutOfMemory) => {
                            error!("Out of Memory");
                            *control_flow = ControlFlow::Exit;
                        },
                        // All other errors (Outdated, Timeout) should be resolved by the next frame
                        Err(e) => eprintln!("{:?}", e),
                    }
                }
                Event::Suspended => {
                    state.suspend();
                }
                Event::Resumed => {
                    state.recreate_surface(&window);
                    state.resize(window.inner_size()); // FIXME: Resumed is also called when the app launches for the first time. Instead of first using a "fake" inner_size() in State::new we should initialize with a proper size from the beginning
                    state.resume();
                }
                Event::MainEventsCleared => {
                    // RedrawRequested will only trigger once, unless we manually
                    // request it.
                    window.request_redraw();
                }
                _ => {}
            }
        }
    });
}
