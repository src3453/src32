// Main entry point for the CPT32 emulator

use std::env;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use cpt32::bus::Bus;
use cpt32::cpu::Cpu;
use cpt32::devices::pec::serial::connect_uart;
use cpt32::devices::vdp::vdp::connect_vdp_with_font;
use cpt32::devices::ram::connect_ram;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes};

mod monitor;
mod render;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;

fn load_binary_data(path: &str, bus: &mut Bus) {
    // Utility used to load programs and datas into the bus memory
    let data = std::fs::read(path).expect("Failed to read binary file");
    for (i, byte) in data.iter().enumerate() {
        bus.write_u8(i as u32, *byte);
    }
}

struct GuiApp {
    cpu: Cpu,
    vdp: Rc<RefCell<cpt32::devices::vdp::vdp::Vdp>>,
    display_handle: Option<OwnedDisplayHandle>,
    window: Option<Window>,
    presenter: Option<render::WgpuPresenter>,
}

impl GuiApp {
    fn new(program_path: &str, font_path: Option<&str>, display_handle: OwnedDisplayHandle) -> Self {
        let mut bus = Bus::new();
        connect_ram(&mut bus);
        connect_uart(&mut bus);

        load_binary_data(program_path, &mut bus);

        let vdp = connect_vdp_with_font(&mut bus, font_path.map(Path::new));

        let cpu = Cpu::new(bus);
        Self {
            cpu,
            vdp,
            display_handle: Some(display_handle),
            window: None,
            presenter: None,
        }
    }
}

impl ApplicationHandler for GuiApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes: WindowAttributes = Window::default_attributes()
            .with_title("CPT32")
            .with_inner_size(LogicalSize::new((WIDTH * 2) as f64, (HEIGHT * 2) as f64));

        let window = event_loop
            .create_window(attributes)
            .expect("Failed to create window");
        let display_handle = self
            .display_handle
            .take()
            .expect("Missing display handle for rendering");
        let presenter = render::WgpuPresenter::new(&window, Box::new(display_handle));

        self.presenter = Some(presenter);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(presenter) = self.presenter.as_mut() {
                    presenter.resize(size);
                }
            }
            WindowEvent::RedrawRequested => {
                self.vdp.borrow_mut().tick();
                self.cpu.run(cpt32::cpu::CYCLES_PER_FRAME as usize);

                if let (Some(window), Some(presenter)) = (self.window.as_ref(), self.presenter.as_mut()) {
                    match presenter.render(&self.vdp) {
                        Ok(()) => {}
                        Err(render::FrameError::Lost) | Err(render::FrameError::Outdated) => {
                            presenter.resize(window.inner_size())
                        }
                        Err(render::FrameError::Timeout) | Err(render::FrameError::Occluded) => {}
                        Err(render::FrameError::Validation) => {
                            eprintln!("Render validation error")
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn run_gui(program_path: &str, font_path: Option<&str>) {
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let display_handle = event_loop.owned_display_handle();
    let mut app = GuiApp::new(program_path, font_path, display_handle);
    event_loop
        .run_app(&mut app)
        .expect("Failed to run application");
}

fn main() {
    let mut args = env::args().skip(1);
    let first = args.next();

    match first.as_deref() {
        Some("monitor") | Some("-m") | Some("--monitor") => {
            monitor::run(args.next().as_deref());
        }
        Some(program_path) => run_gui(program_path, args.next().as_deref()),
        None => {
            eprintln!("Usage: cargo run -- <program.bin> [font.bin] | cargo run -- [-m|--monitor] [program.bin]");
        }
    }
}