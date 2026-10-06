// Main entry point for the CPT32 emulator

use std::cell::RefCell;
use std::env;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cpt32::bus::Bus;
use cpt32::cpu::{Cpu, InstructionMode};
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::pec::serial::connect_uart;
use cpt32::devices::ram::connect_ram;
use cpt32::devices::sgu::s3w2::S3w2Sound;
use cpt32::devices::sgu::sgu::connect_sgu;
use cpt32::devices::vdp::vdp::{connect_vdp_with_font, Vdp};
use imgui::{Condition, Ui};
use imgui_wgpu::{Renderer, RendererConfig};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::Event;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes};

mod audio;
mod monitor;
mod render;

const WIDTH: u32 = render::PRESENT_WIDTH;
const HEIGHT: u32 = render::PRESENT_HEIGHT;
const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

fn load_binary_data(path: &str, bus: &mut Bus) {
    // Utility used to load programs and datas into the bus memory
    let data = std::fs::read(path).expect("Failed to read binary file");
    for (i, byte) in data.iter().enumerate() {
        bus.write_u8(i as u32, *byte);
    }
}

struct GuiApp {
    cpu: Cpu,
    vdp: Rc<RefCell<Vdp>>,
    sgu: [Rc<RefCell<S3w2Sound>>; 2],
    audio_host: Option<audio::AudioHost>,
    display_handle: Option<OwnedDisplayHandle>,
    window: Option<Window>,
    presenter: Option<render::WgpuPresenter>,
    debug_gui: Option<DebugGui>,
    enable_debug_gui: bool,
    start_paused: bool,
    next_frame_deadline: Instant,
}

impl GuiApp {
    fn new(
        program_path: &str,
        font_path: Option<&str>,
        display_handle: OwnedDisplayHandle,
        enable_debug_gui: bool,
        start_paused: bool,
    ) -> Self {
        let mut bus = Bus::new();
        connect_ram(&mut bus);
        cpt32::bus::connect_bmc_dmac(&mut bus);
        connect_uart(&mut bus);
        connect_rng(&mut bus);
        let sgu = connect_sgu(&mut bus);

        load_binary_data(program_path, &mut bus);

        let vdp = connect_vdp_with_font(&mut bus, font_path.map(Path::new));

        let audio_host = match audio::AudioHost::new() {
            Ok(host) => Some(host),
            Err(e) => {
                eprintln!("Warning: Failed to initialize audio host: {}", e);
                None
            }
        };

        let cpu = Cpu::new(bus);
        Self {
            cpu,
            vdp,
            sgu,
            audio_host,
            display_handle: Some(display_handle),
            window: None,
            presenter: None,
            debug_gui: None,
            enable_debug_gui,
            start_paused,
            next_frame_deadline: Instant::now(),
        }
    }
}

struct DebugUiState {
    running: bool,
    visible_windows: [bool; DebugWindow::COUNT],
    cycles_per_frame_input: String,
    queued_steps: usize,
    disasm_follow_pc: bool,
    disasm_base_input: String,
    disasm_count_input: String,
    mem_base_input: String,
    mem_count_input: String,
}

#[derive(Clone, Copy)]
enum DebugWindow {
    Controls,
    Disassembly,
    Memory,
    Compositor,
}

impl DebugWindow {
    const ALL: [Self; Self::COUNT] = [
        Self::Controls,
        Self::Disassembly,
        Self::Memory,
        Self::Compositor,
    ];
    const COUNT: usize = 4;

    fn label(self) -> &'static str {
        match self {
            Self::Controls => "Debug Controls",
            Self::Disassembly => "Realtime Disassembly",
            Self::Memory => "Realtime Memory Monitor",
            Self::Compositor => "Compositor Debug",
        }
    }
}

impl DebugUiState {
    fn new(start_paused: bool) -> Self {
        Self {
            running: !start_paused,
            visible_windows: [false; DebugWindow::COUNT],
            cycles_per_frame_input: cpt32::cpu::CYCLES_PER_FRAME.to_string(),
            queued_steps: 0,
            disasm_follow_pc: true,
            disasm_base_input: "0x00000000".to_string(),
            disasm_count_input: "32".to_string(),
            mem_base_input: "0x00000000".to_string(),
            mem_count_input: "256".to_string(),
        }
    }

    fn parse_u32(value: &str, fallback: u32) -> u32 {
        let trimmed = value.trim();
        if let Some(hex) = trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
        {
            return u32::from_str_radix(hex, 16).unwrap_or(fallback);
        }
        trimmed.parse::<u32>().unwrap_or(fallback)
    }

    fn parse_usize(value: &str, fallback: usize, max: usize) -> usize {
        let parsed = value.trim().parse::<usize>().unwrap_or(fallback);
        parsed.clamp(1, max)
    }

    fn execute_cpu(&mut self, cpu: &mut Cpu) {
        if self.running {
            let cycles = Self::parse_usize(
                &self.cycles_per_frame_input,
                cpt32::cpu::CYCLES_PER_FRAME as usize,
                10_000_000,
            );
            cpu.run(cycles);
        } else if self.queued_steps > 0 {
            for _ in 0..self.queued_steps {
                if !cpu.step_once() {
                    break;
                }
            }
            self.queued_steps = 0;
        }
    }

    fn draw_windows(&mut self, ui: &Ui, cpu: &mut Cpu, vdp: &Rc<RefCell<Vdp>>) {
        ui.main_menu_bar(|| {
            ui.menu("Windows", || {
                for window in DebugWindow::ALL {
                    let index = window as usize;
                    if ui
                        .menu_item_config(window.label())
                        .selected(self.visible_windows[index])
                        .build()
                    {
                        let visible = &mut self.visible_windows[index];
                        *visible = !*visible;
                    }
                }
            });
        });

        if self.visible_windows[DebugWindow::Controls as usize] {
            self.draw_controls(ui, cpu);
        }
        if self.visible_windows[DebugWindow::Disassembly as usize] {
            self.draw_disassembly(ui, cpu);
        }
        if self.visible_windows[DebugWindow::Memory as usize] {
            self.draw_memory(ui, cpu);
        }
        if self.visible_windows[DebugWindow::Compositor as usize] {
            self.draw_compositor(ui, vdp);
        }
    }

    fn draw_compositor(&mut self, ui: &Ui, vdp: &Rc<RefCell<Vdp>>) {
        let info = vdp.borrow().compositor_debug_info();
        ui.window("Compositor Debug")
            .size([620.0, 460.0], Condition::FirstUseEver)
            .build(|| {
                ui.text(format!(
                    "Display: {:?} | {} | Output: {}x{}",
                    info.display_mode,
                    if info.enabled { "enabled" } else { "disabled" },
                    info.width,
                    info.height
                ));
                ui.text("Composition order: GP0 (back) to GP7 (front)");
                ui.separator();
                for (gp, components) in info.gp_components.iter().enumerate() {
                    ui.text(format!("GP{gp}"));
                    if components.is_empty() {
                        ui.same_line();
                        ui.text("No configured image source");
                    } else {
                        for component in components {
                            ui.bullet_text(component);
                        }
                    }
                }
            });
    }

    fn draw_controls(&mut self, ui: &Ui, cpu: &mut Cpu) {
        ui.window("Debug Controls")
            .size([520.0, 360.0], Condition::FirstUseEver)
            .build(|| {
                ui.checkbox("Run CPU (Realtime)", &mut self.running);
                ui.input_text("Cycles / frame", &mut self.cycles_per_frame_input)
                    .build();
                if ui.button("Pause") {
                    self.running = false;
                }
                ui.same_line();
                if ui.button("Run") {
                    self.running = true;
                }
                if ui.button("Step 1 instruction") {
                    self.running = false;
                    self.queued_steps = self.queued_steps.saturating_add(1);
                }
                ui.same_line();
                if ui.button("Step 10 instructions") {
                    self.running = false;
                    self.queued_steps = self.queued_steps.saturating_add(10);
                }
                ui.same_line();
                if ui.button("Step 100 instructions") {
                    self.running = false;
                    self.queued_steps = self.queued_steps.saturating_add(100);
                }
                ui.separator();
                ui.text(format!("CPU running: {}", cpu.is_running()));
                ui.text(format!("PC: 0x{:08X}", cpu.pc()));
                ui.text(format!("Mode: {:?}", cpu.instruction_mode()));
                ui.text(format!("Total cycles: {}", cpu.cycles()));

                for base in (0..32).step_by(4) {
                    ui.text(format!(
                        "R{:<2}=0x{:08X}   R{:<2}=0x{:08X}   R{:<2}=0x{:08X}   R{:<2}=0x{:08X}",
                        base,
                        cpu.read_reg(base),
                        base + 1,
                        cpu.read_reg(base + 1),
                        base + 2,
                        cpu.read_reg(base + 2),
                        base + 3,
                        cpu.read_reg(base + 3),
                    ));
                }
            });
    }

    fn draw_disassembly(&mut self, ui: &Ui, cpu: &mut Cpu) {
        ui.window("Realtime Disassembly")
            .size([780.0, 420.0], Condition::FirstUseEver)
            .build(|| {
                ui.checkbox("Follow PC", &mut self.disasm_follow_pc);
                ui.input_text("Address", &mut self.disasm_base_input)
                    .build();
                ui.input_text("Line count", &mut self.disasm_count_input)
                    .build();
                ui.separator();

                let base = if self.disasm_follow_pc {
                    cpu.pc()
                } else {
                    Self::parse_u32(&self.disasm_base_input, cpu.pc())
                };
                let count = Self::parse_usize(&self.disasm_count_input, 16, 256);
                let mut addr = base;
                let mut mode = if base == cpu.pc() {
                    cpu.instruction_mode()
                } else {
                    InstructionMode::Normal
                };
                for _ in 0..count {
                    let marker = if addr == cpu.pc() { "=>" } else { "  " };
                    let decoded = cpu.decode_at(addr, mode);
                    if decoded.size == 2 {
                        let raw = cpu.read_u16_be(addr);
                        ui.text(format!(
                            "{} 0x{:08X}: {:04X}      {}",
                            marker, addr, raw, decoded.text
                        ));
                    } else {
                        let raw = cpu.read_u32(addr);
                        ui.text(format!(
                            "{} 0x{:08X}: {:08X}  {}",
                            marker, addr, raw, decoded.text
                        ));
                    }
                    addr = addr.wrapping_add(decoded.size as u32);
                    mode = decoded.next_mode;
                }
            });
    }

    fn draw_memory(&mut self, ui: &Ui, cpu: &mut Cpu) {
        ui.window("Realtime Memory Monitor")
            .size([780.0, 320.0], Condition::FirstUseEver)
            .build(|| {
                ui.input_text("Base address", &mut self.mem_base_input)
                    .build();
                ui.input_text("Byte count", &mut self.mem_count_input)
                    .build();
                ui.separator();

                let base = Self::parse_u32(&self.mem_base_input, 0);
                let count = Self::parse_usize(&self.mem_count_input, 64, 512);
                let mut row_addr = base;
                for _ in 0..count.div_ceil(16) {
                    let mut hex = String::new();
                    let mut ascii = String::new();
                    for col in 0..16 {
                        let absolute = row_addr.wrapping_add(col as u32);
                        let current_offset = absolute.wrapping_sub(base) as usize;
                        if current_offset >= count {
                            hex.push_str("   ");
                            ascii.push(' ');
                            continue;
                        }
                        match cpu.read_debug_mem_u8(absolute) {
                            Some(byte) => {
                                hex.push_str(&format!("{:02X} ", byte));
                                if byte.is_ascii_graphic() || byte == b' ' {
                                    ascii.push(byte as char);
                                } else {
                                    ascii.push('.');
                                }
                            }
                            None => {
                                hex.push_str("xx ");
                                ascii.push('.');
                            }
                        }
                    }
                    ui.text(format!("0x{:08X}: {}|{}|", row_addr, hex, ascii));
                    row_addr = row_addr.wrapping_add(16);
                }
            });
    }
}

struct DebugGui {
    imgui: imgui::Context,
    platform: WinitPlatform,
    renderer: Renderer,
    state: DebugUiState,
    last_frame: Instant,
}

impl DebugGui {
    fn new(window: &Window, presenter: &render::WgpuPresenter, start_paused: bool) -> Self {
        let mut imgui = imgui::Context::create();
        imgui.set_ini_filename(None);
        let mut platform = WinitPlatform::new(&mut imgui);
        platform.attach_window(imgui.io_mut(), window, HiDpiMode::Default);
        let renderer = Renderer::new(
            &mut imgui,
            presenter.device(),
            presenter.queue(),
            RendererConfig {
                texture_format: presenter.surface_format(),
                ..RendererConfig::default()
            },
        );

        Self {
            imgui,
            platform,
            renderer,
            state: DebugUiState::new(start_paused),
            last_frame: Instant::now(),
        }
    }

    fn handle_window_event(
        &mut self,
        window: &Window,
        window_id: winit::window::WindowId,
        event: &WindowEvent,
    ) {
        let wrapped_event = Event::<()>::WindowEvent {
            window_id,
            event: event.clone(),
        };
        self.platform
            .handle_event(self.imgui.io_mut(), window, &wrapped_event);
    }

    fn render_frame(
        &mut self,
        window: &Window,
        cpu: &mut Cpu,
        vdp: &Rc<RefCell<Vdp>>,
        presenter: &mut render::WgpuPresenter,
    ) -> Result<(), render::FrameError> {
        self.state.execute_cpu(cpu);

        let now = Instant::now();
        self.imgui.io_mut().update_delta_time(now - self.last_frame);
        self.last_frame = now;
        self.platform
            .prepare_frame(self.imgui.io_mut(), window)
            .map_err(|err| {
                render::FrameError::Overlay(format!("ImGui frame prepare failed: {err}"))
            })?;

        let ui = self.imgui.frame();
        self.state.draw_windows(ui, cpu, vdp);
        self.platform.prepare_render(ui, window);
        let draw_data = self.imgui.render();

        presenter.render_with_overlay(vdp, |device, queue, encoder, surface_view| {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("CPT32 ImGui Overlay Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: surface_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            self.renderer
                .render(draw_data, queue, device, &mut render_pass)
                .map_err(|err| render::FrameError::Overlay(format!("ImGui render failed: {err}")))
        })
    }
}

impl ApplicationHandler for GuiApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes: WindowAttributes = Window::default_attributes()
            .with_title("CPT32")
            .with_inner_size(LogicalSize::new((WIDTH) as f64, (HEIGHT) as f64))
            .with_min_inner_size(LogicalSize::new(WIDTH as f64, HEIGHT as f64));

        let window = event_loop
            .create_window(attributes)
            .expect("Failed to create window");
        let display_handle = self
            .display_handle
            .take()
            .expect("Missing display handle for rendering");
        let presenter = render::WgpuPresenter::new(&window, Box::new(display_handle));
        let debug_gui = if self.enable_debug_gui {
            Some(DebugGui::new(&window, &presenter, self.start_paused))
        } else {
            None
        };

        self.presenter = Some(presenter);
        self.debug_gui = debug_gui;
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let (Some(window), Some(debug_gui)) = (self.window.as_ref(), self.debug_gui.as_mut()) {
            debug_gui.handle_window_event(window, window_id, &event);
        }

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
                let now = Instant::now();
                if now < self.next_frame_deadline {
                    return;
                }
                self.next_frame_deadline = now + FRAME_INTERVAL;
                self.vdp.borrow_mut().tick();
                self.cpu.set_irq_source(0, true);
                self.cpu.set_irq_source(0, false);
                if let (Some(window), Some(presenter)) =
                    (self.window.as_ref(), self.presenter.as_mut())
                {
                    let render_result = if let Some(debug_gui) = self.debug_gui.as_mut() {
                        debug_gui.render_frame(window, &mut self.cpu, &self.vdp, presenter)
                    } else {
                        self.cpu.run(cpt32::cpu::CYCLES_PER_FRAME as usize);
                        presenter.render(&self.vdp)
                    };

                    if let Some(audio_host) = self.audio_host.as_ref() {
                        let sample_count =
                            (audio_host.sample_rate() / cpt32::sys::FRAME_RATE) as usize;
                        let (left0, right0) = self.sgu[0].borrow_mut().clock_mixed(sample_count);
                        let (left1, right1) = self.sgu[1].borrow_mut().clock_mixed(sample_count);
                        let left: Vec<i16> = left0
                            .into_iter()
                            .zip(left1)
                            .map(|(a, b)| a.saturating_add(b))
                            .collect();
                        let right: Vec<i16> = right0
                            .into_iter()
                            .zip(right1)
                            .map(|(a, b)| a.saturating_add(b))
                            .collect();
                        audio_host.push_samples_i16(&left, &right);
                    }

                    match render_result {
                        Ok(()) => {}
                        Err(render::FrameError::Lost) | Err(render::FrameError::Outdated) => {
                            presenter.resize(window.inner_size())
                        }
                        Err(render::FrameError::Timeout) | Err(render::FrameError::Occluded) => {}
                        Err(render::FrameError::Validation) => {
                            eprintln!("Render validation error")
                        }
                        Err(render::FrameError::Overlay(message)) => {
                            eprintln!("{}", message)
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_frame_deadline {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame_deadline));
    }
}

struct GuiLaunchOptions {
    program_path: String,
    font_path: Option<String>,
    enable_debug_gui: bool,
    start_paused: bool,
}

fn parse_gui_args(args: &[String]) -> Result<GuiLaunchOptions, String> {
    let mut positional = Vec::new();
    let mut enable_debug_gui = false;
    let mut start_paused = false;

    for arg in args {
        match arg.as_str() {
            "--debug-gui" | "--imgui-debug" => enable_debug_gui = true,
            "--start-paused" | "--pause-on-start" => start_paused = true,
            _ if arg.starts_with('-') => {
                return Err(format!("Unknown option: {arg}"));
            }
            _ => positional.push(arg.clone()),
        }
    }

    if positional.is_empty() {
        return Err("Missing program path".to_string());
    }
    if positional.len() > 2 {
        return Err("Too many positional arguments".to_string());
    }

    Ok(GuiLaunchOptions {
        program_path: positional[0].clone(),
        font_path: positional.get(1).cloned(),
        enable_debug_gui,
        start_paused,
    })
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let first = args.first().map(String::as_str);

    match first {
        Some("monitor") | Some("-m") | Some("--monitor") => {
            monitor::run(args.get(1).map(String::as_str));
        }
        Some(_) => match parse_gui_args(&args) {
            Ok(options) => {
                if options.start_paused && !options.enable_debug_gui {
                    eprintln!("Warning: --start-paused requires --debug-gui and will be ignored.");
                }
                run_gui_with_options(&options)
            }
            Err(message) => {
                eprintln!("Error: {}", message);
                eprintln!(
                    "Usage: cargo run -- <program.bin> [font.bin] [--debug-gui] [--start-paused] | cargo run -- [-m|--monitor] [program.bin]"
                );
            }
        },
        None => {
            eprintln!(
                "Usage: cargo run -- <program.bin> [font.bin] [--debug-gui] [--start-paused] | cargo run -- [-m|--monitor] [program.bin]"
            );
        }
    }
}

fn run_gui_with_options(options: &GuiLaunchOptions) {
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let display_handle = event_loop.owned_display_handle();
    let mut app = GuiApp::new(
        &options.program_path,
        options.font_path.as_deref(),
        display_handle,
        options.enable_debug_gui,
        options.enable_debug_gui && options.start_paused,
    );
    event_loop
        .run_app(&mut app)
        .expect("Failed to run application");
}
