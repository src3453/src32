use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpt32::devices::sgu::s3w2::{PCM_RAM_SIZE, S3w2Sound};
use cpt32::music::{gui::TrackerUi, project::Song};
use imgui_wgpu::{Renderer, RendererConfig};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, Event, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{ModifiersState, PhysicalKey};
use winit::window::{Window, WindowId};

#[path = "../audio.rs"]
mod audio;

const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

struct EditorWindow {
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    imgui: imgui::Context,
    platform: WinitPlatform,
    renderer: Renderer,
    last_frame: Instant,
    render_error: Option<String>,
}

impl EditorWindow {
    fn new(window: Arc<Window>, display: OwnedDisplayHandle) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(display),
        ));
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| format!("Cannot create editor surface: {error}"))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("Cannot initialize GPU runtime: {error}"))?;
        let (adapter, device, queue) = runtime.block_on(async {
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await
                .map_err(|error| format!("Cannot find GPU adapter: {error}"))?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("Music Editor Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                    trace: wgpu::Trace::default(),
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                })
                .await
                .map_err(|error| format!("Cannot create GPU device: {error}"))?;
            Ok::<_, String>((adapter, device, queue))
        })?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| caps.formats.first().copied())
            .ok_or("GPU surface has no supported formats")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: *caps
                .alpha_modes
                .first()
                .ok_or("GPU surface has no alpha mode")?,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let mut imgui = imgui::Context::create();
        imgui.set_ini_filename(None);
        let mut platform = WinitPlatform::new(&mut imgui);
        platform.attach_window(imgui.io_mut(), &window, HiDpiMode::Default);
        let renderer = Renderer::new(
            &mut imgui,
            &device,
            &queue,
            RendererConfig {
                texture_format: format,
                ..RendererConfig::default()
            },
        );
        Ok(Self {
            surface,
            window,
            device,
            queue,
            config,
            size,
            imgui,
            platform,
            renderer,
            last_frame: Instant::now(),
            render_error: None,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.size = size;
        if size.width != 0 && size.height != 0 {
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn draw(&mut self, tracker: &mut TrackerUi) {
        if self.size.width == 0 || self.size.height == 0 {
            return;
        }
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.resize(self.window.inner_size());
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Validation => {
                self.render_error = Some("GPU surface validation error".into());
                return;
            }
        };
        let now = Instant::now();
        self.imgui.io_mut().update_delta_time(now - self.last_frame);
        self.last_frame = now;
        if let Err(error) = self
            .platform
            .prepare_frame(self.imgui.io_mut(), &self.window)
        {
            self.render_error = Some(format!("Cannot prepare editor frame: {error}"));
            return;
        }
        let ui = self.imgui.frame();
        tracker.draw(ui);
        if let Some(message) = &self.render_error {
            ui.window("Rendering error")
                .always_auto_resize(true)
                .build(|| {
                    ui.text_wrapped(message);
                });
        }
        self.platform.prepare_render(ui, &self.window);
        let draw_data = self.imgui.render();
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Music Editor Encoder"),
            });
        let result = {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Music Editor ImGui Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.035,
                            g: 0.04,
                            b: 0.05,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.renderer
                .render(draw_data, &self.queue, &self.device, &mut pass)
        };
        if let Err(error) = result {
            self.render_error = Some(format!("Cannot render editor: {error}"));
            return;
        }
        self.queue.submit(Some(encoder.finish()));
        output.present();
    }
}

struct MusicEditor {
    tracker: TrackerUi,
    cores: [Rc<RefCell<S3w2Sound>>; 2],
    audio: audio::AudioHost,
    display: Option<OwnedDisplayHandle>,
    editor: Option<EditorWindow>,
    modifiers: ModifiersState,
    next_tick: Instant,
    sample_remainder: u32,
    fatal_error: Option<String>,
}

impl MusicEditor {
    fn new(project: Option<PathBuf>, display: OwnedDisplayHandle) -> Result<Self, String> {
        let audio = audio::AudioHost::new()?;
        let pcm_ram = Rc::new(RefCell::new(vec![0; PCM_RAM_SIZE]));
        let cores = std::array::from_fn(|_| {
            Rc::new(RefCell::new(S3w2Sound::with_pcm_ram(pcm_ram.clone())))
        });
        Ok(Self {
            tracker: TrackerUi::new(project),
            cores,
            audio,
            display: Some(display),
            editor: None,
            modifiers: ModifiersState::empty(),
            next_tick: Instant::now(),
            sample_remainder: 0,
            fatal_error: None,
        })
    }

    fn tick_audio(&mut self) {
        // Player register updates must precede synthesis on both shared-PCMRAM cores.
        self.tracker.advance_vsync(&self.cores);
        self.sample_remainder += self.audio.sample_rate();
        let samples = (self.sample_remainder / 60) as usize;
        self.sample_remainder %= 60;
        let (mut left, mut right) = self.cores[0].borrow_mut().clock_mixed(samples);
        let (left1, right1) = self.cores[1].borrow_mut().clock_mixed(samples);
        for (output, sample) in left.iter_mut().zip(left1) {
            *output = output.saturating_add(sample);
        }
        for (output, sample) in right.iter_mut().zip(right1) {
            *output = output.saturating_add(sample);
        }
        self.audio.push_samples_i16(&left, &right);
    }
}

impl ApplicationHandler for MusicEditor {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.editor.is_some() {
            return;
        }
        let result = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Music Editor")
                    .with_inner_size(LogicalSize::new(1440.0, 900.0))
                    .with_min_inner_size(LogicalSize::new(960.0, 640.0)),
            )
            .map_err(|error| format!("Cannot create editor window: {error}"))
            .and_then(|window| EditorWindow::new(Arc::new(window), self.display.take().unwrap()));
        match result {
            Ok(editor) => {
                self.editor = Some(editor);
                self.next_tick = Instant::now();
            }
            Err(error) => {
                self.fatal_error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.window.id() != id {
            return;
        }
        editor.platform.handle_event(
            editor.imgui.io_mut(),
            &editor.window,
            &Event::<()>::WindowEvent {
                window_id: id,
                event: event.clone(),
            },
        );
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(false) => self.modifiers = ModifiersState::empty(),
            WindowEvent::KeyboardInput { event, .. } => {
                if !editor.imgui.io().want_text_input {
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.tracker.handle_input(
                            key,
                            event.state == ElementState::Pressed,
                            self.modifiers.control_key(),
                            self.modifiers.shift_key(),
                            self.modifiers.alt_key(),
                        );
                    }
                }
            }
            WindowEvent::Resized(size) => editor.resize(size),
            WindowEvent::RedrawRequested => editor.draw(&mut self.tracker),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.editor.is_none() {
            return;
        }
        let now = Instant::now();
        if now >= self.next_tick {
            self.tick_audio();
            self.next_tick += FRAME_INTERVAL;
            // Do not enqueue a burst of stale sound after a stalled or dragged window.
            if self.next_tick <= now {
                self.next_tick = now + FRAME_INTERVAL;
            }
            if let Some(editor) = &self.editor {
                editor.window.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_tick));
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let project = args.next().map(PathBuf::from);
    if args.next().is_some() {
        return Err("Usage: music_editor [project.toml]".into());
    }
    if let Some(path) = &project {
        Song::load(path).map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
    }
    let event_loop =
        EventLoop::new().map_err(|error| format!("Cannot create event loop: {error}"))?;
    let mut editor = MusicEditor::new(project, event_loop.owned_display_handle())?;
    event_loop
        .run_app(&mut editor)
        .map_err(|error| format!("Editor event loop failed: {error}"))?;
    match editor.fatal_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Music Editor: {error}");
        std::process::exit(1);
    }
}
