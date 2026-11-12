//! SGUB Player: standalone SGUB stream player with a real-time playback monitor.
//!
//! Usage: `sgub_player [--sgu-version 1|2|2.1|3|3.1] [--card SLOT=PACKAGE]... [song.sgub]`.
//! A file can also be dropped onto the window. Expansion cards required by a
//! v3/v4 stream are loaded from `--card`, or found by file name next to the
//! stream, next to the executable, or in the repository's example targets.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpt32::devices::sgu::s3w2::{FIFO_CAPACITY, ModulationType, SguVersion, WaveformType};
use cpt32::music::CardRequirement;
use cpt32::music::monitor::{
    CardKind, CardSlot, CardStatus, ChannelView, Engine, FRAME_SAMPLES, HsView, Meter, Transport,
    VOICES, WtView, midi_from_hz, modulation_tag, note_name, spectrum_db, waveform_tag,
};
use imgui::{Condition, DrawListMut, MouseButton, Ui, WindowFlags};
use imgui_wgpu::{Renderer, RendererConfig};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, Event, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowId};

#[allow(dead_code)]
#[path = "../audio.rs"]
mod audio;

const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);
const FAST_FACTOR: usize = 4;
const SEEK_SECONDS: f64 = 5.0;
const DESIGN_SIZE: [f32; 2] = [1328.0, 864.0];
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
    /// Whole-number pixel scale (1 design pixel = `scale` x `scale` screen pixels).
    scale: u32,
    fixed_scale: Option<u32>,
}

/// Largest whole-number scale at which the design fits the window, or the
/// user's `--scale`. Fractional scales would map one font or 7-segment pixel to
/// 1 or 2 screen pixels and blur them through filtering.
fn pixel_scale(size: PhysicalSize<u32>, fixed: Option<u32>) -> u32 {
    fixed.unwrap_or_else(|| {
        let fit_w = size.width as f32 / DESIGN_SIZE[0];
        let fit_h = size.height as f32 / DESIGN_SIZE[1];
        (fit_w.min(fit_h).floor() as u32).max(1)
    })
}

/// Rebuild the font atlas texture with nearest-neighbour sampling so glyphs stay
/// pixel-exact at integer scales (imgui-wgpu's default sampler is linear).
fn install_nearest_font(
    imgui: &mut imgui::Context,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
) {
    let fonts = imgui.fonts();
    renderer.textures.remove(fonts.tex_id);
    let handle = fonts.build_rgba32_texture();
    let mut config = imgui_wgpu::TextureConfig {
        label: Some("SGUB Player font atlas"),
        size: wgpu::Extent3d {
            width: handle.width,
            height: handle.height,
            ..Default::default()
        },
        format: Some(wgpu::TextureFormat::Rgba8Unorm),
        ..Default::default()
    };
    config.sampler_desc.mag_filter = wgpu::FilterMode::Nearest;
    config.sampler_desc.min_filter = wgpu::FilterMode::Nearest;
    config.sampler_desc.mipmap_filter = wgpu::MipmapFilterMode::Nearest;
    let texture = imgui_wgpu::Texture::new(device, renderer, config);
    texture.write(queue, handle.data, handle.width, handle.height);
    fonts.tex_id = renderer.textures.insert(texture);
    fonts.clear_tex_data();
}

impl MonitorWindow {
    fn new(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        fixed_scale: Option<u32>,
    ) -> Result<Self, String> {
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
        // Hard pixel edges everywhere: anti-aliased fringes look like blur at 1x.
        let style = imgui.style_mut();
        style.anti_aliased_lines = false;
        style.anti_aliased_lines_use_tex = false;
        style.anti_aliased_fill = false;
        let scale = pixel_scale(size, fixed_scale);
        let mut platform = WinitPlatform::new(&mut imgui);
        platform.attach_window(imgui.io_mut(), &window, HiDpiMode::Locked(f64::from(scale)));
        let mut renderer = Renderer::new(
            &mut imgui,
            &device,
            &queue,
            RendererConfig {
                texture_format: format,
                ..RendererConfig::default()
            },
        );
        install_nearest_font(&mut imgui, &device, &queue, &mut renderer);
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
            scale,
            fixed_scale,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.size = size;
        let scale = pixel_scale(size, self.fixed_scale);
        if scale != self.scale {
            self.scale = scale;
            self.platform.attach_window(
                self.imgui.io_mut(),
                &self.window,
                HiDpiMode::Locked(f64::from(scale)),
            );
        }
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

/// What the left panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Sgu,
    Si22,
    /// Index into `Engine::cards`.
    Card(usize),
}

struct Monitor {
    engine: Option<Engine>,
    sgu_version: SguVersion,
    view: View,
    /// Card channel whose registers the right panel dumps (16.. = PCM/WT).
    selected: usize,
    card_paths: [Option<PathBuf>; 8],
    file_name: String,
    status: String,
    fast: bool,
    animation: u32,
    spectrum: [Vec<f32>; 2],
    spectrum_peaks: [Vec<f32>; 2],
}

impl Monitor {
    fn new(sgu_version: SguVersion, card_paths: [Option<PathBuf>; 8]) -> Self {
        Self {
            engine: None,
            sgu_version,
            view: View::Sgu,
            selected: 0,
            card_paths,
            file_name: String::new(),
            status: "Drop an .sgub file onto this window".into(),
            fast: false,
            animation: 0,
            spectrum: [vec![-96.0; BANDS], vec![-96.0; BANDS]],
            spectrum_peaks: [vec![-96.0; BANDS], vec![-96.0; BANDS]],
        }
    }

    fn open(&mut self, path: &Path) {
        let explicit = self.card_paths.clone();
        let dirs = card_search_dirs(path);
        let mut resolve =
            |requirement: &CardRequirement, kind: CardKind| -> Result<Option<PathBuf>, String> {
                if let Some(path) = &explicit[usize::from(requirement.slot)] {
                    return Ok(Some(path.clone()));
                }
                Ok(kind.package_file_name().and_then(|name| {
                    dirs.iter()
                        .map(|dir| dir.join(name))
                        .find(|candidate| candidate.is_file())
                }))
            };
        let result = std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                Engine::with_cards(bytes, self.sgu_version, &mut resolve).map_err(|e| e.to_string())
            });
        match result {
            Ok(engine) => {
                let problems = engine.cards.iter().filter(|card| !card.is_ready()).count();
                let devices = engine.cards.len() + usize::from(engine.uses_si22());
                self.engine = Some(engine);
                self.view = View::Sgu;
                self.selected = 0;
                self.file_name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let name = &self.file_name;
                self.status = if problems > 0 {
                    format!("Loaded {name}: {problems} card slot(s) not ready (Tab)")
                } else if devices > 0 {
                    format!("Loaded {name}  (Tab: device views)")
                } else {
                    format!("Loaded {name}")
                };
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

    fn seek_relative(&mut self, seconds: f64) {
        if let Some(engine) = self.engine.as_mut() {
            let delta = (seconds * engine.frames_per_second()).round() as i64;
            let target = (engine.elapsed_frames as i64).saturating_add(delta).max(0) as u64;
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
            KeyCode::ArrowLeft => self.seek_relative(-SEEK_SECONDS),
            KeyCode::ArrowRight => self.seek_relative(SEEK_SECONDS),
            KeyCode::PageUp => self.seek_relative(-SEEK_SECONDS * 6.0),
            KeyCode::PageDown => self.seek_relative(SEEK_SECONDS * 6.0),
            KeyCode::Home => self.seek_relative(-1e9),
            KeyCode::Tab => self.cycle_view(if shift { -1 } else { 1 }),
            KeyCode::KeyM => self.toggle_device_mute(),
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

    fn views(&self) -> Vec<View> {
        let mut views = vec![View::Sgu];
        if let Some(engine) = &self.engine {
            if engine.uses_si22() {
                views.push(View::Si22);
            }
            views.extend((0..engine.cards.len()).map(View::Card));
        }
        views
    }

    fn cycle_view(&mut self, step: isize) {
        let views = self.views();
        let index = views.iter().position(|v| *v == self.view).unwrap_or(0) as isize;
        self.view = views[(index + step).rem_euclid(views.len() as isize) as usize];
        self.selected = 0;
    }

    fn view_label(&self, view: View) -> String {
        match (view, &self.engine) {
            (View::Sgu, _) => "SGU+SI22 18CH".into(),
            (View::Si22, _) => "34SI22".into(),
            (View::Card(index), Some(engine)) => {
                let card = &engine.cards[index];
                format!("SLOT{} {}", card.slot, card.kind.name())
            }
            (View::Card(_), None) => "-".into(),
        }
    }

    /// M: mute the device shown in the left panel (SGU voices use 1-8).
    fn toggle_device_mute(&mut self) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        match self.view {
            View::Sgu => {}
            View::Si22 => engine.si22_muted = !engine.si22_muted,
            View::Card(index) => {
                if let Some(card) = engine.cards.get_mut(index) {
                    card.muted = !card.muted;
                }
            }
        }
    }

    fn device_muted(&self, view: View) -> bool {
        match (view, &self.engine) {
            (View::Si22, Some(engine)) => engine.si22_muted,
            (View::Card(index), Some(engine)) => engine.cards[index].muted,
            _ => false,
        }
    }

    fn draw_left(&mut self, ui: &Ui, draw: &DrawListMut, [x0, y0]: [f32; 2]) {
        // View tabs.
        let mut tx = x0;
        let mut chosen = None;
        let mouse = ui.io().mouse_pos;
        for view in self.views() {
            let label = self.view_label(view);
            let w = label.len() as f32 * 7.0 + 16.0;
            let active = view == self.view;
            let muted = self.device_muted(view);
            let max = [tx + w, y0 + 15.0];
            let hovered =
                mouse[0] >= tx && mouse[0] < max[0] && mouse[1] >= y0 && mouse[1] < max[1];
            if active {
                draw.add_rect([tx, y0], max, TEXT).filled(true).build();
            } else if hovered {
                draw.add_rect([tx, y0], max, PANEL).filled(true).build();
            }
            draw.add_rect([tx, y0], max, DIM).build();
            let color = if active {
                BG
            } else if muted {
                METER_HI
            } else {
                ACCENT
            };
            draw.add_text([tx + 8.0, y0 + 1.0], color, label);
            if hovered && ui.is_mouse_clicked(MouseButton::Left) {
                chosen = Some(view);
            }
            tx += w + 4.0;
        }
        draw.add_text([tx + 6.0, y0 + 1.0], DIM, "TAB / click");
        if let Some(view) = chosen {
            self.view = view;
            self.selected = 0;
        }
        let origin = [x0, y0 + 18.0];
        match self.view {
            View::Sgu => self.draw_tracks(ui, draw, origin),
            View::Si22 => self.draw_si22(draw, origin),
            View::Card(index) => self.draw_card(ui, draw, origin, index),
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
                self.draw_left(ui, &draw, [8.0, 8.0]);
                self.draw_header(ui, &draw, [724.0, 8.0]);
                self.draw_transport(ui, &draw, [724.0, 64.0]);
                self.draw_spectrum(&draw, [724.0, 192.0]);
                self.draw_scope(&draw, [724.0, 398.0]);
                self.draw_registers(&draw, [724.0, 486.0]);
                let status: String = self.status.chars().take(84).collect();
                draw.add_text([724.0, 800.0], LABEL, status);
                draw.add_text(
                    [724.0, 816.0],
                    DIM,
                    "SPACE play/pause  S stop  F fast  LEFT/RIGHT 5s  PgUp/PgDn 30s  Home top",
                );
                draw.add_text(
                    [724.0, 830.0],
                    DIM,
                    "1-8/Shift+1-8 mute  0 unmute  click: mute  right: solo  TAB view  M mute device",
                );
            });
    }

    fn draw_tracks(&mut self, ui: &Ui, draw: &DrawListMut, origin: [f32; 2]) {
        // 18 rows: SGU0/SGU1 x 8 channels, then 34SI22 PLAY A/B.
        const TRACK_H: f32 = 46.0;
        let [x0, y0] = origin;
        let mut clicked = None;
        for ch in 0..2 {
            let y = y0 + (VOICES + ch) as f32 * TRACK_H;
            draw_si22_track(draw, [x0, y], ch, self.engine.as_ref());
            if hit(ui, [x0, y], [x0 + 700.0, y + TRACK_H - 3.0]) {
                if let Some(engine) = self.engine.as_mut() {
                    engine.si22_muted = !engine.si22_muted;
                }
            }
        }
        for voice in 0..VOICES {
            let y = y0 + voice as f32 * TRACK_H;
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
                self.engine.as_ref(),
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
        ui.set_window_font_scale(3.0);
        ui.text_colored(TEXT, "SGUB PLAYER");
        ui.set_window_font_scale(1.0);
        draw.add_text([x + 250.0, y + 6.0], LABEL, "for CPT32 SGU");
        draw.add_text([x + 250.0, y + 20.0], LABEL, "STATUS DISPLAY");
        let (info, file) = match &self.engine {
            Some(engine) => (
                format!(
                    "SGUB v{}  {} bytes  {}\n{}  {} frames",
                    engine.stream_version(),
                    engine.stream_size(),
                    if engine.repeats() { "REPEAT" } else { "ONCE" },
                    if engine.frame_period() == 0 {
                        "VSYNC 60 Hz".to_string()
                    } else {
                        format!("TIMER {:.2} Hz", engine.frames_per_second())
                    },
                    engine.frame_count()
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
            Some(4) => self.seek_relative(-SEEK_SECONDS),
            Some(5) => self.seek_relative(SEEK_SECONDS),
            _ => {}
        }

        // Time display.
        let (elapsed, total, index, count, loops) = match &self.engine {
            Some(e) => (
                e.elapsed_frames as f64 / e.frames_per_second(),
                f64::from(e.frame_count()) / e.frames_per_second(),
                e.frame_index(),
                e.frame_count(),
                e.loops,
            ),
            None => (0.0, 0.0, 0, 1, 0),
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
            // Whole-pixel bar width so every band is the same width.
            let bar_w = ((w - 12.0) / BANDS as f32).floor();
            let area = bar_w * BANDS as f32;
            for db in [-24.0, -48.0, -72.0] {
                let ly = py + h * (-db / 96.0);
                draw.add_line([px, ly], [px + area, ly], [0.12, 0.13, 0.14, 1.0])
                    .build();
            }
            draw.add_line([px, py + h + 1.0], [px + area, py + h + 1.0], DIM)
                .build();
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
                    let sy = (py + h - peak * h).floor();
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
                let fx = (px + area * ((hz / 40.0f32).ln() / ratio)).floor();
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
        let Some(engine) = &self.engine else { return };
        let mut notes = Vec::new();
        for card in &engine.cards {
            if !card.is_ready() {
                notes.push(card_status_line(card));
            }
        }
        if engine.stray_writes > 0 {
            notes.push(format!(
                "writes to undeclared slots: {}",
                engine.stray_writes
            ));
        }
        if let Some(error) = &engine.error {
            notes.push(format!("STREAM ERROR: {error}"));
        }
        for (index, note) in notes.iter().take(2).enumerate() {
            let note: String = note.chars().take(84).collect();
            draw.add_text([x + 8.0, y + 290.0 + index as f32 * 13.0], METER_HI, note);
        }
        match self.view {
            View::Sgu => {}
            View::Si22 => return draw_si22_registers(draw, [x, y], engine),
            View::Card(index) => {
                return draw_card_registers(draw, [x, y], engine, index, self.selected);
            }
        }
        draw.add_text(
            [x + 20.0, y + 18.0],
            LABEL,
            "CH   WAVE FREQ  VL PN MOD    P1   P2   START  END    LOOP   C  FIFO",
        );
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
            draw.add_text([x + 20.0, y + 32.0 + voice as f32 * 14.0], color, row);
        }
        // 34SI22 play channels as rows 17 and 18.
        let chip = engine.si22();
        for (ch, p) in chip.play.iter().enumerate() {
            let row = format!(
                "S.{}  {} {:<5} VL {:2} DIV {:2} OUT {}{}  START {:06X} STOP {:06X} ADDR {:06X}",
                if ch == 0 { 'A' } else { 'B' },
                ["STOP", "PLAY", "WAIT", "END "][usize::from(p.status & 3)],
                si22_format(p.format_field).trim_end(),
                p.volume,
                p.div,
                if p.l_disable { '-' } else { 'L' },
                if p.r_disable { '-' } else { 'R' },
                p.start,
                p.stop,
                chip.play_address(ch)
            );
            let color = if engine.si22_muted {
                DIM
            } else if p.status == 1 {
                TEXT
            } else {
                LABEL
            };
            draw.add_text(
                [x + 20.0, y + 32.0 + (VOICES + ch) as f32 * 14.0],
                color,
                row,
            );
        }
    }
}

/// 34SI22 play channel `ch` as an SGU-style track row (tracks 17 and 18).
fn draw_si22_track(draw: &DrawListMut, [x, y]: [f32; 2], ch: usize, engine: Option<&Engine>) {
    draw.add_rect([x, y], [x + 700.0, y + 44.0], PANEL)
        .filled(true)
        .build();
    let muted = engine.is_some_and(|e| e.si22_muted);
    draw.add_text([x + 2.0, y + 1.0], LABEL, "SI22");
    draw.add_text([x + 2.0, y + 13.0], LABEL, "TRACK");
    draw_segments(
        draw,
        [x + 2.0, y + 25.0],
        17.0,
        &format!("{:02}", VOICES + ch + 1),
        if muted { DIM } else { TEXT },
    );
    let lx = x + 44.0;
    let Some(engine) = engine else { return };
    let chip = engine.si22();
    let p = &chip.play[ch];
    let running = p.status == 1 && !muted;
    if muted {
        draw.add_rect([lx, y + 2.0], [lx + 36.0, y + 14.0], METER_HI)
            .filled(true)
            .build();
        draw.add_text([lx + 3.0, y + 1.0], BG, "MUTE");
    } else {
        draw.add_text(
            [lx, y + 1.0],
            if running { [0.4, 1.0, 0.6, 1.0] } else { LABEL },
            ["STOP", "PLAY", "WAIT", "END "][usize::from(p.status & 3)],
        );
    }
    let rate = 48_000 / (u32::from(p.div) + 1);
    let fields = [
        (
            40.0,
            format!(
                "{} {}",
                if ch == 0 { "A" } else { "B" },
                si22_format(p.format_field)
            ),
        ),
        (100.0, format!("RT:{rate:5}")),
        (174.0, format!("VL:{:3}", p.volume)),
        (
            230.0,
            format!(
                "OUT:{}{}",
                if p.l_disable { '-' } else { 'L' },
                if p.r_disable { '-' } else { 'R' }
            ),
        ),
    ];
    for (dx, text) in fields {
        draw.add_text([lx + dx, y + 1.0], if running { TEXT } else { LABEL }, text);
    }
    // Playback position from START to STOP over the keyboard and wave box.
    let (bx, by, bw, bh) = (lx, y + 18.0, 472.0, 24.0);
    draw.add_rect([bx, by], [bx + bw, by + bh], BG)
        .filled(true)
        .build();
    draw.add_rect([bx, by], [bx + bw, by + bh], DIM).build();
    let address = chip.play_address(ch);
    let length = p.stop.saturating_sub(p.start).max(1) as f32;
    let fraction = match p.status {
        1 => (address.saturating_sub(p.start) as f32 / length).clamp(0.0, 1.0),
        3 => 1.0,
        _ => 0.0,
    };
    draw.add_rect(
        [bx + 1.0, by + 13.0],
        [bx + 1.0 + (bw - 2.0) * fraction, by + bh - 2.0],
        if running { [0.4, 1.0, 0.6, 1.0] } else { DIM },
    )
    .filled(true)
    .build();
    draw.add_text(
        [bx + 4.0, by],
        LABEL,
        format!(
            "START {:06X}  STOP {:06X}  ADDR {:06X}",
            p.start, p.stop, address
        ),
    );
    // The chip output meter is shared by both channels; show it on the active ones.
    let mx = bx + bw + 8.0;
    let level = if running {
        engine.si22_meters[0].level.max(engine.si22_meters[1].level)
    } else {
        0.0
    };
    draw.add_text([mx, y + 1.0], LABEL, "LV");
    draw_meter(
        draw,
        [mx + 18.0, y + 4.0],
        [x + 696.0, y + 12.0],
        level_meter(level),
        24,
    );
    draw_pan(
        draw,
        [mx, y],
        x + 696.0,
        (
            if p.l_disable { 0 } else { 15 },
            if p.r_disable { 0 } else { 15 },
        ),
        running,
    );
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

fn format_time(seconds: f64) -> String {
    let centis = (seconds * 100.0) as u64;
    format!(
        "{:03}:{:02}.{:02}",
        centis / 6000,
        centis / 100 % 60,
        centis % 100
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_track(
    draw: &DrawListMut,
    [x, y]: [f32; 2],
    voice: usize,
    engine: Option<&Engine>,
    view: Option<&ChannelView>,
    meter: Meter,
    muted: bool,
    animation: u32,
) {
    let audible = view.is_some_and(|v| v.audible()) && !muted;
    draw.add_rect([x, y], [x + 700.0, y + 44.0], PANEL)
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
            draw_polyline(
                draw,
                view.wave_points(bw - 2.0, bh - 2.0)
                    .into_iter()
                    .map(|[px, py]| [bx + 1.0 + px, by + 1.0 + py]),
                color,
            );
            // Pool entry in the wave SRAM, cycle length and interpolation,
            // above the box (clear of the MD field, which ends by bx + 8).
            let pool = view
                .wave_pool
                .map_or_else(|| "---".into(), |id| format!("{id:03}"));
            draw.add_text(
                [bx + 14.0, y + 1.0],
                if audible { TEXT } else { LABEL },
                format!(
                    "WP:{pool} L{}{}",
                    view.wave_length,
                    if view.interpolate { "" } else { " HOLD" }
                ),
            );
        }
        WaveformType::Pcm | WaveformType::Sqv4 => {
            let inner = bw - 2.0;
            let progress = view.pcm_progress();
            // Played part as the background gauge, behind the waveform.
            if let Some((position, _)) = progress {
                let gauge = if audible {
                    [color[0] * 0.3, color[1] * 0.3, color[2] * 0.3, 1.0]
                } else {
                    [0.12, 0.13, 0.14, 1.0]
                };
                draw.add_rect(
                    [bx + 1.0, by + 1.0],
                    [bx + 1.0 + inner * position, by + bh - 1.0],
                    gauge,
                )
                .filled(true)
                .build();
            }
            match (view.waveform, engine) {
                // PCM: the whole sample, min..max of the bytes under each column.
                (WaveformType::Pcm, Some(engine)) => {
                    let to_y = |value: u8| by + 1.0 + f32::from(255 - value) / 255.0 * (bh - 2.0);
                    let overview = engine.pcm_overview(voice, inner as usize);
                    for (column, [low, high]) in overview.into_iter().enumerate() {
                        let cx = bx + 1.0 + column as f32;
                        draw.add_rect([cx, to_y(high)], [cx + 1.0, to_y(low) + 1.0], color)
                            .filled(true)
                            .build();
                    }
                }
                // SQV4 is ADPCM: show the voice's recent output instead,
                // auto-scaled so quiet passages keep their shape.
                (WaveformType::Sqv4, Some(engine)) => {
                    let recent = engine.voice_history(voice);
                    let peak = recent
                        .iter()
                        .map(|s| i32::from(*s).abs())
                        .max()
                        .unwrap_or(0)
                        .max(256) as f32;
                    let mid = by + bh / 2.0;
                    let last = (recent.len().max(2) - 1) as f32;
                    draw_polyline(
                        draw,
                        recent.iter().enumerate().map(|(i, &sample)| {
                            [
                                bx + 1.0 + i as f32 * inner / last,
                                mid - f32::from(sample) / peak * (bh / 2.0 - 1.0),
                            ]
                        }),
                        color,
                    );
                }
                _ => {}
            }
            if let Some((_, loop_start)) = progress
                && view.pcm_control & 2 != 0
            {
                let loop_x = bx + 1.0 + inner * loop_start;
                draw.add_line([loop_x, by + 1.0], [loop_x, by + bh - 1.0], KEY_ON)
                    .build();
            }
            draw.add_text(
                [bx + 14.0, y + 1.0],
                if audible { TEXT } else { LABEL },
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
            let kx = ox + (after + 1) as f32 * ww - bw / 2.0;
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
    // Every segment gets the same whole-pixel width; any remainder is left blank.
    let seg_w = (width / segments as f32).floor().max(2.0);
    let (min, max) = ([min[0].floor(), min[1].floor()], [max[0], max[1].floor()]);
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
/// Every size is a whole number of pixels so all digits of one height look
/// identical: stroke `t`, cell width `w`, and a 1-pixel gap between segments.
fn draw_segments(draw: &DrawListMut, [x, y]: [f32; 2], h: f32, text: &str, color: Rgba) {
    // Segment bits: a b c d e f g.
    const DIGITS: [u8; 10] = [
        0b1111110, 0b0110000, 0b1101101, 0b1111001, 0b0110011, 0b1011011, 0b1011111, 0b1110000,
        0b1111111, 0b1111011,
    ];
    let h = h.round().max(7.0);
    let t = (h / 8.0).floor().max(1.0);
    // Equal upper and lower halves need an even (h - t).
    let h = if (h - t) % 2.0 == 1.0 { h - 1.0 } else { h };
    let w = (h * 0.55).round();
    // Gaps between segments only where the strokes are thick enough to keep them.
    let g = if t >= 3.0 { 1.0 } else { 0.0 };
    // Top of the middle bar; the two halves are equal when (h - t) is even.
    let mid = ((h - t) / 2.0).floor();
    let (x, y) = (x.floor(), y.floor());
    let off = [color[0] * 0.12, color[1] * 0.12, color[2] * 0.12, 1.0];
    let rect = |min: [f32; 2], max: [f32; 2], color: Rgba| {
        draw.add_rect(min, max, color).filled(true).build();
    };
    let mut cx = x;
    for ch in text.chars() {
        match ch {
            ':' => {
                let upper = y + (h * 0.3).round();
                let lower = y + (h * 0.7).round() - t;
                for dy in [upper, lower] {
                    rect([cx + 1.0, dy], [cx + 1.0 + t, dy + t], color);
                }
                cx += t + 3.0;
            }
            '.' => {
                rect([cx + 1.0, y + h - t], [cx + 1.0 + t, y + h], color);
                cx += t + 3.0;
            }
            '0'..='9' => {
                let bits = DIGITS[ch as usize - '0' as usize];
                let (left, right) = (cx, cx + w);
                let (inner_l, inner_r) = (left + t + g, right - t - g);
                let segments = [
                    // a, b, c, d, e, f, g
                    ([inner_l, y], [inner_r, y + t]),
                    ([right - t, y + t + g], [right, y + mid - g]),
                    ([right - t, y + mid + t + g], [right, y + h - t - g]),
                    ([inner_l, y + h - t], [inner_r, y + h]),
                    ([left, y + mid + t + g], [left + t, y + h - t - g]),
                    ([left, y + t + g], [left + t, y + mid - g]),
                    ([inner_l, y + mid], [inner_r, y + mid + t]),
                ];
                for (index, (min, max)) in segments.into_iter().enumerate() {
                    let on = bits & (0b1000000 >> index) != 0;
                    rect(min, max, if on { color } else { off });
                }
                cx += w + t + 1.0;
            }
            _ => cx += w,
        }
    }
}

// ---------------------------------------------------------------------------
// Application loop.

struct PlayerApp {
    monitor: Monitor,
    /// `--scale N`: fixed whole-number pixel scale instead of fitting the window.
    fixed_scale: Option<u32>,
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
        // Start at the OS scale rounded down to a whole number, but never larger
        // than the monitor; resizing picks the largest whole scale that fits.
        let initial_scale = self.fixed_scale.unwrap_or_else(|| {
            event_loop.primary_monitor().map_or(1, |monitor| {
                let size = monitor.size();
                let fit = (size.width as f32 / DESIGN_SIZE[0])
                    .min(size.height as f32 * 0.9 / DESIGN_SIZE[1])
                    .floor();
                (monitor.scale_factor().floor() as f32).min(fit).max(1.0) as u32
            })
        });
        let fixed_scale = self.fixed_scale;
        let result = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("SGUB Player")
                    .with_inner_size(PhysicalSize::new(
                        (DESIGN_SIZE[0] * initial_scale as f32) as u32,
                        (DESIGN_SIZE[1] * initial_scale as f32) as u32,
                    ))
                    .with_min_inner_size(PhysicalSize::new(
                        DESIGN_SIZE[0] as u32,
                        DESIGN_SIZE[1] as u32,
                    )),
            )
            .map_err(|error| format!("Cannot create player window: {error}"))
            .and_then(|window| {
                MonitorWindow::new(Arc::new(window), self.display.take().unwrap(), fixed_scale)
            });
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

const USAGE: &str = "Usage: sgub_player [--sgu-version 1|2|2.1|3|3.1] [--card SLOT=PACKAGE]... [--scale 1..8] [song.sgub]";

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let mut path = None;
    let mut version = SguVersion::default();
    let mut card_paths: [Option<PathBuf>; 8] = Default::default();
    let mut fixed_scale = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--scale") => {
                let value = args.next().ok_or(USAGE)?;
                fixed_scale = Some(
                    value
                        .to_str()
                        .and_then(|text| text.parse::<u32>().ok())
                        .filter(|scale| (1..=8).contains(scale))
                        .ok_or_else(|| format!("--scale must be 1..8\n{USAGE}"))?,
                );
            }
            Some("--card") => {
                let value = args.next().ok_or(USAGE)?;
                let text = value.to_str().ok_or(USAGE)?;
                let (slot, package) = text
                    .split_once('=')
                    .ok_or_else(|| format!("--card expects SLOT=PACKAGE\n{USAGE}"))?;
                let slot: usize = slot
                    .parse()
                    .ok()
                    .filter(|slot| *slot < 8)
                    .ok_or_else(|| format!("--card slot must be 0..7\n{USAGE}"))?;
                card_paths[slot] = Some(PathBuf::from(package));
            }
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
        monitor: Monitor::new(version, card_paths),
        fixed_scale,
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

// ---------------------------------------------------------------------------
// Device views: 34SI22 and expansion cards.

const ROW_H: f32 = 51.0;
const SELECTED: Rgba = [0.09, 0.11, 0.14, 1.0];
const CARD_TAG: Rgba = [1.0, 0.7, 0.35, 1.0];

/// Places to look for a card package that `--card` did not name.
fn card_search_dirs(stream: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = stream.parent() {
        roots.push(dir.to_path_buf());
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        // target/release/sgub_player(.exe) -> repository root two levels up.
        if let Some(repo) = dir.parent().and_then(Path::parent) {
            roots.push(repo.to_path_buf());
        }
        roots.push(dir);
    }
    if let Ok(dir) = std::env::current_dir() {
        roots.push(dir);
    }
    let mut dirs = Vec::new();
    for root in roots {
        for sub in [
            "",
            "cards",
            "examples/wasm-3hs816/target",
            "examples/wasm-3ws88pwn4/target",
        ] {
            let dir = if sub.is_empty() {
                root.clone()
            } else {
                root.join(sub)
            };
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

fn card_status_line(card: &CardSlot) -> String {
    let slot = card.slot;
    match &card.status {
        CardStatus::Ready => format!(
            "SLOT{slot} {} READY  writes {}",
            card.kind.name(),
            card.writes
        ),
        CardStatus::Missing => format!(
            "SLOT{slot} MISSING: no package for ID '{}' (--card {slot}=PATH{})",
            card.required_id(),
            card.kind
                .package_file_name()
                .map(|name| format!(" or {name}"))
                .unwrap_or_default()
        ),
        CardStatus::LoadError(error) => format!("SLOT{slot} LOAD ERROR: {error}"),
        CardStatus::IdMismatch => format!(
            "SLOT{slot} ID MISMATCH: card does not report '{}' at +0x{:X}",
            card.required_id(),
            card.requirement.id_offset
        ),
        CardStatus::Faulted(reason) => format!("SLOT{slot} FAULTED: {reason}"),
    }
}

fn si22_format(field: u8) -> &'static str {
    match field & 7 {
        1 => "PCM16",
        2 => "SQV4H",
        3 => "SQV4L",
        4 => "SQV4+",
        _ => "PCM8 ",
    }
}

fn hit(ui: &Ui, min: [f32; 2], max: [f32; 2]) -> bool {
    let mouse = ui.io().mouse_pos;
    ui.is_mouse_clicked(MouseButton::Left)
        && mouse[0] >= min[0]
        && mouse[0] < max[0]
        && mouse[1] >= min[1]
        && mouse[1] < max[1]
}

fn level_meter(level: f32) -> Meter {
    let mut meter = Meter::default();
    meter.level = level;
    meter
}

/// L/R pan bars in the right column of a track row.
fn draw_pan(draw: &DrawListMut, [mx, y]: [f32; 2], right: f32, (l, r): (u8, u8), active: bool) {
    let pw = right - (mx + 18.0);
    for (row, (label, value)) in [("L", l), ("R", r)].into_iter().enumerate() {
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
            [mx + 18.0 + pw * f32::from(value) / 15.0, py + 7.0],
            if active {
                [0.5, 0.52, 0.55, 1.0]
            } else {
                [0.2, 0.21, 0.22, 1.0]
            },
        )
        .filled(true)
        .build();
    }
}

fn draw_row_label(draw: &DrawListMut, [x, y]: [f32; 2], top: &str, number: usize, dim: bool) {
    draw.add_text([x + 2.0, y + 1.0], LABEL, top);
    draw.add_text([x + 2.0, y + 13.0], LABEL, "CH");
    draw_segments(
        draw,
        [x + 2.0, y + 27.0],
        17.0,
        &format!("{number:02}"),
        if dim { DIM } else { TEXT },
    );
}

fn draw_polyline(draw: &DrawListMut, points: impl Iterator<Item = [f32; 2]>, color: Rgba) {
    let mut prev = None;
    for point in points {
        if let Some(prev) = prev {
            draw.add_line(prev, point, color).build();
        }
        prev = Some(point);
    }
}

/// One 8-operator harmonic channel (3HS816, or the HS part of 3HS88PWN4).
fn draw_hs_track(
    draw: &DrawListMut,
    [x, y]: [f32; 2],
    ch: usize,
    hs: &HsView,
    card_muted: bool,
    selected: bool,
) {
    let loudest = hs.volumes.iter().copied().max().unwrap_or(0);
    let active = hs.gate && !hs.muted && !card_muted && loudest > 0;
    draw.add_rect(
        [x, y],
        [x + 700.0, y + 48.0],
        if selected { SELECTED } else { PANEL },
    )
    .filled(true)
    .build();
    draw_row_label(draw, [x, y], "HS", ch + 1, hs.muted || card_muted);
    let lx = x + 44.0;
    let lamp = if hs.key_flash > 0 {
        KEY_ON
    } else if hs.gate {
        [0.55, 0.1, 0.08, 1.0]
    } else {
        [0.15, 0.15, 0.16, 1.0]
    };
    draw.add_rect([lx, y + 3.0], [lx + 22.0, y + 12.0], lamp)
        .filled(true)
        .build();
    let note = if hs.frequency > 0 {
        note_name(midi_from_hz(f32::from(hs.frequency)))
    } else {
        "---".into()
    };
    let (l, r) = hs.pan_lr();
    let fields = [
        (28.0, format!("NT:{note}")),
        (88.0, format!("FQ:{:5}", hs.frequency)),
        (162.0, format!("AL:{:2}", hs.algorithm)),
        (
            210.0,
            format!("FB:{:3}>{}", hs.feedback, hs.feedback_target + 1),
        ),
        (290.0, format!("PN:{l:X}/{r:X}")),
    ];
    for (dx, text) in fields {
        draw.add_text([lx + dx, y + 1.0], if active { TEXT } else { LABEL }, text);
    }
    let key = (active && hs.frequency > 0).then(|| midi_from_hz(f32::from(hs.frequency)));
    let level = f32::from(loudest) / 255.0;
    draw_keyboard(draw, [lx, y + 18.0], key, level);

    // Operator volumes, OP1..OP8.
    let bx = lx + 342.0;
    let (bw, bh) = (130.0, 26.0);
    let by = y + 18.0;
    draw.add_rect([bx, by], [bx + bw, by + bh], BG)
        .filled(true)
        .build();
    draw.add_rect([bx, by], [bx + bw, by + bh], DIM).build();
    if hs.muted {
        draw.add_rect([bx, y + 2.0], [bx + 36.0, y + 14.0], METER_HI)
            .filled(true)
            .build();
        draw.add_text([bx + 3.0, y + 1.0], BG, "MUTE");
    } else {
        draw.add_text([bx, y + 1.0], LABEL, "OP 1-8");
    }
    let slot_w = (bw - 2.0) / 8.0;
    for (op, &volume) in hs.volumes.iter().enumerate() {
        let ox = bx + 1.0 + op as f32 * slot_w;
        let height = (f32::from(volume) / 255.0 * (bh - 4.0)).round();
        draw.add_rect(
            [ox + 2.0, by + bh - 2.0 - height],
            [ox + slot_w - 2.0, by + bh - 2.0],
            if active { CARD_TAG } else { DIM },
        )
        .filled(true)
        .build();
    }
    let mx = bx + bw + 8.0;
    draw.add_text([mx, y + 1.0], LABEL, "VL");
    let shown = if active { level } else { 0.0 };
    draw_meter(
        draw,
        [mx + 18.0, y + 4.0],
        [x + 696.0, y + 12.0],
        level_meter(shown),
        24,
    );
    draw_pan(draw, [mx, y], x + 696.0, (l, r), active);
}

/// One 3HS88PWN4 PCM/WT channel.
fn draw_wt_track(
    draw: &DrawListMut,
    [x, y]: [f32; 2],
    w: usize,
    wt: &WtView,
    card_muted: bool,
    selected: bool,
    animation: u32,
) {
    let active = wt.volume > 0 && wt.frequency > 0 && wt.mode <= 5 && !wt.muted && !card_muted;
    draw.add_rect(
        [x, y],
        [x + 700.0, y + 48.0],
        if selected { SELECTED } else { PANEL },
    )
    .filled(true)
    .build();
    draw_row_label(draw, [x, y], "WT", w + 1, wt.muted || card_muted);
    let lx = x + 44.0;
    draw.add_text(
        [lx, y + 1.0],
        if active { CARD_TAG } else { DIM },
        wt.mode_name(),
    );
    let pitched = matches!(wt.mode, 1 | 4) && wt.frequency > 0;
    let note = if pitched {
        note_name(midi_from_hz(f32::from(wt.frequency)))
    } else {
        "---".into()
    };
    let (l, r) = if wt.pan == 0 {
        (15, 15)
    } else {
        (wt.pan >> 4, wt.pan & 15)
    };
    let fields = [
        (40.0, format!("NT:{note}")),
        (100.0, format!("FQ:{:5}", wt.frequency)),
        (174.0, format!("VL:{:3}", wt.volume)),
        (
            230.0,
            format!("FL:{} {:3}/{:3}", wt.filter_name(), wt.cutoff, wt.resonance),
        ),
    ];
    for (dx, text) in fields {
        draw.add_text([lx + dx, y + 1.0], if active { TEXT } else { LABEL }, text);
    }
    let key = (active && pitched).then(|| midi_from_hz(f32::from(wt.frequency)));
    let level = f32::from(wt.volume) / 255.0;
    draw_keyboard(draw, [lx, y + 18.0], key, level);

    let bx = lx + 342.0;
    let (bw, bh) = (130.0, 26.0);
    let by = y + 18.0;
    draw.add_rect([bx, by], [bx + bw, by + bh], BG)
        .filled(true)
        .build();
    draw.add_rect([bx, by], [bx + bw, by + bh], DIM).build();
    if wt.muted {
        draw.add_rect([bx, y + 2.0], [bx + 36.0, y + 14.0], METER_HI)
            .filled(true)
            .build();
        draw.add_text([bx + 3.0, y + 1.0], BG, "MUTE");
    }
    let color = if active { CARD_TAG } else { DIM };
    match wt.mode {
        0 => {
            let (start, end, loop_start) = wt.pcm_range();
            draw.add_text([bx + 3.0, by], LABEL, format!("{start:06X}-{end:06X}"));
            draw.add_text([bx + 3.0, by + 12.0], color, format!("LP {loop_start:06X}"));
        }
        1 | 4 => draw_polyline(
            draw,
            wt.table.iter().enumerate().map(|(i, &value)| {
                [
                    bx + 1.0 + i as f32 * (bw - 2.0) / 31.0,
                    by + bh - 1.0 - f32::from(value) / 255.0 * (bh - 2.0),
                ]
            }),
            color,
        ),
        2 | 3 if active => {
            let mut seed = animation.wrapping_mul(2_654_435_761) ^ (w as u32 + 77);
            draw_polyline(
                draw,
                (0..32).map(|i| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    [
                        bx + 1.0 + i as f32 * (bw - 2.0) / 31.0,
                        by + bh - 1.0 - (seed & 0xFF) as f32 / 255.0 * (bh - 2.0),
                    ]
                }),
                color,
            );
        }
        5 => {
            draw.add_text([bx + 3.0, by + 6.0], color, "FIFO DAC");
        }
        _ => {}
    }
    let mx = bx + bw + 8.0;
    draw.add_text([mx, y + 1.0], LABEL, "VL");
    draw_meter(
        draw,
        [mx + 18.0, y + 4.0],
        [x + 696.0, y + 12.0],
        level_meter(if active { level } else { 0.0 }),
        24,
    );
    draw_pan(draw, [mx, y], x + 696.0, (l, r), active);
}

impl Monitor {
    fn draw_si22(&self, draw: &DrawListMut, [x0, y0]: [f32; 2]) {
        let Some(engine) = &self.engine else { return };
        let chip = engine.si22();
        let muted = engine.si22_muted;
        const PLAY: [&str; 4] = ["STOP", "PLAY", "WAIT", "END "];
        const REC: [&str; 4] = ["STOP", "REC ", "WAIT", "END "];
        for ch in 0..2 {
            let y = y0 + ch as f32 * ROW_H;
            let p = &chip.play[ch];
            let running = p.status == 1 && !muted;
            draw.add_rect([x0, y], [x0 + 700.0, y + 48.0], PANEL)
                .filled(true)
                .build();
            draw_row_label(draw, [x0, y], "PLAY", ch + 1, muted);
            let lx = x0 + 44.0;
            draw.add_text(
                [lx, y + 1.0],
                if running { ACCENT } else { LABEL },
                PLAY[usize::from(p.status & 3)],
            );
            let rate = 48_000 / (u32::from(p.div) + 1);
            let fields = [
                (40.0, si22_format(p.format_field).to_string()),
                (96.0, format!("VL:{:2}", p.volume)),
                (150.0, format!("DIV:{:2} {:5} Hz", p.div, rate)),
                (
                    270.0,
                    format!(
                        "OUT:{}{}",
                        if p.l_disable { '-' } else { 'L' },
                        if p.r_disable { '-' } else { 'R' }
                    ),
                ),
            ];
            for (dx, text) in fields {
                draw.add_text([lx + dx, y + 1.0], if running { TEXT } else { LABEL }, text);
            }
            // Playback position between START and STOP.
            let (bx, by, bw) = (lx, y + 18.0, 470.0);
            draw.add_rect([bx, by], [bx + bw, by + 26.0], BG)
                .filled(true)
                .build();
            draw.add_rect([bx, by], [bx + bw, by + 26.0], DIM).build();
            let length = p.stop.saturating_sub(p.start).max(1) as f32;
            let position = chip.play_address(ch).saturating_sub(p.start) as f32;
            let fraction = if p.status == 3 {
                1.0
            } else if p.status == 1 {
                (position / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            draw.add_rect(
                [bx + 1.0, by + 14.0],
                [bx + 1.0 + (bw - 2.0) * fraction, by + 24.0],
                if running { ACCENT } else { DIM },
            )
            .filled(true)
            .build();
            draw.add_text(
                [bx + 4.0, by],
                LABEL,
                format!(
                    "START {:06X}  STOP {:06X}  ADDR {:06X}",
                    p.start,
                    p.stop,
                    chip.play_address(ch)
                ),
            );
            let mx = bx + bw + 8.0;
            for (side, label) in ["L", "R"].into_iter().enumerate() {
                let my = y + 4.0 + side as f32 * 14.0;
                draw.add_text([mx, my - 3.0], LABEL, label);
                draw_meter(
                    draw,
                    [mx + 18.0, my],
                    [x0 + 696.0, my + 8.0],
                    engine.si22_meters[side],
                    24,
                );
            }
            draw.add_text([mx, y + 32.0], DIM, "chip output");
        }
        for ch in 0..2 {
            let y = y0 + (2 + ch) as f32 * ROW_H;
            let rec = &chip.rec[ch];
            let active = rec.status == 1 || rec.status == 2;
            draw.add_rect([x0, y], [x0 + 700.0, y + 48.0], PANEL)
                .filled(true)
                .build();
            draw_row_label(draw, [x0, y], "REC", ch + 1, false);
            let lx = x0 + 44.0;
            let rate = 48_000 / (u32::from(rec.div) + 1);
            let fields = [
                (0.0, REC[usize::from(rec.status & 3)].to_string()),
                (40.0, si22_format(rec.format_field).to_string()),
                (96.0, format!("GAIN:{:2}", rec.gain)),
                (170.0, format!("DIV:{:2} {:5} Hz", rec.div, rate)),
                (
                    290.0,
                    format!("AGC:{}", if chip.agc_enable[ch] { "ON" } else { "--" }),
                ),
            ];
            for (dx, text) in fields {
                draw.add_text([lx + dx, y + 1.0], if active { TEXT } else { LABEL }, text);
            }
            draw.add_text(
                [lx, y + 22.0],
                LABEL,
                format!(
                    "START {:06X}  STOP {:06X}  (ADC {})",
                    rec.start,
                    rec.stop,
                    if ch == 0 { "L" } else { "R" }
                ),
            );
        }
        let y = y0 + 4.0 * ROW_H + 4.0;
        draw.add_text(
            [x0, y],
            LABEL,
            format!(
                "STATUS_CTRL flags {:02X}   VOX threshold {:3}  hold {:3}{}",
                chip.flags,
                chip.vox_threshold,
                chip.vox_hold,
                if muted { "   MUTED (M)" } else { "" }
            ),
        );
        draw.add_text(
            [x0, y + 16.0],
            DIM,
            "34SI22 plays from the shared PCM RAM; the stream writes its registers as target 11.",
        );
    }

    fn draw_card(&mut self, ui: &Ui, draw: &DrawListMut, [x0, y0]: [f32; 2], index: usize) {
        let Some(engine) = &self.engine else { return };
        let Some(card) = engine.cards.get(index) else {
            return;
        };
        if !card.is_ready() {
            draw.add_rect([x0, y0], [x0 + 700.0, y0 + 110.0], PANEL)
                .filled(true)
                .build();
            draw.add_text(
                [x0 + 12.0, y0 + 10.0],
                TEXT,
                format!(
                    "SLOT {}  requires ID '{}' at +0x{:X}",
                    card.slot,
                    card.required_id(),
                    card.requirement.id_offset
                ),
            );
            draw.add_text([x0 + 12.0, y0 + 30.0], METER_HI, card_status_line(card));
            if let Some(path) = &card.path {
                draw.add_text(
                    [x0 + 12.0, y0 + 50.0],
                    LABEL,
                    format!("package: {}", path.display()),
                );
            }
            draw.add_text(
                [x0 + 12.0, y0 + 70.0],
                LABEL,
                "Writes to this slot are dropped; SGU and 34SI22 keep playing.",
            );
            draw.add_text(
                [x0 + 12.0, y0 + 86.0],
                LABEL,
                "Load a card with --card SLOT=PACKAGE, or place the package next to the .sgub.",
            );
            return;
        }
        let mut clicked = None;
        match card.kind {
            CardKind::Hs816 | CardKind::Hs88pwn4 => {
                for ch in 0..card.kind.hs_channels() {
                    let y = y0 + ch as f32 * ROW_H;
                    draw_hs_track(
                        draw,
                        [x0, y],
                        ch,
                        &card.hs(ch),
                        card.muted,
                        self.selected == ch,
                    );
                    if hit(ui, [x0, y], [x0 + 700.0, y + 48.0]) {
                        clicked = Some(ch);
                    }
                }
                if card.kind == CardKind::Hs88pwn4 {
                    for w in 0..4 {
                        let y = y0 + (8 + w) as f32 * ROW_H;
                        draw_wt_track(
                            draw,
                            [x0, y],
                            w,
                            &card.wt(w),
                            card.muted,
                            self.selected == 16 + w,
                            self.animation,
                        );
                        if hit(ui, [x0, y], [x0 + 700.0, y + 48.0]) {
                            clicked = Some(16 + w);
                        }
                    }
                    let y = y0 + 12.0 * ROW_H + 4.0;
                    let m = |i: u32| card.reg(0x4002C0 + i);
                    draw.add_text(
                        [x0, y],
                        LABEL,
                        format!(
                            "MASTER  VOL {:3}   COMP {} THR {:3} SPD {:3}   EQ {} L {:3} M {:3} H {:3}{}",
                            m(4),
                            if m(0) == 1 { "ON " } else { "OFF" },
                            m(2),
                            m(3),
                            if m(1) == 1 { "ON " } else { "OFF" },
                            m(5),
                            m(6),
                            m(7),
                            if card.muted { "   MUTED (M)" } else { "" }
                        ),
                    );
                }
            }
            CardKind::Unknown => {
                draw.add_rect([x0, y0], [x0 + 700.0, y0 + 60.0], PANEL)
                    .filled(true)
                    .build();
                draw.add_text(
                    [x0 + 12.0, y0 + 10.0],
                    TEXT,
                    format!(
                        "SLOT {}: card ID '{}' has no monitor layout",
                        card.slot,
                        card.required_id()
                    ),
                );
                draw.add_text(
                    [x0 + 12.0, y0 + 30.0],
                    LABEL,
                    format!("It plays normally; {} writes delivered.", card.writes),
                );
            }
        }
        if let Some(channel) = clicked {
            self.selected = channel;
        }
    }
}

fn draw_hex_rows(
    draw: &DrawListMut,
    [x, y]: [f32; 2],
    base: u32,
    length: u32,
    read: impl Fn(u32) -> u8,
) -> f32 {
    let mut row_y = y;
    for row in (0..length).step_by(16) {
        let bytes: Vec<String> = (row..(row + 16).min(length))
            .map(|offset| format!("{:02X}", read(base + offset)))
            .collect();
        draw.add_text(
            [x, row_y],
            TEXT,
            format!("{:06X}: {}", base + row, bytes.join(" ")),
        );
        row_y += 14.0;
    }
    row_y
}

fn draw_si22_registers(draw: &DrawListMut, [x, y]: [f32; 2], engine: &Engine) {
    draw.add_text([x + 90.0, y], LABEL, "34SI22  0x80022000 + offset");
    let chip = engine.si22();
    let end = draw_hex_rows(draw, [x + 20.0, y + 20.0], 0, 0x30, |offset| {
        chip.read_register(offset)
    });
    draw.add_text(
        [x + 20.0, end + 6.0],
        LABEL,
        "00-0B PLAY START/STOP  0C PLAY_CTRL  0D VOL  0E/0F DIV",
    );
    draw.add_text(
        [x + 20.0, end + 20.0],
        LABEL,
        "10-1B REC START/STOP   1C REC_CTRL   1D GAIN 20 STATUS 21/22 FORMAT",
    );
}

fn draw_card_registers(
    draw: &DrawListMut,
    [x, y]: [f32; 2],
    engine: &Engine,
    index: usize,
    selected: usize,
) {
    let Some(card) = engine.cards.get(index) else {
        return;
    };
    draw.add_text(
        [x + 90.0, y],
        LABEL,
        format!(
            "SLOT{} {}  writes {}  dropped {}{}",
            card.slot,
            card.kind.name(),
            card.writes,
            card.dropped,
            if card.muted { "  MUTED" } else { "" }
        ),
    );
    for (side, label) in ["L", "R"].into_iter().enumerate() {
        let my = y + 18.0 + side as f32 * 12.0;
        draw.add_text([x + 20.0, my - 3.0], LABEL, label);
        draw_meter(
            draw,
            [x + 34.0, my],
            [x + 596.0, my + 8.0],
            card.meters[side],
            48,
        );
    }
    if !card.is_ready() || card.kind == CardKind::Unknown {
        if let Some(path) = &card.path {
            draw.add_text([x + 20.0, y + 46.0], LABEL, path.display().to_string());
        }
        return;
    }
    let wt = selected >= 16 && card.kind == CardKind::Hs88pwn4;
    let (title, base, length) = if wt {
        let w = selected - 16;
        (
            format!("WT{} REGISTER", w + 1),
            0x400200 + 48 * w as u32,
            48,
        )
    } else {
        let ch = selected.min(card.kind.hs_channels() - 1);
        let base = match card.kind {
            CardKind::Hs88pwn4 => 0x400000,
            _ => 0,
        } + 0x40 * ch as u32;
        (
            format!("HS{} REGISTER (click a row to select)", ch + 1),
            base,
            64,
        )
    };
    draw.add_text([x + 20.0, y + 44.0], ACCENT, title);
    let end = draw_hex_rows(draw, [x + 20.0, y + 60.0], base, length, |a| card.reg(a));
    if wt {
        return;
    }
    // Decoded operators of the selected HS channel.
    draw.add_text(
        [x + 20.0, end + 4.0],
        LABEL,
        "OP  RATIO   VL WV   A   D   S   R",
    );
    for op in 0..8u32 {
        let ratio = if op == 0 {
            4096
        } else {
            card.reg16(base + 2 * op)
        };
        let wave_byte = card.reg(base + 0x18 + op / 2);
        let wave = if op % 2 == 0 {
            wave_byte >> 4
        } else {
            wave_byte & 15
        };
        let adsr = base + 0x20 + op * 4;
        draw.add_text(
            [x + 20.0, end + 18.0 + op as f32 * 13.0],
            TEXT,
            format!(
                "{}  {:6.3} {:3} {:2} {:3} {:3} {:3} {:3}",
                op + 1,
                f32::from(ratio) / 4096.0,
                card.reg(base + 0x10 + op),
                wave,
                card.reg(adsr),
                card.reg(adsr + 1),
                card.reg(adsr + 2),
                card.reg(adsr + 3)
            ),
        );
    }
}
