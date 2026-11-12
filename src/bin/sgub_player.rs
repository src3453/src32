//! SGUB Player: standalone SGUB stream player with a real-time playback monitor.
//!
//! Usage: `sgub_player [--sgu-version 1|2|2.1|3|3.1] [song.sgub]`. A file can
//! also be dropped onto the window.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpt32::devices::sgu::s3w2::{FIFO_CAPACITY, ModulationType, SguVersion, WaveformType};
use cpt32::music::monitor::{
    ChannelView, Engine, FRAME_SAMPLES, Meter, Transport, VOICES, modulation_tag, note_name,
    spectrum_db, waveform_tag,
};
use imgui::{Condition, DrawListMut, MouseButton, Ui, WindowFlags};
use imgui_wgpu::{Renderer, RendererConfig};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, Event, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowId};

#[allow(dead_code)]
#[path = "../audio.rs"]
mod audio;

const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);
const FAST_FACTOR: usize = 4;
const SEEK_FRAMES: u64 = 5 * 60;
const DESIGN_SIZE: [f32; 2] = [1328.0, 848.0];
const BANDS: usize = 36;

type Rgba = [f32; 4];
const BG: Rgba = [0.0, 0.0, 0.0, 1.0];
const PANEL: Rgba = [0.05, 0.06, 0.07, 1.0];
const DIM: Rgba = [0.32, 0.34, 0.36, 1.0];
const TEXT: Rgba = [0.86, 0.88, 0.9, 1.0];
const LABEL: Rgba = [0.55, 0.58, 0.62, 1.0];
const ACCENT: Rgba = [0.55, 0.85, 1.0, 1.0];
const KEY_ON: Rgba = [0.95, 0.12, 0.1, 1.0];
const METER_LO: Rgba = [0.75, 0.78, 0.8, 1.0];
const METER_HI: Rgba = [1.0, 0.35, 0.25, 1.0];

// ---------------------------------------------------------------------------
// Window / GPU plumbing (same setup as music_editor).

struct MonitorWindow {
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

impl MonitorWindow {
    fn new(window: Arc<Window>, display: OwnedDisplayHandle) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(display),
        ));
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| format!("Cannot create player surface: {error}"))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("Cannot initialize GPU runtime: {error}"))?;
        let (adapter, device, queue) = runtime.block_on(async {
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await
                .map_err(|error| format!("Cannot find GPU adapter: {error}"))?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("SGUB Player Device"),
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

    fn draw(&mut self, monitor: &mut Monitor) {
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
            self.render_error = Some(format!("Cannot prepare player frame: {error}"));
            return;
        }
        let ui = self.imgui.frame();
        monitor.draw(ui);
        self.platform.prepare_render(ui, &self.window);
        let draw_data = self.imgui.render();
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("SGUB Player Encoder"),
            });
        let result = {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SGUB Player ImGui Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
            self.render_error = Some(format!("Cannot render player: {error}"));
            return;
        }
        self.queue.submit(Some(encoder.finish()));
        output.present();
    }
}

// ---------------------------------------------------------------------------
// Monitor state and drawing.

struct Monitor {
    engine: Option<Engine>,
    sgu_version: SguVersion,
    file_name: String,
    status: String,
    fast: bool,
    animation: u32,
    spectrum: [Vec<f32>; 2],
    spectrum_peaks: [Vec<f32>; 2],
}

impl Monitor {
    fn new(sgu_version: SguVersion) -> Self {
        Self {
            engine: None,
            sgu_version,
            file_name: String::new(),
            status: "Drop an .sgub file onto this window".into(),
            fast: false,
            animation: 0,
            spectrum: [vec![-96.0; BANDS], vec![-96.0; BANDS]],
            spectrum_peaks: [vec![-96.0; BANDS], vec![-96.0; BANDS]],
        }
    }

    fn open(&mut self, path: &Path) {
        let result = std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| Engine::new(bytes, self.sgu_version).map_err(|e| e.to_string()));
        match result {
            Ok(engine) => {
                self.engine = Some(engine);
                self.file_name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.status = format!("Loaded {}", path.display());
                self.fast = false;
            }
            Err(error) => self.status = format!("Cannot open {}: {error}", path.display()),
        }
    }

    /// Advance one 60 Hz tick and return the samples to play.
    fn tick(&mut self, samples: usize) -> (Vec<i16>, Vec<i16>) {
        self.animation = self.animation.wrapping_add(1);
        let Some(engine) = self.engine.as_mut() else {
            return (vec![0; samples], vec![0; samples]);
        };
        if self.fast && engine.transport == Transport::Play {
            for _ in 1..FAST_FACTOR {
                engine.tick(FRAME_SAMPLES);
            }
        }
        let out = engine.tick(samples);
        if engine.transport == Transport::Play && engine.finished() {
            engine.transport = Transport::Stop;
        }
        for side in 0..2 {
            let bands = spectrum_db(engine.history(side), BANDS, 40.0, 20_000.0);
            for (band, value) in bands.into_iter().enumerate() {
                let shown = &mut self.spectrum[side][band];
                *shown = if value > *shown {
                    value
                } else {
                    (*shown - 2.5).max(value)
                };
                let peak = &mut self.spectrum_peaks[side][band];
                *peak = if *shown > *peak {
                    *shown
                } else {
                    (*peak - 0.6).max(*shown)
                };
            }
        }
        out
    }

    fn toggle_play(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            engine.transport = match engine.transport {
                Transport::Play => Transport::Pause,
                Transport::Pause => Transport::Play,
                Transport::Stop => {
                    if engine.finished() {
                        engine.seek(0);
                    }
                    Transport::Play
                }
            };
        }
    }

    fn stop(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            engine.stop();
        }
        self.fast = false;
    }

    fn seek_relative(&mut self, delta: i64) {
        if let Some(engine) = self.engine.as_mut() {
            let target = (engine.elapsed_frames as i64 + delta).max(0) as u64;
            engine.seek(target);
            if engine.transport == Transport::Stop {
                engine.transport = Transport::Pause;
            }
        }
    }

    fn toggle_mute(&mut self, voice: usize, solo: bool) {
        if let Some(engine) = self.engine.as_mut() {
            if solo {
                let only = (0..VOICES).all(|v| engine.muted[v] == (v != voice));
                for v in 0..VOICES {
                    engine.muted[v] = !only && v != voice;
                }
            } else {
                engine.muted[voice] = !engine.muted[voice];
            }
        }
    }

    fn handle_key(&mut self, key: KeyCode, shift: bool) {
        const DIGITS: [KeyCode; 8] = [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
            KeyCode::Digit6,
            KeyCode::Digit7,
            KeyCode::Digit8,
        ];
        match key {
            KeyCode::Space => self.toggle_play(),
            KeyCode::KeyS | KeyCode::Escape => self.stop(),
            KeyCode::KeyF => self.fast = !self.fast,
            KeyCode::ArrowLeft => self.seek_relative(-(SEEK_FRAMES as i64)),
            KeyCode::ArrowRight => self.seek_relative(SEEK_FRAMES as i64),
            KeyCode::PageUp => self.seek_relative(-(SEEK_FRAMES as i64) * 6),
            KeyCode::PageDown => self.seek_relative(SEEK_FRAMES as i64 * 6),
            KeyCode::Home => self.seek_relative(i64::MIN / 2),
            KeyCode::Digit0 => {
                if let Some(engine) = self.engine.as_mut() {
                    engine.muted = [false; VOICES];
                }
            }
            _ => {
                if let Some(index) = DIGITS.iter().position(|digit| *digit == key) {
                    self.toggle_mute(index + if shift { 8 } else { 0 }, false);
                }
            }
        }
    }

    fn draw(&mut self, ui: &Ui) {
        let display = ui.io().display_size;
        ui.window("##sgub_monitor")
            .position([0.0, 0.0], Condition::Always)
            .size(display, Condition::Always)
            .flags(
                WindowFlags::NO_DECORATION
                    | WindowFlags::NO_MOVE
                    | WindowFlags::NO_BACKGROUND
                    | WindowFlags::NO_SCROLL_WITH_MOUSE
                    | WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS,
            )
            .build(|| {
                let draw = ui.get_window_draw_list();
                draw.add_rect([0.0, 0.0], display, BG).filled(true).build();
                self.draw_tracks(ui, &draw, [8.0, 8.0]);
                self.draw_header(ui, &draw, [724.0, 8.0]);
                self.draw_transport(ui, &draw, [724.0, 64.0]);
                self.draw_spectrum(&draw, [724.0, 192.0]);
                self.draw_scope(&draw, [724.0, 398.0]);
                self.draw_registers(&draw, [724.0, 486.0]);
                draw.add_text([724.0, 800.0], LABEL, &self.status);
                draw.add_text(
                    [724.0, 816.0],
                    DIM,
                    "SPACE play/pause  S stop  F fast  LEFT/RIGHT 5s  PgUp/PgDn 30s  Home top",
                );
                draw.add_text(
                    [724.0, 830.0],
                    DIM,
                    "1-8 / Shift+1-8 mute  0 unmute all  click track: mute  right-click: solo",
                );
            });
    }

    fn draw_tracks(&mut self, ui: &Ui, draw: &DrawListMut, origin: [f32; 2]) {
        const TRACK_H: f32 = 51.0;
        let [x0, y0] = origin;
        draw.add_text([x0, y0], ACCENT, "SGU TRACK MONITOR");
        draw.add_text(
            [x0 + 160.0, y0],
            LABEL,
            "2 x 3WS8PN  /  8 CH x 2  /  48 kHz",
        );
        let mut clicked = None;
        for voice in 0..VOICES {
            let y = y0 + 18.0 + voice as f32 * TRACK_H;
            let (view, meter, muted) = match &self.engine {
                Some(engine) => (
                    Some(engine.channel(voice)),
                    engine.channel_meters[voice],
                    engine.muted[voice],
                ),
                None => (None, Meter::default(), false),
            };
            draw_track(
                draw,
                [x0, y],
                voice,
                view.as_ref(),
                meter,
                muted,
                self.animation,
            );
            let mouse = ui.io().mouse_pos;
            if mouse[0] >= x0
                && mouse[0] < x0 + 700.0
                && mouse[1] >= y
                && mouse[1] < y + TRACK_H - 3.0
            {
                if ui.is_mouse_clicked(MouseButton::Left) {
                    clicked = Some((voice, false));
                } else if ui.is_mouse_clicked(MouseButton::Right) {
                    clicked = Some((voice, true));
                }
            }
        }
        if let Some((voice, solo)) = clicked {
            self.toggle_mute(voice, solo);
        }
    }

    fn draw_header(&self, ui: &Ui, draw: &DrawListMut, [x, y]: [f32; 2]) {
        ui.set_cursor_screen_pos([x, y]);
        ui.set_window_font_scale(2.6);
        ui.text_colored(TEXT, "SGUB PLAYER");
        ui.set_window_font_scale(1.0);
        draw.add_text([x + 250.0, y + 6.0], LABEL, "for CPT32 SGU");
        draw.add_text([x + 250.0, y + 20.0], LABEL, "STATUS DISPLAY");
        let (info, file) = match &self.engine {
            Some(engine) => (
                format!(
                    "SGUB v{}  {} bytes{}",
                    engine.stream_version(),
                    engine.stream_size(),
                    if engine.repeats() {
                        "  REPEAT"
                    } else {
                        "  ONCE"
                    }
                ),
                self.file_name.as_str(),
            ),
            None => (String::from("NO STREAM"), "-"),
        };
        let file: String = file.chars().take(34).collect();
        draw.add_text([x + 380.0, y + 6.0], TEXT, file);
        draw.add_text([x + 380.0, y + 20.0], LABEL, info);
        draw.add_line([x, y + 46.0], [x + 596.0, y + 46.0], DIM)
            .build();
    }

    fn draw_transport(&mut self, ui: &Ui, draw: &DrawListMut, [x, y]: [f32; 2]) {
        let transport = self.engine.as_ref().map(|e| e.transport);
        let playing = transport == Some(Transport::Play);
        // Rotating activity indicator.
        let center = [x + 24.0, y + 26.0];
        let phase = if playing {
            self.animation as f32 * 0.12
        } else {
            0.0
        };
        for spoke in 0..12 {
            let angle = phase + spoke as f32 * std::f32::consts::TAU / 12.0;
            let (s, c) = angle.sin_cos();
            let bright = if playing {
                0.25 + 0.75 * (spoke as f32 / 11.0)
            } else {
                0.3
            };
            draw.add_line(
                [center[0] + c * 10.0, center[1] + s * 10.0],
                [center[0] + c * 21.0, center[1] + s * 21.0],
                [bright, bright, bright, 1.0],
            )
            .thickness(2.0)
            .build();
        }
        let buttons: [(&str, bool, u8); 6] = [
            (">  PLAY", playing, 0),
            ("|| PAUSE", transport == Some(Transport::Pause), 1),
            ("[] STOP", transport == Some(Transport::Stop), 2),
            (">> FAST", self.fast, 3),
            ("<< 5s", false, 4),
            ("   5s >>", false, 5),
        ];
        let mut action = None;
        for (index, (label, active, id)) in buttons.iter().enumerate() {
            let bx = x + 60.0 + (index % 3) as f32 * 82.0;
            let by = y + 4.0 + (index / 3) as f32 * 24.0;
            let max = [bx + 76.0, by + 19.0];
            let mouse = ui.io().mouse_pos;
            let hovered =
                mouse[0] >= bx && mouse[0] < max[0] && mouse[1] >= by && mouse[1] < max[1];
            if *active {
                draw.add_rect([bx, by], max, TEXT).filled(true).build();
            } else if hovered {
                draw.add_rect([bx, by], max, PANEL).filled(true).build();
            }
            draw.add_rect([bx, by], max, DIM).build();
            draw.add_text([bx + 6.0, by + 3.0], if *active { BG } else { TEXT }, label);
            if hovered && ui.is_mouse_clicked(MouseButton::Left) {
                action = Some(*id);
            }
        }
        match action {
            Some(0) => {
                if !playing {
                    self.toggle_play();
                }
            }
            Some(1) => {
                if playing {
                    self.toggle_play();
                }
            }
            Some(2) => self.stop(),
            Some(3) => self.fast = !self.fast,
            Some(4) => self.seek_relative(-(SEEK_FRAMES as i64)),
            Some(5) => self.seek_relative(SEEK_FRAMES as i64),
            _ => {}
        }

        // Time display.
        let (elapsed, total, index, count, loops) = match &self.engine {
            Some(e) => (
                e.elapsed_frames,
                u64::from(e.frame_count()),
                e.frame_index(),
                e.frame_count(),
                e.loops,
            ),
            None => (0, 0, 0, 1, 0),
        };
        let tx = x + 318.0;
        draw.add_rect([tx, y + 2.0], [tx + 3.0, y + 46.0], TEXT)
            .filled(true)
            .build();
        draw.add_text([tx + 8.0, y + 2.0], LABEL, "PASSED");
        draw.add_text([tx + 8.0, y + 14.0], LABEL, "TIME");
        draw_segments(
            draw,
            [tx + 64.0, y + 2.0],
            24.0,
            &format_time(elapsed),
            TEXT,
        );
        draw.add_text([tx + 8.0, y + 32.0], LABEL, "TOTAL");
        draw_segments(
            draw,
            [tx + 64.0, y + 32.0],
            12.0,
            &format_time(total),
            LABEL,
        );
        draw.add_text([tx + 160.0, y + 32.0], LABEL, format!("LOOP {loops:>3}"));

        // Seek bar over one pass of the stream.
        let bar = [x, y + 60.0, x + 596.0, y + 68.0];
        draw.add_rect([bar[0], bar[1]], [bar[2], bar[3]], DIM)
            .build();
        let fraction = index as f32 / count.max(1) as f32;
        draw.add_rect(
            [bar[0] + 1.0, bar[1] + 1.0],
            [
                bar[0] + 1.0 + (bar[2] - bar[0] - 2.0) * fraction,
                bar[3] - 1.0,
            ],
            ACCENT,
        )
        .filled(true)
        .build();
        draw.add_text(
            [x, y + 72.0],
            LABEL,
            format!("FRAME {index:>7} / {count:<7}"),
        );
        let mouse = ui.io().mouse_pos;
        if ui.is_mouse_clicked(MouseButton::Left)
            && mouse[0] >= bar[0]
            && mouse[0] < bar[2]
            && mouse[1] >= bar[1] - 3.0
            && mouse[1] < bar[3] + 3.0
        {
            if let Some(engine) = self.engine.as_mut() {
                let pass_start = engine.elapsed_frames - u64::from(engine.frame_index());
                let target = ((mouse[0] - bar[0]) / (bar[2] - bar[0]) * count as f32) as u64;
                engine.seek(pass_start + target);
                if engine.transport == Transport::Stop {
                    engine.transport = Transport::Pause;
                }
            }
        }

        // Master meters.
        let meters = self
            .engine
            .as_ref()
            .map(|e| e.master_meters)
            .unwrap_or_default();
        for (side, meter) in meters.iter().enumerate() {
            let my = y + 92.0 + side as f32 * 14.0;
            draw.add_text(
                [x + 160.0, my - 2.0],
                LABEL,
                if side == 0 { "L" } else { "R" },
            );
            draw_meter(draw, [x + 174.0, my], [x + 596.0, my + 9.0], *meter, 52);
        }
        draw.add_text([x, y + 92.0], LABEL, "MASTER");
        draw.add_text([x, y + 106.0], LABEL, "OUTPUT");
    }

    fn draw_spectrum(&self, draw: &DrawListMut, [x, y]: [f32; 2]) {
        draw.add_rect([x, y], [x + 3.0, y + 12.0], TEXT)
            .filled(true)
            .build();
        draw.add_text([x + 8.0, y], TEXT, "SPECTRUM ANALYZER");
        let (w, h) = (288.0, 160.0);
        for side in 0..2 {
            let px = x + 20.0 + side as f32 * (w + 8.0);
            let py = y + 20.0;
            if side == 0 {
                for (db, label) in [(0.0, "0"), (-48.0, "-48"), (-96.0, "-96")] {
                    let ly = py + h * (-db / 96.0);
                    draw.add_text([x - 2.0, ly - 6.0], LABEL, label);
                }
                draw.add_text([x + 150.0, y], LABEL, "dB / 40 Hz - 20 kHz");
            }
            for db in [-24.0, -48.0, -72.0] {
                let ly = py + h * (-db / 96.0);
                draw.add_line([px, ly], [px + w - 12.0, ly], [0.12, 0.13, 0.14, 1.0])
                    .build();
            }
            draw.add_line([px, py + h + 1.0], [px + w - 12.0, py + h + 1.0], DIM)
                .build();
            let bar_w = (w - 12.0) / BANDS as f32;
            for band in 0..BANDS {
                let value = self.spectrum[side][band];
                let level = ((value + 96.0) / 96.0).clamp(0.0, 1.0);
                let bx = px + band as f32 * bar_w;
                // Blocky bars, 4 px per segment.
                let segments = (level * h / 4.0) as i32;
                for s in 0..segments {
                    let sy = py + h - (s + 1) as f32 * 4.0;
                    draw.add_rect([bx + 1.0, sy + 1.0], [bx + bar_w - 1.0, sy + 4.0], METER_LO)
                        .filled(true)
                        .build();
                }
                let peak = ((self.spectrum_peaks[side][band] + 96.0) / 96.0).clamp(0.0, 1.0);
                if peak > 0.0 {
                    let sy = py + h - peak * h;
                    draw.add_rect([bx + 1.0, sy], [bx + bar_w - 1.0, sy + 2.0], ACCENT)
                        .filled(true)
                        .build();
                }
            }
            // Frequency labels on the log axis.
            let ratio = (20_000.0f32 / 40.0).ln();
            for (hz, label) in [
                (100.0, "100"),
                (500.0, "500"),
                (1000.0, "1K"),
                (4000.0, "4K"),
                (10000.0, "10K"),
            ] {
                let fx = px + (w - 12.0) * ((hz / 40.0f32).ln() / ratio);
                draw.add_line([fx, py + h + 1.0], [fx, py + h + 4.0], DIM)
                    .build();
                draw.add_text([fx - 6.0, py + h + 5.0], LABEL, label);
            }
            draw.add_text(
                [px + w - 26.0, py],
                LABEL,
                if side == 0 { "L" } else { "R" },
            );
        }
    }

    fn draw_scope(&self, draw: &DrawListMut, [x, y]: [f32; 2]) {
        draw.add_rect([x, y], [x + 3.0, y + 12.0], TEXT)
            .filled(true)
            .build();
        draw.add_text([x + 8.0, y], TEXT, "OSCILLOSCOPE");
        draw.add_text([x + 150.0, y], LABEL, "x4  1 frame");
        let Some(engine) = &self.engine else { return };
        let (w, h) = (288.0, 64.0);
        for side in 0..2 {
            let px = x + 20.0 + side as f32 * (w + 8.0);
            let py = y + 16.0;
            draw.add_rect([px, py], [px + w - 12.0, py + h], PANEL)
                .filled(true)
                .build();
            draw.add_line([px, py + h / 2.0], [px + w - 12.0, py + h / 2.0], DIM)
                .build();
            let history = engine.history(side);
            let count = FRAME_SAMPLES;
            let start = history.len() - count;
            let step = 2;
            let mut prev = None;
            for i in (0..count).step_by(step) {
                let sample = (f32::from(history[start + i]) / 8192.0).clamp(-1.0, 1.0);
                let point = [
                    px + (w - 12.0) * i as f32 / count as f32,
                    py + h / 2.0 - sample * h / 2.0,
                ];
                if let Some(prev) = prev {
                    draw.add_line(prev, point, ACCENT).build();
                }
                prev = Some(point);
            }
        }
    }

    fn draw_registers(&self, draw: &DrawListMut, [x, y]: [f32; 2]) {
        draw.add_rect([x, y], [x + 3.0, y + 12.0], TEXT)
            .filled(true)
            .build();
        draw.add_text([x + 8.0, y], TEXT, "REGISTER");
        draw.add_text(
            [x + 20.0, y + 18.0],
            LABEL,
            "CH   WAVE FREQ  VL PN MOD    P1   P2   START  END    LOOP   C  FIFO",
        );
        let Some(engine) = &self.engine else { return };
        for voice in 0..VOICES {
            let c = engine.channel(voice);
            let row = format!(
                "{}.{}  {} {:04X}  {:02X} {:02X} {:5}  {:04X} {:04X} {:06X} {:06X} {:06X} {:X}  {:>4}",
                voice / 8,
                voice % 8,
                waveform_tag(c.waveform),
                c.frequency,
                c.volume,
                c.panpot,
                modulation_label(&c),
                c.param1,
                c.param2,
                c.pcm_start,
                c.pcm_end,
                c.pcm_loop,
                c.pcm_control,
                if c.waveform == WaveformType::FifoDac {
                    c.fifo_len.to_string()
                } else {
                    "-".into()
                },
            );
            let color = if engine.muted[voice] {
                DIM
            } else if c.audible() {
                TEXT
            } else {
                LABEL
            };
            draw.add_text([x + 20.0, y + 32.0 + voice as f32 * 15.0], color, row);
        }
        let mut notes = Vec::new();
        if engine.card_writes > 0 {
            notes.push(format!(
                "v3 card writes ignored: {} (no expansion card attached)",
                engine.card_writes
            ));
        }
        if let Some(error) = &engine.error {
            notes.push(format!("STREAM ERROR: {error}"));
        }
        for (index, note) in notes.iter().enumerate() {
            draw.add_text([x + 20.0, y + 276.0 + index as f32 * 14.0], METER_HI, note);
        }
    }
}

/// "PHS>3" (absolute target) / "PHS+1" (relative), or "---" without modulation.
fn modulation_label(view: &ChannelView) -> String {
    if view.modulation == ModulationType::None {
        return "---".into();
    }
    format!(
        "{}{}{}",
        modulation_tag(view.modulation),
        if view.modulation_relative { '+' } else { '>' },
        view.modulation_target
    )
}

fn format_time(frames: u64) -> String {
    let centis = frames * 100 / 60;
    format!(
        "{:03}:{:02}.{:02}",
        centis / 6000,
        centis / 100 % 60,
        centis % 100
    )
}

fn draw_track(
    draw: &DrawListMut,
    [x, y]: [f32; 2],
    voice: usize,
    view: Option<&ChannelView>,
    meter: Meter,
    muted: bool,
    animation: u32,
) {
    let audible = view.is_some_and(|v| v.audible()) && !muted;
    draw.add_rect([x, y], [x + 700.0, y + 48.0], PANEL)
        .filled(true)
        .build();
    // Track label: chip and two-digit 7-segment channel number.
    draw.add_text([x + 2.0, y + 1.0], LABEL, format!("SGU{}", voice / 8));
    draw.add_text([x + 2.0, y + 13.0], LABEL, "TRACK");
    draw_segments(
        draw,
        [x + 2.0, y + 27.0],
        17.0,
        &format!("{:02}", voice + 1),
        if muted { DIM } else { TEXT },
    );
    let lx = x + 44.0;
    let Some(view) = view else {
        draw_keyboard(draw, [lx, y + 18.0], None, 0.0);
        return;
    };
    let tag_color = match view.waveform {
        WaveformType::Wavetable => ACCENT,
        WaveformType::Pcm | WaveformType::Sqv4 => [1.0, 0.85, 0.35, 1.0],
        WaveformType::Noise => [0.75, 0.75, 0.75, 1.0],
        WaveformType::FifoDac => [0.95, 0.55, 1.0, 1.0],
        WaveformType::DmaPcm => DIM,
    };
    if muted {
        draw.add_rect([lx, y + 2.0], [lx + 36.0, y + 14.0], METER_HI)
            .filled(true)
            .build();
        draw.add_text([lx + 3.0, y + 1.0], BG, "MUTE");
    } else {
        draw.add_text([lx, y + 1.0], tag_color, waveform_tag(view.waveform));
    }
    let note = view
        .midi_note()
        .map(note_name)
        .unwrap_or_else(|| "---".into());
    let fields = [
        (40.0, format!("NT:{note}")),
        (100.0, format!("FQ:{:5}", view.frequency)),
        (174.0, format!("VL:{:3}", view.volume)),
        (
            230.0,
            format!("PN:{:X}/{:X}", view.panpot >> 4, view.panpot & 15),
        ),
        (294.0, format!("MD:{}", modulation_label(view))),
    ];
    for (dx, text) in fields {
        draw.add_text([lx + dx, y + 1.0], if audible { TEXT } else { LABEL }, text);
    }
    let key = if audible { view.midi_note() } else { None };
    draw_keyboard(draw, [lx, y + 18.0], key, meter.level);

    // Per-voice detail box: wave shape, PCM position or FIFO fill.
    let bx = lx + 342.0;
    let (bw, bh) = (130.0, 26.0);
    let by = y + 18.0;
    draw.add_rect([bx, by], [bx + bw, by + bh], BG)
        .filled(true)
        .build();
    draw.add_rect([bx, by], [bx + bw, by + bh], DIM).build();
    let color = if audible { tag_color } else { DIM };
    match view.waveform {
        WaveformType::Wavetable => {
            let mut prev = None;
            for i in 0..64 {
                let value = f32::from(view.wavetable[i * 4]) / 255.0;
                let point = [
                    bx + 1.0 + i as f32 * (bw - 2.0) / 63.0,
                    by + bh - 1.0 - value * (bh - 2.0),
                ];
                if let Some(prev) = prev {
                    draw.add_line(prev, point, color).build();
                }
                prev = Some(point);
            }
        }
        WaveformType::Pcm | WaveformType::Sqv4 => {
            let length = view.pcm_end.saturating_sub(view.pcm_start).max(1) as f32;
            let to_x = |offset: f32| bx + 1.0 + (bw - 2.0) * (offset / length).clamp(0.0, 1.0);
            let loop_x = to_x(view.pcm_loop.saturating_sub(view.pcm_start) as f32);
            let pos_x = to_x(view.pcm_position as f32);
            draw.add_rect([bx + 1.0, by + 12.0], [pos_x, by + bh - 2.0], color)
                .filled(true)
                .build();
            if view.pcm_control & 2 != 0 {
                draw.add_line([loop_x, by + 1.0], [loop_x, by + bh - 1.0], KEY_ON)
                    .build();
            }
            draw.add_text(
                [bx + 3.0, by],
                LABEL,
                format!("{:05X}-{:05X}", view.pcm_start, view.pcm_end),
            );
        }
        WaveformType::Noise => {
            if audible {
                let mut seed = animation.wrapping_mul(2_654_435_761) ^ voice as u32;
                let mut prev = None;
                for i in 0..32 {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let value = (seed & 0xFF) as f32 / 255.0;
                    let point = [
                        bx + 1.0 + i as f32 * (bw - 2.0) / 31.0,
                        by + bh - 1.0 - value * (bh - 2.0),
                    ];
                    if let Some(prev) = prev {
                        draw.add_line(prev, point, color).build();
                    }
                    prev = Some(point);
                }
            } else {
                draw.add_line(
                    [bx + 1.0, by + bh / 2.0],
                    [bx + bw - 1.0, by + bh / 2.0],
                    color,
                )
                .build();
            }
        }
        WaveformType::FifoDac => {
            let fill = view.fifo_len as f32 / FIFO_CAPACITY as f32;
            draw.add_rect(
                [bx + 1.0, by + 12.0],
                [bx + 1.0 + (bw - 2.0) * fill, by + bh - 2.0],
                color,
            )
            .filled(true)
            .build();
            draw.add_text([bx + 3.0, by], LABEL, format!("FIFO {:4}", view.fifo_len));
        }
        WaveformType::DmaPcm => {}
    }

    // Level meter and pan.
    let mx = bx + bw + 8.0;
    draw.add_text([mx, y + 1.0], LABEL, "LV");
    draw_meter(draw, [mx + 18.0, y + 4.0], [x + 696.0, y + 12.0], meter, 24);
    let pan_l = f32::from(view.panpot >> 4) / 15.0;
    let pan_r = f32::from(view.panpot & 15) / 15.0;
    let pw = x + 696.0 - (mx + 18.0);
    for (row, (label, value)) in [("L", pan_l), ("R", pan_r)].into_iter().enumerate() {
        let py = y + 20.0 + row as f32 * 13.0;
        draw.add_text([mx, py - 3.0], LABEL, label);
        draw.add_rect(
            [mx + 18.0, py + 1.0],
            [mx + 18.0 + pw, py + 7.0],
            [0.13, 0.14, 0.15, 1.0],
        )
        .filled(true)
        .build();
        draw.add_rect(
            [mx + 18.0, py + 1.0],
            [mx + 18.0 + pw * value, py + 7.0],
            if audible {
                [0.5, 0.52, 0.55, 1.0]
            } else {
                [0.2, 0.21, 0.22, 1.0]
            },
        )
        .filled(true)
        .build();
    }
}

/// C0..B7 piano keyboard; `key` is a fractional MIDI note to highlight.
fn draw_keyboard(draw: &DrawListMut, [x, y]: [f32; 2], key: Option<f32>, level: f32) {
    const WHITE: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
    const BLACK: [(u8, usize); 5] = [(1, 0), (3, 1), (6, 3), (8, 4), (10, 5)];
    const OCTAVES: usize = 8;
    let (ww, wh, bw, bh) = (6.0, 26.0, 4.0, 15.0);
    let pressed = key.map(|k| k.round() as i32);
    let intensity = 0.55 + 0.45 * level.clamp(0.0, 1.0);
    let lit = [KEY_ON[0] * intensity, KEY_ON[1], KEY_ON[2], 1.0];
    for octave in 0..OCTAVES {
        let ox = x + octave as f32 * 7.0 * ww;
        let base = 12 + octave as i32 * 12;
        for (i, semitone) in WHITE.iter().enumerate() {
            let kx = ox + i as f32 * ww;
            let on = pressed == Some(base + i32::from(*semitone));
            draw.add_rect(
                [kx, y],
                [kx + ww - 1.0, y + wh],
                if on { lit } else { [0.88, 0.88, 0.88, 1.0] },
            )
            .filled(true)
            .build();
        }
        for (semitone, after) in BLACK {
            let kx = ox + (after + 1) as f32 * ww - bw / 2.0 - 0.5;
            let on = pressed == Some(base + i32::from(semitone));
            draw.add_rect([kx, y], [kx + bw, y + bh], if on { lit } else { BG })
                .filled(true)
                .build();
        }
    }
}

/// Segmented horizontal level meter.
fn draw_meter(draw: &DrawListMut, min: [f32; 2], max: [f32; 2], meter: Meter, segments: usize) {
    let width = max[0] - min[0];
    let seg_w = width / segments as f32;
    let lit = (meter.level * segments as f32).round() as usize;
    let peak = ((meter.peak * segments as f32).round() as usize).min(segments);
    for s in 0..segments {
        let sx = min[0] + s as f32 * seg_w;
        let hot = s * 10 >= segments * 8;
        let color = if s < lit {
            if hot { METER_HI } else { METER_LO }
        } else if s + 1 == peak {
            ACCENT
        } else {
            [0.13, 0.14, 0.15, 1.0]
        };
        draw.add_rect([sx, min[1]], [sx + seg_w - 1.0, max[1]], color)
            .filled(true)
            .build();
    }
}

/// Seven-segment digits for `0-9`, `:` and `.`; `h` is the digit height.
fn draw_segments(draw: &DrawListMut, [x, y]: [f32; 2], h: f32, text: &str, color: Rgba) {
    // Segment bits: a b c d e f g.
    const DIGITS: [u8; 10] = [
        0b1111110, 0b0110000, 0b1101101, 0b1111001, 0b0110011, 0b1011011, 0b1011111, 0b1110000,
        0b1111111, 0b1111011,
    ];
    let w = h * 0.5;
    let t = (h * 0.12).max(1.5);
    let off = [color[0] * 0.12, color[1] * 0.12, color[2] * 0.12, 1.0];
    let mut cx = x;
    for ch in text.chars() {
        match ch {
            ':' => {
                for dy in [h * 0.3, h * 0.7] {
                    draw.add_rect(
                        [cx + t * 0.5, y + dy - t / 2.0],
                        [cx + t * 1.5, y + dy + t / 2.0],
                        color,
                    )
                    .filled(true)
                    .build();
                }
                cx += t * 2.5;
            }
            '.' => {
                draw.add_rect([cx + t * 0.3, y + h - t], [cx + t * 1.3, y + h], color)
                    .filled(true)
                    .build();
                cx += t * 2.0;
            }
            '0'..='9' => {
                let bits = DIGITS[ch as usize - '0' as usize];
                let mid = y + h / 2.0;
                let segments = [
                    ([cx + t, y], [cx + w - t, y + t]),
                    ([cx + w - t, y + t], [cx + w, mid]),
                    ([cx + w - t, mid], [cx + w, y + h - t]),
                    ([cx + t, y + h - t], [cx + w - t, y + h]),
                    ([cx, mid], [cx + t, y + h - t]),
                    ([cx, y + t], [cx + t, mid]),
                    ([cx + t, mid - t / 2.0], [cx + w - t, mid + t / 2.0]),
                ];
                for (index, (min, max)) in segments.into_iter().enumerate() {
                    let on = bits & (0b1000000 >> index) != 0;
                    draw.add_rect(min, max, if on { color } else { off })
                        .filled(true)
                        .build();
                }
                cx += w + t * 1.5;
            }
            _ => cx += w,
        }
    }
}

// ---------------------------------------------------------------------------
// Application loop.

struct PlayerApp {
    monitor: Monitor,
    initial_path: Option<PathBuf>,
    audio: audio::AudioHost,
    display: Option<OwnedDisplayHandle>,
    window: Option<MonitorWindow>,
    modifiers: ModifiersState,
    next_tick: Instant,
    sample_remainder: u32,
    fatal_error: Option<String>,
}

impl PlayerApp {
    fn tick_audio(&mut self) {
        self.sample_remainder += self.audio.sample_rate();
        let samples = (self.sample_remainder / 60) as usize;
        self.sample_remainder %= 60;
        let (left, right) = self.monitor.tick(samples);
        self.audio.push_samples_i16(&left, &right);
    }

    fn update_title(&self) {
        if let Some(window) = &self.window {
            let title = if self.monitor.file_name.is_empty() {
                "SGUB Player".to_string()
            } else {
                format!("SGUB Player - {}", self.monitor.file_name)
            };
            window.window.set_title(&title);
        }
    }
}

impl ApplicationHandler for PlayerApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("SGUB Player")
                    .with_inner_size(LogicalSize::new(DESIGN_SIZE[0], DESIGN_SIZE[1]))
                    .with_min_inner_size(LogicalSize::new(DESIGN_SIZE[0], DESIGN_SIZE[1])),
            )
            .map_err(|error| format!("Cannot create player window: {error}"))
            .and_then(|window| MonitorWindow::new(Arc::new(window), self.display.take().unwrap()));
        match result {
            Ok(window) => {
                self.window = Some(window);
                if let Some(path) = self.initial_path.take() {
                    self.monitor.open(&path);
                }
                self.update_title();
                self.next_tick = Instant::now();
            }
            Err(error) => {
                self.fatal_error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.as_mut() else {
            return;
        };
        if window.window.id() != id {
            return;
        }
        window.platform.handle_event(
            window.imgui.io_mut(),
            &window.window,
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
                if event.state == ElementState::Pressed && !event.repeat {
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.monitor.handle_key(key, self.modifiers.shift_key());
                    }
                } else if event.state == ElementState::Pressed {
                    // Allow held arrows to keep seeking.
                    if let PhysicalKey::Code(key @ (KeyCode::ArrowLeft | KeyCode::ArrowRight)) =
                        event.physical_key
                    {
                        self.monitor.handle_key(key, false);
                    }
                }
            }
            WindowEvent::DroppedFile(path) => {
                self.monitor.open(&path);
                self.update_title();
            }
            WindowEvent::Resized(size) => window.resize(size),
            WindowEvent::RedrawRequested => window.draw(&mut self.monitor),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
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
            if let Some(window) = &self.window {
                window.window.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_tick));
    }
}

const USAGE: &str = "Usage: sgub_player [--sgu-version 1|2|2.1|3|3.1] [song.sgub]";

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let mut path = None;
    let mut version = SguVersion::default();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--sgu-version") => {
                let value = args.next().ok_or(USAGE)?;
                version = value
                    .to_str()
                    .and_then(SguVersion::parse)
                    .ok_or_else(|| format!("Unknown SGU version {value:?}\n{USAGE}"))?;
            }
            Some("-h" | "--help") => {
                println!("{USAGE}");
                return Ok(());
            }
            _ if path.is_none() => path = Some(PathBuf::from(arg)),
            _ => return Err(USAGE.into()),
        }
    }
    if let Some(path) = &path {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
        Engine::new(bytes, version)
            .map_err(|error| format!("Cannot play {}: {error}", path.display()))?;
    }
    let event_loop =
        EventLoop::new().map_err(|error| format!("Cannot create event loop: {error}"))?;
    let mut app = PlayerApp {
        monitor: Monitor::new(version),
        initial_path: path,
        audio: audio::AudioHost::new()?,
        display: Some(event_loop.owned_display_handle()),
        window: None,
        modifiers: ModifiersState::empty(),
        next_tick: Instant::now(),
        sample_remainder: 0,
        fatal_error: None,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|error| format!("Player event loop failed: {error}"))?;
    match app.fatal_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn main() {
    cpt32::init_logging();
    if let Err(error) = run() {
        tracing::error!(%error, "SGUB Player failed");
        eprintln!("{error}");
        std::process::exit(1);
    }
}
