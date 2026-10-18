// Main entry point for the CPT32 emulator

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::env;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cpt32::bus::Bus;
use cpt32::cpu::InstructionMode;
use cpt32::cpu::{CPU_PROFILE_STAGE_NAMES, CpuProfile};
use cpt32::cpu_select::{AnyCpu, CpuKind, resolve_cpu_kind, split_cpu_arg};
use cpt32::devices::pec::idc::idc::connect_idc_from_config;
use cpt32::devices::pec::pec::{HostInputEvent, connect_pec_input, pec_trace_enabled};
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::pec::serial::connect_uart;
use cpt32::devices::ram::connect_ram;
use cpt32::devices::sgu::s3w2::S3w2Sound;
use cpt32::devices::sgu::sgu::connect_sgu;
use cpt32::devices::vdp::vdp::{Vdp, connect_vdp};
use cpt32::music::guest::{STREAM_BASE, read_music_stream};
use gilrs::{
    Axis as GilrsAxis, Button as GilrsButton, EventType as GilrsEventType, GamepadId, Gilrs,
};
use imgui::{Condition, ProgressBar, StyleColor, TableFlags, Ui};
use imgui_wgpu::{Renderer, RendererConfig};
use imgui_winit_support::{HiDpiMode, WinitPlatform};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{DeviceEvent, ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowAttributes};

mod audio;
mod monitor;
mod render;

const WIDTH: u32 = render::PRESENT_WIDTH;
const HEIGHT: u32 = render::PRESENT_HEIGHT;
const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

fn load_program_data(
    data: &[u8],
    music_stream: Option<&Path>,
    bus: &mut Bus,
) -> Result<u32, String> {
    let entry = cpt32::wax::load_bytes(bus, data)?;
    let stream = music_stream
        .map(|path| read_music_stream(path, data.len()))
        .transpose()
        .map_err(|error| format!("Music stream startup error: {error}"))?;
    if let Some(stream) = stream {
        for (offset, value) in stream.into_iter().enumerate() {
            bus.write_u8(STREAM_BASE + offset as u32, value);
        }
    }
    tracing::info!(
        "Loaded program data: {} bytes, music stream: {}",
        data.len(),
        music_stream
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".to_owned())
    );
    Ok(entry)
}

struct GuestMachine {
    cpu: AnyCpu,
    vdp: Rc<RefCell<Vdp>>,
    sgu: [Rc<RefCell<S3w2Sound>>; 2],
    idc: Option<Rc<RefCell<cpt32::devices::pec::idc::idc::Idc>>>,
    input_sender: tokio::sync::mpsc::UnboundedSender<HostInputEvent>,
    title: Option<String>,
}

fn is_cartridge_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cpt32pkg"))
}

fn build_guest_machine(
    program_path: &Path,
    music_stream: Option<&Path>,
    user_data_root: Option<&Path>,
    cpu_kind: CpuKind,
) -> Result<GuestMachine, String> {
    let cartridge = if is_cartridge_path(program_path) {
        Some(cpt32::cartridge::Cartridge::load(program_path)?)
    } else {
        None
    };
    let mut bus = Bus::new();
    let input_sender = connect_pec_input(&mut bus);
    connect_ram(&mut bus);
    cpt32::bus::connect_bmc_dmac(&mut bus);
    connect_uart(&mut bus);
    connect_rng(&mut bus);
    let idc = if let Some(cartridge) = cartridge.as_ref() {
        let root = if cartridge
            .disks
            .iter()
            .any(|disk| disk.mode == cpt32::cartridge::CartridgeDiskMode::ReadWrite)
        {
            let user_data_root = match user_data_root {
                Some(root) => root.to_path_buf(),
                None => cpt32::cartridge::default_user_data_root()?,
            };
            Some(user_data_root.join(&cartridge.id))
        } else {
            None
        };
        let drives = cpt32::devices::pec::idc::idc::Idc::load_cartridge_drives(
            &cartridge.disks,
            root.as_deref().unwrap_or_else(|| Path::new(".")),
        )?;
        let idc = cpt32::devices::pec::idc::idc::connect_idc(&mut bus, drives);
        // Cartridge disks carry no sound assignment, so only the kind
        // defaults apply. A broken sound setup only warns, never fails boot.
        match cpt32::devices::pec::idc::idc::Idc::load_sound_config(Path::new("config.yaml")) {
            Ok(sound) => idc.borrow_mut().apply_sound_config(&sound),
            Err(error) => tracing::warn!(%error, "IDC audio: ignoring sound configuration"),
        }
        idc
    } else {
        connect_idc_from_config(&mut bus)?
    };
    let sgu = connect_sgu(&mut bus);
    let program = match cartridge.as_ref() {
        Some(cartridge) => cartridge.program.as_slice(),
        None => {
            return build_raw_guest(
                program_path,
                music_stream,
                bus,
                input_sender,
                sgu,
                idc,
                cpu_kind,
            );
        }
    };
    let entry = load_program_data(program, music_stream, &mut bus)?;
    let vdp = connect_vdp(&mut bus);
    if let Some(font_path) = pcg_font_path_from_config()? {
        vdp.borrow()
            .load_pcg_font_from_file(&font_path)
            .map_err(|error| format!("Failed to load PCG font {}: {error}", font_path.display()))?;
    }
    let plugins = cartridge
        .as_ref()
        .unwrap()
        .plugins
        .iter()
        .map(|plugin| cpt32::devices::expansion::ExpansionCardPackage {
            slot: plugin.slot,
            label: plugin.path.clone(),
            bytes: plugin.package.clone(),
        })
        .collect();
    cpt32::devices::expansion::load_expansion_cards_from_bytes(plugins, &mut bus)?;
    let entry = match cpt32::devices::bios::load_bios(Path::new("config.yaml"), &mut bus)? {
        Some(bios_entry) => bios_entry,
        None => entry,
    };
    let title = cartridge
        .as_ref()
        .map(|cartridge| format!("CPT32 — {}", cartridge.title));
    let mut cpu = AnyCpu::new(cpu_kind, bus);
    cpu.set_pc(entry);
    Ok(GuestMachine {
        cpu,
        vdp,
        sgu,
        idc: Some(idc),
        input_sender,
        title,
    })
}

fn build_raw_guest(
    program_path: &Path,
    music_stream: Option<&Path>,
    mut bus: Bus,
    input_sender: tokio::sync::mpsc::UnboundedSender<HostInputEvent>,
    sgu: [Rc<RefCell<S3w2Sound>>; 2],
    idc: Rc<RefCell<cpt32::devices::pec::idc::idc::Idc>>,
    cpu_kind: CpuKind,
) -> Result<GuestMachine, String> {
    let data = std::fs::read(program_path).map_err(|error| {
        format!(
            "Failed to read binary file {}: {error}",
            program_path.display()
        )
    })?;
    let entry = load_program_data(&data, music_stream, &mut bus)?;
    let vdp = connect_vdp(&mut bus);
    if let Some(font_path) = pcg_font_path_from_config()? {
        vdp.borrow()
            .load_pcg_font_from_file(&font_path)
            .map_err(|error| format!("Failed to load PCG font {}: {error}", font_path.display()))?;
    }
    cpt32::devices::expansion::load_expansion_cards(Path::new("config.yaml"), &mut bus)?;
    let entry = match cpt32::devices::bios::load_bios(Path::new("config.yaml"), &mut bus)? {
        Some(bios_entry) => bios_entry,
        None => entry,
    };
    let mut cpu = AnyCpu::new(cpu_kind, bus);
    cpu.set_pc(entry);
    Ok(GuestMachine {
        cpu,
        vdp,
        sgu,
        idc: Some(idc),
        input_sender,
        title: None,
    })
}

fn keyboard_hid_usage(code: KeyCode) -> Option<u8> {
    Some(match code {
        KeyCode::KeyA => 0x04,
        KeyCode::KeyB => 0x05,
        KeyCode::KeyC => 0x06,
        KeyCode::KeyD => 0x07,
        KeyCode::KeyE => 0x08,
        KeyCode::KeyF => 0x09,
        KeyCode::KeyG => 0x0A,
        KeyCode::KeyH => 0x0B,
        KeyCode::KeyI => 0x0C,
        KeyCode::KeyJ => 0x0D,
        KeyCode::KeyK => 0x0E,
        KeyCode::KeyL => 0x0F,
        KeyCode::KeyM => 0x10,
        KeyCode::KeyN => 0x11,
        KeyCode::KeyO => 0x12,
        KeyCode::KeyP => 0x13,
        KeyCode::KeyQ => 0x14,
        KeyCode::KeyR => 0x15,
        KeyCode::KeyS => 0x16,
        KeyCode::KeyT => 0x17,
        KeyCode::KeyU => 0x18,
        KeyCode::KeyV => 0x19,
        KeyCode::KeyW => 0x1A,
        KeyCode::KeyX => 0x1B,
        KeyCode::KeyY => 0x1C,
        KeyCode::KeyZ => 0x1D,
        KeyCode::Digit1 => 0x1E,
        KeyCode::Digit2 => 0x1F,
        KeyCode::Digit3 => 0x20,
        KeyCode::Digit4 => 0x21,
        KeyCode::Digit5 => 0x22,
        KeyCode::Digit6 => 0x23,
        KeyCode::Digit7 => 0x24,
        KeyCode::Digit8 => 0x25,
        KeyCode::Digit9 => 0x26,
        KeyCode::Digit0 => 0x27,
        KeyCode::Enter => 0x28,
        KeyCode::Escape => 0x29,
        KeyCode::Backspace => 0x2A,
        KeyCode::Tab => 0x2B,
        KeyCode::Space => 0x2C,
        KeyCode::Minus => 0x2D,
        KeyCode::Equal => 0x2E,
        KeyCode::BracketLeft => 0x2F,
        KeyCode::BracketRight => 0x30,
        KeyCode::Backslash => 0x31,
        KeyCode::Semicolon => 0x33,
        KeyCode::Quote => 0x34,
        KeyCode::Backquote => 0x35,
        KeyCode::Comma => 0x36,
        KeyCode::Period => 0x37,
        KeyCode::Slash => 0x38,
        KeyCode::CapsLock => 0x39,
        KeyCode::F1 => 0x3A,
        KeyCode::F2 => 0x3B,
        KeyCode::F3 => 0x3C,
        KeyCode::F4 => 0x3D,
        KeyCode::F5 => 0x3E,
        KeyCode::F6 => 0x3F,
        KeyCode::F7 => 0x40,
        KeyCode::F8 => 0x41,
        KeyCode::F9 => 0x42,
        KeyCode::F10 => 0x43,
        KeyCode::F11 => 0x44,
        KeyCode::F12 => 0x45,
        KeyCode::PrintScreen => 0x46,
        KeyCode::ScrollLock => 0x47,
        KeyCode::Pause => 0x48,
        KeyCode::Insert => 0x49,
        KeyCode::Home => 0x4A,
        KeyCode::PageUp => 0x4B,
        KeyCode::Delete => 0x4C,
        KeyCode::End => 0x4D,
        KeyCode::PageDown => 0x4E,
        KeyCode::ArrowRight => 0x4F,
        KeyCode::ArrowLeft => 0x50,
        KeyCode::ArrowDown => 0x51,
        KeyCode::ArrowUp => 0x52,
        KeyCode::NumLock => 0x53,
        KeyCode::NumpadDivide => 0x54,
        KeyCode::NumpadMultiply => 0x55,
        KeyCode::NumpadSubtract => 0x56,
        KeyCode::NumpadAdd => 0x57,
        KeyCode::NumpadEnter => 0x58,
        KeyCode::Numpad1 => 0x59,
        KeyCode::Numpad2 => 0x5A,
        KeyCode::Numpad3 => 0x5B,
        KeyCode::Numpad4 => 0x5C,
        KeyCode::Numpad5 => 0x5D,
        KeyCode::Numpad6 => 0x5E,
        KeyCode::Numpad7 => 0x5F,
        KeyCode::Numpad8 => 0x60,
        KeyCode::Numpad9 => 0x61,
        KeyCode::Numpad0 => 0x62,
        KeyCode::NumpadDecimal => 0x63,
        KeyCode::NumpadComma => 0x85,
        KeyCode::NumpadParenLeft => 0xB6,
        KeyCode::NumpadParenRight => 0xB7,
        KeyCode::NumpadBackspace => 0xBB,
        KeyCode::IntlBackslash => 0x64,
        KeyCode::ContextMenu => 0x65,
        KeyCode::Power => 0x66,
        KeyCode::NumpadEqual => 0x67,
        KeyCode::NumpadMemoryStore => 0xD0,
        KeyCode::NumpadMemoryRecall => 0xD1,
        KeyCode::NumpadMemoryClear => 0xD2,
        KeyCode::NumpadMemoryAdd => 0xD3,
        KeyCode::NumpadMemorySubtract => 0xD4,
        KeyCode::NumpadClear => 0xD8,
        KeyCode::NumpadClearEntry => 0xD9,
        KeyCode::F13 => 0x68,
        KeyCode::F14 => 0x69,
        KeyCode::F15 => 0x6A,
        KeyCode::F16 => 0x6B,
        KeyCode::F17 => 0x6C,
        KeyCode::F18 => 0x6D,
        KeyCode::F19 => 0x6E,
        KeyCode::F20 => 0x6F,
        KeyCode::F21 => 0x70,
        KeyCode::F22 => 0x71,
        KeyCode::F23 => 0x72,
        KeyCode::F24 => 0x73,
        KeyCode::Help => 0x75,
        KeyCode::Select => 0x77,
        KeyCode::Again => 0x79,
        KeyCode::Undo => 0x7A,
        KeyCode::Cut => 0x7B,
        KeyCode::Copy => 0x7C,
        KeyCode::Paste => 0x7D,
        KeyCode::Find => 0x7E,
        KeyCode::AudioVolumeMute => 0x7F,
        KeyCode::AudioVolumeUp => 0x80,
        KeyCode::AudioVolumeDown => 0x81,
        KeyCode::IntlRo => 0x87,
        KeyCode::KanaMode => 0x88,
        KeyCode::IntlYen => 0x89,
        KeyCode::Convert => 0x8A,
        KeyCode::NonConvert => 0x8B,
        KeyCode::Lang1 => 0x90,
        KeyCode::Lang2 => 0x91,
        KeyCode::Lang3 => 0x92,
        KeyCode::Lang4 => 0x93,
        KeyCode::Lang5 => 0x94,
        KeyCode::ControlLeft => 0xE0,
        KeyCode::ShiftLeft => 0xE1,
        KeyCode::AltLeft => 0xE2,
        KeyCode::SuperLeft => 0xE3,
        KeyCode::ControlRight => 0xE4,
        KeyCode::ShiftRight => 0xE5,
        KeyCode::AltRight => 0xE6,
        KeyCode::SuperRight => 0xE7,
        _ => return None,
    })
}

/// Guest HID input for a window key event: the usage id and whether the key went
/// down, or `None` when the event is not user input.
///
/// winit synthesizes a key state snapshot (`is_synthetic`) for every key the OS
/// reports as pressed when a window gains focus, and a release snapshot when it
/// loses focus (`WM_SETFOCUS` / `WM_KILLFOCUS`). An active IME keeps its own
/// virtual keys (`VK_DBE_ALPHANUMERIC`, `VK_DBE_SBCSCHAR`, ...) reported as
/// pressed, and Windows maps those through the layout onto unrelated scancodes,
/// so focusing the window used to deliver a phantom `Backquote` (HID usage
/// 0x35, `` ` `` in US layouts) press to the guest. A snapshot is not a physical
/// transition, so it must not reach the PeC FIFO.
fn guest_key_input(
    physical_key: &PhysicalKey,
    state: ElementState,
    is_synthetic: bool,
) -> Option<(u8, bool)> {
    if is_synthetic {
        return None;
    }
    let PhysicalKey::Code(code) = physical_key else {
        return None;
    };
    keyboard_hid_usage(*code).map(|usage| (usage, state == ElementState::Pressed))
}

fn debug_menu_reserve_px(gui: Option<&DebugGui>) -> f64 {
    gui.map(|gui| {
        let scale = f64::from(gui.imgui.io().display_framebuffer_scale[1]);
        (f64::from(gui.last_menu_bar_height()) * scale).max(0.0)
    })
    .unwrap_or(0.0)
}

fn mouse_position_to_guest(
    position: PhysicalPosition<f64>,
    size: PhysicalSize<u32>,
    top_reserve_px: f64,
) -> (u16, u16) {
    let (scale, viewport_x, viewport_y) = render::game_viewport(size, top_reserve_px);
    if scale == 0.0 {
        return (0, 0);
    }
    let x = ((position.x - viewport_x) / scale - f64::from(render::BORDER_SIZE))
        .floor()
        .clamp(0.0, f64::from(render::FRAME_WIDTH - 1)) as u16;
    let y = ((position.y - viewport_y) / scale - f64::from(render::BORDER_SIZE))
        .floor()
        .clamp(0.0, f64::from(render::FRAME_HEIGHT - 1)) as u16;
    (x, y)
}

fn normalize_wheel(value: f64, pixel_delta: bool) -> i16 {
    let units = if pixel_delta {
        value * 1.2
    } else {
        value * 120.0
    };
    units
        .round()
        .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

fn normalize_stick(value: f32) -> i16 {
    let scaled = if value < 0.0 {
        value * 32768.0
    } else {
        value * 32767.0
    };
    scaled.round().clamp(-32768.0, 32767.0) as i16
}

fn normalize_trigger(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65535.0).round() as u16
}

const GAMEPAD_BUTTON_MAP: [(GilrsButton, u8); 14] = [
    (GilrsButton::South, 0),
    (GilrsButton::East, 1),
    (GilrsButton::West, 2),
    (GilrsButton::North, 3),
    (GilrsButton::DPadUp, 4),
    (GilrsButton::DPadDown, 5),
    (GilrsButton::DPadLeft, 6),
    (GilrsButton::DPadRight, 7),
    (GilrsButton::Select, 8),
    (GilrsButton::Start, 9),
    (GilrsButton::LeftTrigger, 10),
    (GilrsButton::RightTrigger, 11),
    (GilrsButton::LeftThumb, 12),
    (GilrsButton::RightThumb, 13),
];

fn gamepad_button_index(button: GilrsButton) -> Option<u8> {
    GAMEPAD_BUTTON_MAP
        .iter()
        .find_map(|(mapped, bit)| (*mapped == button).then_some(*bit))
}

fn send_gamepad_snapshot(
    sender: &tokio::sync::mpsc::UnboundedSender<HostInputEvent>,
    slot: u8,
    gamepad: &gilrs::Gamepad<'_>,
) {
    let _ = sender.send(HostInputEvent::PadConnected { slot });
    for (button, bit) in GAMEPAD_BUTTON_MAP {
        if gamepad.is_pressed(button) {
            let _ = sender.send(HostInputEvent::PadButton {
                slot,
                button: bit,
                down: true,
            });
        }
    }
    for (axis, gilrs_axis) in [
        GilrsAxis::LeftStickX,
        GilrsAxis::LeftStickY,
        GilrsAxis::RightStickX,
        GilrsAxis::RightStickY,
    ]
    .into_iter()
    .enumerate()
    {
        let value = gamepad.value(gilrs_axis);
        let value = if axis == 1 || axis == 3 {
            -value
        } else {
            value
        };
        let _ = sender.send(HostInputEvent::PadAxis {
            slot,
            axis: axis as u8,
            value: normalize_stick(value),
        });
    }
    for (trigger, button) in [GilrsButton::LeftTrigger2, GilrsButton::RightTrigger2]
        .into_iter()
        .enumerate()
    {
        let value = gamepad.button_data(button).map_or(0.0, |data| data.value());
        let _ = sender.send(HostInputEvent::PadTrigger {
            slot,
            trigger: trigger as u8,
            value: normalize_trigger(value),
        });
    }
}

fn initial_gamepad_slots(
    gilrs: &Gilrs,
    sender: &tokio::sync::mpsc::UnboundedSender<HostInputEvent>,
) -> [Option<GamepadId>; 4] {
    let mut slots = [None; 4];
    for (id, gamepad) in gilrs.gamepads() {
        if let Some(slot) = slots.iter().position(Option::is_none) {
            slots[slot] = Some(id);
            send_gamepad_snapshot(sender, slot as u8, &gamepad);
        }
    }
    slots
}

fn send_mouse_button_releases(
    sender: &tokio::sync::mpsc::UnboundedSender<HostInputEvent>,
    held: &mut u8,
) {
    for button in 0..5 {
        let mask = 1 << button;
        if *held & mask != 0 {
            let _ = sender.send(HostInputEvent::MouseButton {
                button,
                down: false,
            });
            *held &= !mask;
        }
    }
}

fn enqueue_held_input_releases(
    sender: &tokio::sync::mpsc::UnboundedSender<HostInputEvent>,
    keys: &mut HashSet<u8>,
    mouse_buttons: &mut u8,
) {
    for usage in keys.drain() {
        let _ = sender.send(HostInputEvent::Key { usage, down: false });
    }
    send_mouse_button_releases(sender, mouse_buttons);
}

fn set_mouse_capture(captured: &mut bool, window: &Window, enable: bool) {
    if enable {
        match window.set_cursor_grab(CursorGrabMode::Locked) {
            Ok(()) => {
                *captured = true;
                window.set_cursor_visible(false);
            }
            Err(error) => {
                tracing::warn!(%error, "Failed to capture mouse cursor");
                *captured = false;
                let _ = window.set_cursor_grab(CursorGrabMode::None);
                window.set_cursor_visible(true);
            }
        }
    } else {
        if *captured {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
        }
        *captured = false;
        window.set_cursor_visible(true);
    }
}

struct GuiApp {
    cpu: AnyCpu,
    vdp: Rc<RefCell<Vdp>>,
    sgu: [Rc<RefCell<S3w2Sound>>; 2],
    idc: Option<Rc<RefCell<cpt32::devices::pec::idc::idc::Idc>>>,
    audio_host: Option<audio::AudioHost>,
    audio_left: Vec<i16>,
    audio_right: Vec<i16>,
    input_sender: tokio::sync::mpsc::UnboundedSender<HostInputEvent>,
    held_keys: HashSet<u8>,
    held_mouse_buttons: u8,
    mouse_captured: bool,
    focused: bool,
    gilrs: Option<Gilrs>,
    gamepad_slots: [Option<GamepadId>; 4],
    display_handle: Option<OwnedDisplayHandle>,
    window: Option<Window>,
    presenter: Option<render::WgpuPresenter>,
    debug_gui: Option<DebugGui>,
    frame_profiler: Option<FrameProfiler>,
    cpu_kind: CpuKind,
    enable_debug_gui: bool,
    start_paused: bool,
    music_stream: Option<PathBuf>,
    title: String,
    next_frame_deadline: Instant,
}

impl GuiApp {
    fn new(
        program_path: &str,
        display_handle: OwnedDisplayHandle,
        enable_debug_gui: bool,
        profile_cpu: bool,
        start_paused: bool,
        music_stream: Option<&Path>,
        cpu_kind: CpuKind,
    ) -> Result<Self, String> {
        let mut machine =
            build_guest_machine(Path::new(program_path), music_stream, None, cpu_kind)?;
        machine.cpu.enable_profiling(profile_cpu);
        let (gilrs, gamepad_slots) = match Gilrs::new() {
            Ok(gilrs) => {
                let slots = initial_gamepad_slots(&gilrs, &machine.input_sender);
                (Some(gilrs), slots)
            }
            Err(error) => {
                tracing::warn!(%error, "Failed to initialize gamepad input");
                (None, [None; 4])
            }
        };
        let audio_host = match audio::AudioHost::new() {
            Ok(host) => Some(host),
            Err(error) => {
                tracing::warn!(%error, "Failed to initialize audio host");
                None
            }
        };
        Ok(Self {
            cpu: machine.cpu,
            vdp: machine.vdp,
            sgu: machine.sgu,
            idc: machine.idc,
            audio_host,
            audio_left: Vec::new(),
            audio_right: Vec::new(),
            input_sender: machine.input_sender,
            held_keys: HashSet::new(),
            held_mouse_buttons: 0,
            mouse_captured: false,
            focused: true,
            gilrs,
            gamepad_slots,
            display_handle: Some(display_handle),
            window: None,
            presenter: None,
            debug_gui: None,
            frame_profiler: profile_cpu.then(FrameProfiler::new),
            cpu_kind,
            enable_debug_gui,
            start_paused,
            music_stream: music_stream.map(Path::to_path_buf),
            title: machine.title.unwrap_or_else(|| "CPT32".to_owned()),
            next_frame_deadline: Instant::now(),
        })
    }
    fn release_mouse_buttons(&mut self) {
        send_mouse_button_releases(&self.input_sender, &mut self.held_mouse_buttons);
    }

    fn release_all_inputs(&mut self) {
        enqueue_held_input_releases(
            &self.input_sender,
            &mut self.held_keys,
            &mut self.held_mouse_buttons,
        );
    }

    fn handle_dropped_file(&mut self, path: &Path) {
        if !is_cartridge_path(path) {
            return;
        }
        match build_guest_machine(path, self.music_stream.as_deref(), None, self.cpu_kind) {
            Ok(machine) => {
                self.release_all_inputs();
                self.cpu = machine.cpu;
                self.vdp = machine.vdp;
                self.sgu = machine.sgu;
                self.idc = machine.idc;
                self.input_sender = machine.input_sender;
                if let Some(gilrs) = self.gilrs.as_ref() {
                    for (slot, id) in self.gamepad_slots.iter().enumerate() {
                        if let Some(id) = id {
                            if let Some(gamepad) = gilrs.connected_gamepad(*id) {
                                send_gamepad_snapshot(&self.input_sender, slot as u8, &gamepad);
                            }
                        }
                    }
                }
                self.title = machine.title.unwrap_or_else(|| "CPT32".to_owned());
                if let Some(window) = self.window.as_ref() {
                    window.set_title(&self.title);
                }
            }
            Err(error) => {
                tracing::error!(%error, "Failed to load dropped cartridge");
                self.title = format!("CPT32 — Failed to load {}", path.display());
                if let Some(window) = self.window.as_ref() {
                    window.set_title(&self.title);
                }
            }
        }
    }

    fn honor_imgui_mouse_capture(&mut self) {
        let wants_capture = self
            .debug_gui
            .as_ref()
            .is_some_and(|gui| gui.imgui.io().want_capture_mouse);
        if wants_capture {
            if let Some(window) = self.window.as_ref() {
                set_mouse_capture(&mut self.mouse_captured, window, false);
            }
            self.release_mouse_buttons();
        }
    }

    fn fill_gamepad_slots(&mut self) {
        let ids: Vec<GamepadId> = self
            .gilrs
            .as_ref()
            .map(|gilrs| gilrs.gamepads().map(|(id, _)| id).collect())
            .unwrap_or_default();
        for id in ids {
            if self.gamepad_slots.contains(&Some(id)) {
                continue;
            }
            let Some(slot) = self.gamepad_slots.iter().position(Option::is_none) else {
                break;
            };
            self.gamepad_slots[slot] = Some(id);
            if let Some(gamepad) = self
                .gilrs
                .as_ref()
                .and_then(|gilrs| gilrs.connected_gamepad(id))
            {
                send_gamepad_snapshot(&self.input_sender, slot as u8, &gamepad);
            }
        }
    }

    fn poll_gamepads(&mut self) {
        loop {
            let event = match self.gilrs.as_mut() {
                Some(gilrs) => gilrs.next_event(),
                None => return,
            };
            let Some(event) = event else { break };
            let slot = self
                .gamepad_slots
                .iter()
                .position(|id| *id == Some(event.id))
                .map(|index| index as u8);
            match event.event {
                GilrsEventType::Connected => self.fill_gamepad_slots(),
                GilrsEventType::Disconnected => {
                    if let Some(slot) = slot {
                        let _ = self
                            .input_sender
                            .send(HostInputEvent::PadDisconnected { slot });
                        self.gamepad_slots[usize::from(slot)] = None;
                    }
                    self.fill_gamepad_slots();
                }
                GilrsEventType::ButtonPressed(button, _) => {
                    if let Some(slot) = slot {
                        let trigger = match button {
                            GilrsButton::LeftTrigger2 => Some((0, u16::MAX)),
                            GilrsButton::RightTrigger2 => Some((1, u16::MAX)),
                            _ => None,
                        };
                        if let Some((trigger, value)) = trigger {
                            let _ = self.input_sender.send(HostInputEvent::PadTrigger {
                                slot,
                                trigger,
                                value,
                            });
                        } else if let Some(button) = gamepad_button_index(button) {
                            let _ = self.input_sender.send(HostInputEvent::PadButton {
                                slot,
                                button,
                                down: true,
                            });
                        }
                    }
                }
                GilrsEventType::ButtonReleased(button, _) => {
                    if let Some(slot) = slot {
                        let trigger = match button {
                            GilrsButton::LeftTrigger2 => Some(0),
                            GilrsButton::RightTrigger2 => Some(1),
                            _ => None,
                        };
                        if let Some(trigger) = trigger {
                            let _ = self.input_sender.send(HostInputEvent::PadTrigger {
                                slot,
                                trigger,
                                value: 0,
                            });
                        } else if let Some(button) = gamepad_button_index(button) {
                            let _ = self.input_sender.send(HostInputEvent::PadButton {
                                slot,
                                button,
                                down: false,
                            });
                        }
                    }
                }
                GilrsEventType::ButtonChanged(button, value, _) => {
                    if let Some(slot) = slot {
                        let trigger = match button {
                            GilrsButton::LeftTrigger2 => Some(0),
                            GilrsButton::RightTrigger2 => Some(1),
                            _ => None,
                        };
                        if let Some(trigger) = trigger {
                            let _ = self.input_sender.send(HostInputEvent::PadTrigger {
                                slot,
                                trigger,
                                value: normalize_trigger(value),
                            });
                        } else if let Some(button) = gamepad_button_index(button) {
                            let _ = self.input_sender.send(HostInputEvent::PadButton {
                                slot,
                                button,
                                down: value >= 0.5,
                            });
                        }
                    }
                }
                GilrsEventType::AxisChanged(axis, value, _) => {
                    if let Some(slot) = slot {
                        let (axis, invert_y) = match axis {
                            GilrsAxis::LeftStickX => (Some(0), false),
                            GilrsAxis::LeftStickY => (Some(1), true),
                            GilrsAxis::RightStickX => (Some(2), false),
                            GilrsAxis::RightStickY => (Some(3), true),
                            _ => (None, false),
                        };
                        if let Some(axis) = axis {
                            let value = normalize_stick(if invert_y { -value } else { value });
                            let _ = self.input_sender.send(HostInputEvent::PadAxis {
                                slot,
                                axis,
                                value,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(gilrs) = self.gilrs.as_mut() {
            gilrs.inc();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PerformanceStats {
    running_cycles: u128,
    cycle_budget: usize,
    instructions: u128,
    cpu_utilization_percent: f64,
    runtime_percent: Option<f64>,
    kips: f64,
    ipc: f64,
}

impl PerformanceStats {
    fn from_deltas(
        running_cycles: u128,
        emulated_cycles: u128,
        instructions: u128,
        emulation_elapsed: Duration,
        cycle_budget: usize,
    ) -> Self {
        let cpu_utilization_percent = running_cycles as f64 * 100.0 / cycle_budget as f64;
        let runtime_percent = if emulation_elapsed.is_zero() || emulated_cycles == 0 {
            None
        } else {
            Some(
                emulation_elapsed.as_secs_f64() * f64::from(cpt32::cpu::CPU_CLOCK) * 100.0
                    / emulated_cycles as f64,
            )
        };
        let kips = if emulation_elapsed.is_zero() {
            0.0
        } else {
            instructions as f64 / emulation_elapsed.as_secs_f64() / 1_000.0
        };
        let ipc = if running_cycles == 0 {
            0.0
        } else {
            instructions as f64 / running_cycles as f64
        };

        Self {
            running_cycles,
            cycle_budget,
            instructions,
            cpu_utilization_percent,
            runtime_percent,
            kips,
            ipc,
        }
    }
}
const FRAME_PROFILE_STAGE_NAMES: [&str; 5] = [
    "VDP tick + VSync IRQ",
    "CPU emulation",
    "Video + ImGui submit",
    "Audio synthesis",
    "Audio output queue",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FrameProfile {
    stages: [Duration; FRAME_PROFILE_STAGE_NAMES.len()],
    cpu: CpuProfile,
}

impl FrameProfile {
    fn total(self) -> Duration {
        self.stages.into_iter().sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FrameProfileSummary {
    latest: FrameProfile,
    average: FrameProfile,
    samples: usize,
}

struct FrameProfiler {
    history: VecDeque<FrameProfile>,
}

impl FrameProfiler {
    const HISTORY_SAMPLES: usize = 120;

    fn new() -> Self {
        Self {
            history: VecDeque::with_capacity(Self::HISTORY_SAMPLES),
        }
    }

    fn record(&mut self, profile: FrameProfile) -> FrameProfileSummary {
        if self.history.len() == Self::HISTORY_SAMPLES {
            self.history.pop_front();
        }
        self.history.push_back(profile);

        let mut totals = [Duration::ZERO; FRAME_PROFILE_STAGE_NAMES.len()];
        let mut cpu_total = CpuProfile::default();
        for sample in &self.history {
            for (total, duration) in totals.iter_mut().zip(sample.stages) {
                *total += duration;
            }
            cpu_total.accumulate(sample.cpu);
        }
        let count = self.history.len() as u32;
        FrameProfileSummary {
            latest: profile,
            average: FrameProfile {
                stages: totals.map(|total| total / count),
                cpu: cpu_total.divided(count),
            },
            samples: self.history.len(),
        }
    }
}

fn realtime_multiplier(runtime_percent: f64) -> f64 {
    100.0 / runtime_percent
}

fn runtime_bar_state(runtime_percent: f64) -> (f32, bool) {
    (
        (runtime_percent / 100.0).clamp(0.0, 1.0) as f32,
        runtime_percent > 100.0,
    )
}

struct DebugUiState {
    running: bool,
    visible_windows: [bool; DebugWindow::COUNT],
    monitor_input: String,
    monitor_output: VecDeque<String>,
    monitor_scroll_to_bottom: bool,
    monitor_input_reclaim_focus: bool,
    cycles_per_frame_input: String,
    queued_steps: usize,
    disasm_follow_pc: bool,
    disasm_base_input: String,
    disasm_count_input: String,
    mem_base_input: String,
    mem_count_input: String,
    performance: Option<PerformanceStats>,
    frame_profile: Option<FrameProfileSummary>,
    cpu_utilization_history: VecDeque<f32>,
    runtime_percent_history: VecDeque<f32>,
}

#[derive(Clone, Copy)]
enum DebugWindow {
    Controls,
    Disassembly,
    Memory,
    BusComposition,
    Compositor,
    Performance,
    MonitorConsole,
}

impl DebugWindow {
    const ALL: [Self; Self::COUNT] = [
        Self::Controls,
        Self::Disassembly,
        Self::Memory,
        Self::BusComposition,
        Self::Compositor,
        Self::Performance,
        Self::MonitorConsole,
    ];
    const COUNT: usize = 7;

    fn label(self) -> &'static str {
        match self {
            Self::Controls => "Debug Controls",
            Self::Disassembly => "Realtime Disassembly",
            Self::Memory => "Realtime Memory Monitor",
            Self::BusComposition => "Bus Composition View",
            Self::Compositor => "Compositor Debug",
            Self::Performance => "Performance",
            Self::MonitorConsole => "src32 Debug Monitor Console",
        }
    }
}

impl DebugUiState {
    const PERFORMANCE_HISTORY_SAMPLES: usize = 120;
    const RUNTIME_HISTORY_MAX_PERCENT: f32 = 200.0;

    fn new(start_paused: bool) -> Self {
        Self {
            running: !start_paused,
            visible_windows: [false; DebugWindow::COUNT],
            monitor_input: String::new(),
            monitor_output: VecDeque::from([
                "Type help for monitor commands. Use pause/continue to control emulation.".into(),
            ]),
            monitor_scroll_to_bottom: true,
            monitor_input_reclaim_focus: false,
            cycles_per_frame_input: cpt32::cpu::CYCLES_PER_FRAME.to_string(),
            queued_steps: 0,
            disasm_follow_pc: true,
            disasm_base_input: "0x00000000".to_string(),
            disasm_count_input: "32".to_string(),
            mem_base_input: "0x00000000".to_string(),
            mem_count_input: "256".to_string(),
            performance: None,
            frame_profile: None,
            cpu_utilization_history: VecDeque::with_capacity(Self::PERFORMANCE_HISTORY_SAMPLES),
            runtime_percent_history: VecDeque::with_capacity(Self::PERFORMANCE_HISTORY_SAMPLES),
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

    fn cycles_per_frame(&self) -> usize {
        Self::parse_usize(
            &self.cycles_per_frame_input,
            cpt32::cpu::CYCLES_PER_FRAME as usize,
            10_000_000,
        )
    }

    fn execute_cpu(&mut self, cpu: &mut AnyCpu) {
        if self.running {
            cpu.run(self.cycles_per_frame());
        } else if self.queued_steps > 0 {
            for _ in 0..self.queued_steps {
                if !cpu.step_once() {
                    break;
                }
            }
            self.queued_steps = 0;
        }
    }

    fn reset_cpu(&mut self, cpu: &mut AnyCpu) {
        cpu.reset(0);
        self.running = false;
        self.queued_steps = 0;
    }

    fn draw_windows(&mut self, ui: &Ui, cpu: &mut AnyCpu, vdp: &Rc<RefCell<Vdp>>) {
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
        if self.visible_windows[DebugWindow::BusComposition as usize] {
            self.draw_bus_composition(ui, cpu);
        }
        if self.visible_windows[DebugWindow::Compositor as usize] {
            self.draw_compositor(ui, vdp);
        }
        if self.visible_windows[DebugWindow::Performance as usize] {
            self.draw_performance(ui);
        }
        if self.visible_windows[DebugWindow::MonitorConsole as usize] {
            self.draw_monitor_console(ui, cpu);
        }
    }

    fn draw_monitor_console(&mut self, ui: &Ui, cpu: &mut AnyCpu) {
        ui.window(DebugWindow::MonitorConsole.label())
            .size([720.0, 480.0], Condition::FirstUseEver)
            .build(|| {
                ui.child_window("Monitor output")
                    .size([0.0, -35.0])
                    .border(true)
                    .build(|| {
                        for line in &self.monitor_output {
                            ui.text(line);
                        }
                        if self.monitor_scroll_to_bottom {
                            ui.set_scroll_here_y_with_ratio(1.0);
                            self.monitor_scroll_to_bottom = false;
                        }
                    });
                if self.monitor_input_reclaim_focus {
                    ui.set_keyboard_focus_here();
                    self.monitor_input_reclaim_focus = false;
                }
                let submitted = ui
                    .input_text("src32>", &mut self.monitor_input)
                    .enter_returns_true(true)
                    .build();
                if submitted {
                    self.execute_monitor_command(cpu);
                    self.monitor_input_reclaim_focus = true;
                }
            });
    }

    fn execute_monitor_command(&mut self, cpu: &mut AnyCpu) {
        let command = self.monitor_input.trim().to_owned();
        self.monitor_input.clear();
        if command.is_empty() {
            return;
        }
        self.monitor_output.push_back(format!("src32> {command}"));
        match command.split_whitespace().next().unwrap_or_default() {
            "pause" => {
                self.running = false;
                self.monitor_output.push_back("Emulation paused.".into());
            }
            "continue" | "c" => {
                self.running = true;
                self.monitor_output.push_back("Emulation running.".into());
            }
            command_name => {
                if matches!(command_name, "step" | "s" | "run" | "r") {
                    self.running = false;
                }
                let result = monitor::execute_command(cpu, &command);
                self.monitor_output.extend(result.lines);
            }
        }
        while self.monitor_output.len() > 500 {
            self.monitor_output.pop_front();
        }
        self.monitor_scroll_to_bottom = true;
    }

    fn draw_bus_composition(&self, ui: &Ui, cpu: &AnyCpu) {
        let mut regions: Vec<_> = cpu.bus_memory_map().collect();
        regions.sort_unstable_by_key(|region| region.start);
        let largest_size = regions.iter().map(|region| region.size).max().unwrap_or(1);
        let table_flags = TableFlags::BORDERS | TableFlags::ROW_BG;

        ui.window("Bus Composition View")
            .size([780.0, 600.0], Condition::FirstUseEver)
            .build(|| {
                ui.text("Registered system-bus regions (32-bit address space)");
                ui.text("Mapped range sizes (bar width relative to the largest region)");
                if let Some(_table) =
                    ui.begin_table_with_flags("BusCompositionSizes", 2, table_flags)
                {
                    ui.table_setup_column("Region");
                    ui.table_setup_column("Relative size");
                    ui.table_headers_row();
                    for region in &regions {
                        ui.table_next_row();
                        ui.table_next_column();
                        ui.text(region.name);
                        ui.table_next_column();
                        ProgressBar::new(region.size as f32 / largest_size as f32)
                            .size([220.0, 14.0])
                            .build(ui);
                    }
                }

                ui.separator();
                ui.text("Registered device ranges (unlisted addresses are unmapped)");
                if let Some(_table) = ui.begin_table_with_flags("BusCompositionMap", 4, table_flags)
                {
                    ui.table_setup_column("Region");
                    ui.table_setup_column("Start");
                    ui.table_setup_column("End (inclusive)");
                    ui.table_setup_column("Size");
                    ui.table_headers_row();
                    for region in &regions {
                        ui.table_next_row();
                        ui.table_next_column();
                        ui.text(region.name);
                        ui.table_next_column();
                        ui.text(format!("0x{:08X}", region.start));
                        ui.table_next_column();
                        ui.text(format!("0x{:08X}", region.start + region.size - 1));
                        ui.table_next_column();
                        ui.text(Self::format_region_size(region.size));
                    }
                }
            });
    }

    fn format_region_size(size: u32) -> String {
        if size % (1024 * 1024) == 0 {
            format!("{} MiB", size / (1024 * 1024))
        } else if size % 1024 == 0 {
            format!("{} KiB", size / 1024)
        } else {
            format!("{} B", size)
        }
    }

    fn draw_performance(&self, ui: &Ui) {
        ui.window("Performance")
            .size([560.0, 660.0], Condition::FirstUseEver)
            .build(|| {
                if let Some(stats) = self.performance {
                    ui.text(format!(
                        "CPU utilization: {:.1}% ({} / {} running cycles per VSync)",
                        stats.cpu_utilization_percent, stats.running_cycles, stats.cycle_budget
                    ));
                    ProgressBar::new(
                        (stats.cpu_utilization_percent / 100.0).clamp(0.0, 1.0) as f32,
                    )
                    .size([450.0, 18.0])
                    .build(ui);
                    ui.same_line();
                    ui.text(format!("{:.1}%", stats.cpu_utilization_percent));
                    if let Some(runtime_percent) = stats.runtime_percent {
                        ui.text(format!(
                            "Runtime: {:.1}% of realtime ({:.3}x emulation; bar max 100%)",
                            runtime_percent,
                            realtime_multiplier(runtime_percent)
                        ));
                        let (bar_fraction, over_realtime) = runtime_bar_state(runtime_percent);
                        let runtime_bar = ProgressBar::new(bar_fraction).size([450.0, 18.0]);
                        if over_realtime {
                            let red = ui.push_style_color(
                                StyleColor::PlotHistogram,
                                [1.0, 0.15, 0.15, 1.0],
                            );
                            runtime_bar.build(ui);
                            red.pop();
                        } else {
                            runtime_bar.build(ui);
                        }
                    } else {
                        ui.text("Runtime: N/A (no emulated cycles in this sample)");
                        ProgressBar::new(0.0).size([450.0, 18.0]).build(ui);
                    }
                    ui.text(format!("Instruction throughput: {:.1} KIPS", stats.kips));
                    ui.text(format!("IPC: {:.3} instructions/cycle", stats.ipc));
                    ui.text(format!("Instructions this VSync: {}", stats.instructions));
                } else {
                    ui.text("Waiting for the first VSync sample...");
                }

                if let Some(profile) = self.frame_profile {
                    Self::draw_cpu_profile(ui, profile);
                }
                Self::draw_performance_history(
                    ui,
                    "CPU utilization history (120 VSync samples; 0-100% cycle budget)",
                    &self.cpu_utilization_history,
                    100.0,
                );
                Self::draw_performance_history(
                    ui,
                    "Runtime ratio history (120 active samples; 0-200%, midline=100%)",
                    &self.runtime_percent_history,
                    Self::RUNTIME_HISTORY_MAX_PERCENT,
                );
                if let Some(profile) = self.frame_profile {
                    Self::draw_frame_profile(ui, profile);
                }
            });
    }

    fn draw_cpu_profile(ui: &Ui, profile: FrameProfileSummary) {
        let cpu_total = profile.average.cpu.total();
        ui.separator();
        ui.text("CPU internal stages (latest and average; share of measured CPU stage time)");
        ui.text(format!(
            "CPU ticks: last {} | average {} | average measured {:.3} ms",
            profile.latest.cpu.ticks,
            profile.average.cpu.ticks,
            cpu_total.as_secs_f64() * 1_000.0
        ));
        for (index, name) in CPU_PROFILE_STAGE_NAMES.iter().enumerate() {
            let latest_ms = profile.latest.cpu.stages[index].as_secs_f64() * 1_000.0;
            let average_ms = profile.average.cpu.stages[index].as_secs_f64() * 1_000.0;
            let share = if cpu_total.is_zero() {
                0.0
            } else {
                profile.average.cpu.stages[index].as_secs_f64() * 100.0 / cpu_total.as_secs_f64()
            };
            ui.text(format!(
                "{name:<24} last {latest_ms:>7.3} ms | avg {average_ms:>7.3} ms ({share:>5.1}%)"
            ));
        }
        ui.text("Cached interpreter combines instruction fetch, cache lookup, and decode.");
        ui.text("JIT mode uses Cached execution while CPU profiling is enabled.");
    }

    fn draw_frame_profile(ui: &Ui, profile: FrameProfileSummary) {
        ui.separator();
        ui.text("Host frame work by stage (latest and 120-sample average)");
        ui.text("Video timing is CPU-side submission; GPU execution and VSync wait are excluded.");

        let average_total = profile.average.total();
        ui.text(format!(
            "Frame work: last {:.3} ms | average {:.3} ms ({} samples)",
            profile.latest.total().as_secs_f64() * 1_000.0,
            average_total.as_secs_f64() * 1_000.0,
            profile.samples
        ));
        for (index, name) in FRAME_PROFILE_STAGE_NAMES.iter().enumerate() {
            let latest_ms = profile.latest.stages[index].as_secs_f64() * 1_000.0;
            let average_ms = profile.average.stages[index].as_secs_f64() * 1_000.0;
            let share = if average_total.is_zero() {
                0.0
            } else {
                profile.average.stages[index].as_secs_f64() * 100.0 / average_total.as_secs_f64()
            };
            ui.text(format!(
                "{name:<24} last {latest_ms:>7.3} ms | avg {average_ms:>7.3} ms ({share:>5.1}%)"
            ));
        }
    }

    fn draw_performance_history(ui: &Ui, label: &str, history: &VecDeque<f32>, max_value: f32) {
        ui.separator();
        ui.text(label);
        let size = [500.0, 100.0];
        let origin = ui.cursor_screen_pos();
        ui.invisible_button(label, size);
        let draw = ui.get_window_draw_list();
        let bottom = origin[1] + size[1];
        draw.add_rect(
            origin,
            [origin[0] + size[0], bottom],
            [0.08, 0.11, 0.14, 1.0],
        )
        .filled(true)
        .build();
        for fraction in [0.0, 0.5, 1.0] {
            let y = origin[1] + fraction * size[1];
            draw.add_line(
                [origin[0], y],
                [origin[0] + size[0], y],
                [0.25, 0.29, 0.33, 1.0],
            )
            .build();
        }
        if history.len() > 1 {
            let last_index = history.len() - 1;
            let mut previous = None;
            for (index, value) in history.iter().enumerate() {
                let x = origin[0] + index as f32 / last_index as f32 * size[0];
                let y = bottom - (*value / max_value).clamp(0.0, 1.0) * size[1];
                if let Some(point) = previous {
                    draw.add_line(point, [x, y], [0.35, 0.78, 1.0, 1.0])
                        .thickness(2.0)
                        .build();
                }
                previous = Some([x, y]);
            }
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

    fn draw_controls(&mut self, ui: &Ui, cpu: &mut AnyCpu) {
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
                ui.same_line();
                if ui.button("Reset") {
                    self.reset_cpu(cpu);
                }
                ui.separator();
                if self.running {
                    ui.text("Emulation: RUNNING (realtime)");
                } else {
                    ui.text("Emulation: PAUSED - CPU not ticked; keyboard input is not processed");
                }
                ui.text(format!("CPU running: {}", cpu.is_running()));
                ui.text(format!("PC: 0x{:08X}", cpu.pc()));
                ui.text(format!("Mode: {:?}", cpu.instruction_mode()));
                ui.text(format!("Total cycles: {}", cpu.cycles()));
                ui.text(format!(
                    "Total instructions executed: {}",
                    cpu.instructions_executed()
                ));

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

    fn draw_disassembly(&mut self, ui: &Ui, cpu: &mut AnyCpu) {
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

    fn draw_memory(&mut self, ui: &Ui, cpu: &mut AnyCpu) {
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
    last_performance_sample: Option<(u128, u128, u128)>,
    menu_bar_height: f32,
}

impl DebugGui {
    fn new(
        window: &Window,
        presenter: &render::WgpuPresenter,
        start_paused: bool,
        profile_cpu: bool,
    ) -> Self {
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

        let mut state = DebugUiState::new(start_paused);
        if profile_cpu {
            state.visible_windows[DebugWindow::Performance as usize] = true;
        }
        Self {
            imgui,
            platform,
            renderer,
            state,
            last_frame: Instant::now(),
            last_performance_sample: None,
            menu_bar_height: 0.0,
        }
    }

    fn last_menu_bar_height(&self) -> f32 {
        self.menu_bar_height
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
        cpu: &mut AnyCpu,
        vdp: &Rc<RefCell<Vdp>>,
        presenter: &mut render::WgpuPresenter,
    ) -> (Duration, Duration, Result<(), render::FrameError>) {
        let emulation_start = Instant::now();
        self.state.execute_cpu(cpu);
        let emulation_elapsed = emulation_start.elapsed();
        let emulated_cycles = cpu.cycles();
        let running_cycles = cpu.running_cycles();
        let instructions = cpu.instructions_executed();
        if let Some((last_running_cycles, last_instructions, last_emulated_cycles)) =
            self.last_performance_sample
        {
            let stats = PerformanceStats::from_deltas(
                running_cycles.saturating_sub(last_running_cycles),
                emulated_cycles.saturating_sub(last_emulated_cycles),
                instructions.saturating_sub(last_instructions),
                emulation_elapsed,
                self.state.cycles_per_frame(),
            );
            self.state.performance = Some(stats);
            if self.state.cpu_utilization_history.len() == DebugUiState::PERFORMANCE_HISTORY_SAMPLES
            {
                self.state.cpu_utilization_history.pop_front();
            }
            self.state
                .cpu_utilization_history
                .push_back(stats.cpu_utilization_percent as f32);
            if let Some(runtime_percent) = stats.runtime_percent {
                if self.state.runtime_percent_history.len()
                    == DebugUiState::PERFORMANCE_HISTORY_SAMPLES
                {
                    self.state.runtime_percent_history.pop_front();
                }
                self.state
                    .runtime_percent_history
                    .push_back(runtime_percent as f32);
            }
        }
        self.last_performance_sample = Some((running_cycles, instructions, emulated_cycles));

        let rendering_start = Instant::now();
        let render_result = (|| -> Result<(), render::FrameError> {
            let now = Instant::now();
            self.imgui.io_mut().update_delta_time(now - self.last_frame);
            self.last_frame = now;
            self.platform
                .prepare_frame(self.imgui.io_mut(), window)
                .map_err(|err| {
                    render::FrameError::Overlay(format!("ImGui frame prepare failed: {err}"))
                })?;

            let ui = self.imgui.frame();
            self.menu_bar_height = ui.frame_height();
            self.state.draw_windows(ui, cpu, vdp);
            self.platform.prepare_render(ui, window);
            let framebuffer_scale = f64::from(self.imgui.io().display_framebuffer_scale[1]);
            let top_reserve_px = (f64::from(self.menu_bar_height) * framebuffer_scale).max(0.0);
            let draw_data = self.imgui.render();

            presenter.render_with_overlay(
                vdp,
                top_reserve_px,
                |device, queue, encoder, surface_view| {
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
                        .map_err(|err| {
                            render::FrameError::Overlay(format!("ImGui render failed: {err}"))
                        })
                },
            )
        })();
        let rendering_elapsed = rendering_start.elapsed();
        (emulation_elapsed, rendering_elapsed, render_result)
    }
}

impl ApplicationHandler for GuiApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes: WindowAttributes = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(WIDTH as f64, HEIGHT as f64))
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
            Some(DebugGui::new(
                &window,
                &presenter,
                self.start_paused,
                self.frame_profiler.is_some(),
            ))
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
        if let WindowEvent::DroppedFile(path) = &event {
            self.handle_dropped_file(path);
            return;
        }
        if let (Some(window), Some(debug_gui)) = (self.window.as_ref(), self.debug_gui.as_mut()) {
            debug_gui.handle_window_event(window, window_id, &event);
        }
        self.honor_imgui_mouse_capture();

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::DroppedFile(_) => unreachable!("handled above"),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if !is_synthetic && matches!(&event.logical_key, Key::Named(NamedKey::Escape)) {
                    event_loop.exit();
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    if is_synthetic {
                        // Focus key snapshot, not a physical transition; see
                        // `guest_key_input`.
                        if pec_trace_enabled() && event.state == ElementState::Pressed {
                            tracing::info!(
                                target: "pec_debug",
                                "[PEC-DBG] GUI key focus snapshot ignored physical={code:?}"
                            );
                        }
                    } else if code == KeyCode::F1 {
                        if event.state == ElementState::Pressed && !event.repeat {
                            let imgui_captures = self
                                .debug_gui
                                .as_ref()
                                .is_some_and(|gui| gui.imgui.io().want_capture_mouse);
                            if !imgui_captures {
                                if let Some(window) = self.window.as_ref() {
                                    let enable = !self.mouse_captured;
                                    set_mouse_capture(&mut self.mouse_captured, window, enable);
                                }
                            }
                        }
                    } else if let Some((usage, down)) =
                        guest_key_input(&event.physical_key, event.state, is_synthetic)
                    {
                        if pec_trace_enabled() {
                            tracing::info!(
                                target: "pec_debug",
                                "[PEC-DBG] GUI key physical={code:?} usage=0x{usage:02X} down={down} repeat={} total_cycles={} running_cycles={} instructions={} cpu_running={} cpu_irq_enabled={} pc=0x{:08X} epc=0x{:08X} irq_cause={} debug_gui_running={:?}",
                                event.repeat,
                                self.cpu.cycles(),
                                self.cpu.running_cycles(),
                                self.cpu.instructions_executed(),
                                self.cpu.is_running(),
                                self.cpu.irq_enabled(),
                                self.cpu.pc(),
                                self.cpu.epc(),
                                self.cpu.irq_cause(),
                                self.debug_gui.as_ref().map(|gui| gui.state.running)
                            );
                        }
                        if down {
                            self.held_keys.insert(usage);
                        } else {
                            self.held_keys.remove(&usage);
                        }
                        let send_result =
                            self.input_sender.send(HostInputEvent::Key { usage, down });
                        if pec_trace_enabled() {
                            tracing::info!(
                                target: "pec_debug",
                                "[PEC-DBG] GUI send key usage=0x{usage:02X} down={down} queued={}",
                                send_result.is_ok()
                            );
                        }
                    } else if pec_trace_enabled() && event.state == ElementState::Pressed {
                        tracing::info!(
                            target: "pec_debug",
                            "[PEC-DBG] GUI key unmapped physical={code:?}"
                        );
                    }
                }
            }
            WindowEvent::Focused(focused) => {
                if pec_trace_enabled() {
                    tracing::info!(
                        target: "pec_debug",
                        "[PEC-DBG] GUI focus focused={focused}"
                    );
                }
                self.focused = focused;
                if !focused {
                    if let Some(window) = self.window.as_ref() {
                        set_mouse_capture(&mut self.mouse_captured, window, false);
                    }
                    self.release_all_inputs();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let imgui_captures = self
                    .debug_gui
                    .as_ref()
                    .is_some_and(|gui| gui.imgui.io().want_capture_mouse);
                if self.focused && !imgui_captures {
                    let button = match button {
                        MouseButton::Left => Some(0),
                        MouseButton::Right => Some(1),
                        MouseButton::Middle => Some(2),
                        MouseButton::Back => Some(3),
                        MouseButton::Forward => Some(4),
                        _ => None,
                    };
                    if let Some(button) = button {
                        let down = state == ElementState::Pressed;
                        let mask = 1 << button;
                        if down {
                            self.held_mouse_buttons |= mask;
                        } else {
                            self.held_mouse_buttons &= !mask;
                        }
                        let _ = self
                            .input_sender
                            .send(HostInputEvent::MouseButton { button, down });
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let imgui_captures = self
                    .debug_gui
                    .as_ref()
                    .is_some_and(|gui| gui.imgui.io().want_capture_mouse);
                if self.focused && !self.mouse_captured && !imgui_captures {
                    if let Some(window) = self.window.as_ref() {
                        let top_reserve_px = debug_menu_reserve_px(self.debug_gui.as_ref());
                        let (x, y) =
                            mouse_position_to_guest(position, window.inner_size(), top_reserve_px);
                        let _ = self
                            .input_sender
                            .send(HostInputEvent::MousePosition { x, y });
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let imgui_captures = self
                    .debug_gui
                    .as_ref()
                    .is_some_and(|gui| gui.imgui.io().want_capture_mouse);
                if self.focused && !imgui_captures {
                    let (vertical, horizontal) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => (
                            normalize_wheel(f64::from(y), false),
                            normalize_wheel(f64::from(x), false),
                        ),
                        MouseScrollDelta::PixelDelta(position) => (
                            normalize_wheel(position.y, true),
                            normalize_wheel(position.x, true),
                        ),
                    };
                    let _ = self.input_sender.send(HostInputEvent::MouseWheel {
                        vertical,
                        horizontal,
                    });
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
                let mut profile = self
                    .frame_profiler
                    .as_ref()
                    .map(|_| FrameProfile::default());
                let device_start = profile.as_ref().map(|_| Instant::now());
                self.vdp.borrow_mut().tick();
                self.cpu.set_irq_source(0, true);
                self.cpu.set_irq_source(0, false);
                if let (Some(profile), Some(start)) = (profile.as_mut(), device_start) {
                    profile.stages[0] = start.elapsed();
                }
                if let (Some(window), Some(presenter)) =
                    (self.window.as_ref(), self.presenter.as_mut())
                {
                    let render_result = if let Some(debug_gui) = self.debug_gui.as_mut() {
                        let (cpu_elapsed, render_elapsed, result) =
                            debug_gui.render_frame(window, &mut self.cpu, &self.vdp, presenter);
                        if let Some(profile) = profile.as_mut() {
                            profile.stages[1] = cpu_elapsed;
                            profile.stages[2] = render_elapsed;
                        }
                        result
                    } else {
                        let cpu_start = profile.as_ref().map(|_| Instant::now());
                        self.cpu.run(cpt32::cpu::CYCLES_PER_FRAME as usize);
                        if let (Some(profile), Some(start)) = (profile.as_mut(), cpu_start) {
                            profile.stages[1] = start.elapsed();
                        }
                        let render_start = profile.as_ref().map(|_| Instant::now());
                        let result = presenter.render(&self.vdp);
                        if let (Some(profile), Some(start)) = (profile.as_mut(), render_start) {
                            profile.stages[2] = start.elapsed();
                        }
                        result
                    };

                    if let Some(audio_host) = self.audio_host.as_ref() {
                        let sample_rate = audio_host.sample_rate();
                        let sample_count = (sample_rate / cpt32::sys::FRAME_RATE) as usize;
                        let audio_start = profile.as_ref().map(|_| Instant::now());
                        self.audio_left.resize(sample_count, 0);
                        self.audio_right.resize(sample_count, 0);
                        self.audio_left.fill(0);
                        self.audio_right.fill(0);
                        for sgu in &self.sgu {
                            sgu.borrow_mut()
                                .clock_mixed_into(&mut self.audio_left, &mut self.audio_right);
                        }
                        if let Some(idc) = self.idc.as_ref() {
                            idc.borrow_mut().render_audio(
                                sample_rate,
                                &mut self.audio_left,
                                &mut self.audio_right,
                            );
                        }
                        if let (Some(profile), Some(start)) = (profile.as_mut(), audio_start) {
                            profile.stages[3] = start.elapsed();
                        }
                        let queue_start = profile.as_ref().map(|_| Instant::now());
                        audio_host.push_samples_i16(&self.audio_left, &self.audio_right);
                        if let (Some(profile), Some(start)) = (profile.as_mut(), queue_start) {
                            profile.stages[4] = start.elapsed();
                        }
                    }

                    match render_result {
                        Ok(()) => {}
                        Err(render::FrameError::Lost) | Err(render::FrameError::Outdated) => {
                            presenter.resize(window.inner_size())
                        }
                        Err(render::FrameError::Timeout) | Err(render::FrameError::Occluded) => {}
                        Err(render::FrameError::Validation) => {
                            tracing::error!("Render validation error");
                        }
                        Err(render::FrameError::Overlay(message)) => {
                            tracing::error!(message = %message, "Render overlay error");
                        }
                    }
                    if let (Some(profile), Some(profiler), Some(debug_gui)) = (
                        profile,
                        self.frame_profiler.as_mut(),
                        self.debug_gui.as_mut(),
                    ) {
                        let mut profile = profile;
                        profile.cpu = self.cpu.take_profile().unwrap_or_default();
                        debug_gui.state.frame_profile = Some(profiler.record(profile));
                    }
                }
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        self.honor_imgui_mouse_capture();
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.mouse_captured && self.focused {
                if let Some(window) = self.window.as_ref() {
                    let top_reserve_px = debug_menu_reserve_px(self.debug_gui.as_ref());
                    let (scale, _, _) = render::game_viewport(window.inner_size(), top_reserve_px);
                    if scale > 0.0 {
                        let dx = (delta.0 / scale)
                            .round()
                            .clamp(f64::from(i16::MIN), f64::from(i16::MAX))
                            as i16;
                        let dy = (delta.1 / scale)
                            .round()
                            .clamp(f64::from(i16::MIN), f64::from(i16::MAX))
                            as i16;
                        let _ = self
                            .input_sender
                            .send(HostInputEvent::MouseRelative { dx, dy });
                    }
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_gamepads();
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
    enable_debug_gui: bool,
    profile_cpu: bool,
    start_paused: bool,
    music_stream: Option<PathBuf>,
    cpu: Option<CpuKind>,
}

fn pcg_font_path_from_config() -> Result<Option<PathBuf>, String> {
    let config_path = Path::new("config.yaml");
    let contents = match std::fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", config_path.display())),
    };
    let root: yaml_serde::Value = yaml_serde::from_str(&contents)
        .map_err(|error| format!("invalid YAML in {}: {error}", config_path.display()))?;
    let Some(pcg) = root.get("pcg") else {
        return Ok(None);
    };
    let table = pcg.as_mapping().ok_or("`pcg` must be a mapping")?;
    let Some(font) = table.get("font") else {
        return Ok(None);
    };
    let font = font.as_str().ok_or("`pcg.font` must be a path string")?;
    let path = Path::new(font);
    Ok(Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    }))
}

fn parse_gui_args(args: &[String]) -> Result<GuiLaunchOptions, String> {
    let mut positional = Vec::new();
    let mut enable_debug_gui = false;
    let mut profile_cpu = false;
    let mut start_paused = false;

    let mut music_stream = None;
    let mut cpu: Option<CpuKind> = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--debug-gui" | "--imgui-debug" => enable_debug_gui = true,
            "--profile-cpu" => {
                profile_cpu = true;
                enable_debug_gui = true;
            }
            "--start-paused" | "--pause-on-start" => start_paused = true,
            "--music-stream" => {
                if music_stream.is_some() {
                    return Err("Duplicate --music-stream".into());
                }
                music_stream = Some(PathBuf::from(
                    args.next().ok_or("--music-stream requires a path")?,
                ));
            }
            "--cpu" => {
                if cpu.is_some() {
                    return Err("Duplicate --cpu".into());
                }
                cpu = Some(
                    args.next()
                        .ok_or("--cpu requires 'scalar', 'cached', 'jit' (feature required), or 'pipeline'")?
                        .parse::<CpuKind>()?,
                );
            }
            _ if arg.starts_with("--cpu=") => {
                if cpu.is_some() {
                    return Err("Duplicate --cpu".into());
                }
                cpu = Some(arg["--cpu=".len()..].parse::<CpuKind>()?);
            }
            _ if arg.starts_with('-') => return Err(format!("Unknown option: {arg}")),
            _ => positional.push(arg.clone()),
        }
    }

    if positional.is_empty() {
        return Err("Missing program path".to_string());
    }
    if positional.len() > 1 {
        return Err("Too many positional arguments".to_string());
    }

    Ok(GuiLaunchOptions {
        program_path: positional[0].clone(),
        enable_debug_gui,
        profile_cpu,
        start_paused,
        music_stream,
        cpu,
    })
}

fn main() {
    cpt32::init_logging();

    let args: Vec<String> = env::args().skip(1).collect();
    let first = args.first().map(String::as_str);

    match first {
        Some("monitor") | Some("-m") | Some("--monitor") => match split_cpu_arg(&args[1..]) {
            Ok((rest, cpu)) => {
                monitor::run(rest.first().map(String::as_str), cpu);
            }
            Err(message) => {
                tracing::error!(%message, "Invalid command line");
                std::process::exit(2);
            }
        },
        Some(_) => match parse_gui_args(&args) {
            Ok(options) => {
                if options.start_paused && !options.enable_debug_gui {
                    tracing::warn!("--start-paused requires --debug-gui and will be ignored");
                }
                if let Err(message) = run_gui_with_options(&options) {
                    tracing::error!(%message, "Failed to start emulator");
                    std::process::exit(2);
                }
            }
            Err(message) => {
                tracing::error!(%message, "Invalid command line");
                eprintln!(
                    "Usage: cargo run -- <program.bin> [--debug-gui] [--profile-cpu] [--start-paused] [--music-stream song.sgub] [--cpu scalar|cached|jit|pipeline] | cargo run -- [-m|--monitor] [program.bin] [--cpu scalar|cached|jit|pipeline]; editor: cargo run --bin music_editor -- [project.yaml]"
                );
            }
        },
        None => {
            eprintln!(
                "Usage: cargo run -- <program.bin> [--debug-gui] [--profile-cpu] [--start-paused] [--music-stream song.sgub] [--cpu scalar|cached|jit|pipeline] | cargo run -- [-m|--monitor] [program.bin] [--cpu scalar|cached|jit|pipeline]; editor: cargo run --bin music_editor -- [project.yaml]"
            );
        }
    }
}

fn run_gui_with_options(options: &GuiLaunchOptions) -> Result<(), String> {
    let event_loop =
        EventLoop::new().map_err(|error| format!("Failed to create event loop: {error}"))?;
    let cpu_kind = resolve_cpu_kind(options.cpu, Path::new("config.yaml"))?;
    tracing::info!(cpu = cpu_kind.as_str(), "CPU micro-architecture selected");
    let display_handle = event_loop.owned_display_handle();
    let mut app = GuiApp::new(
        &options.program_path,
        display_handle,
        options.enable_debug_gui,
        options.profile_cpu,
        options.enable_debug_gui && options.start_paused,
        options.music_stream.as_deref(),
        cpu_kind,
    )?;
    event_loop
        .run_app(&mut app)
        .map_err(|error| format!("Failed to run application: {error}"))
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn debug_reset_restarts_cpu_paused_without_clearing_ram() {
        let mut bus = Bus::new();
        connect_ram(&mut bus);
        bus.write_u8(0x100, 0xA5);
        let mut cpu = AnyCpu::new(CpuKind::Scalar, bus);
        cpu.set_pc(0x40);
        cpu.run(12);
        assert!(cpu.cycles() > 0);
        assert!(cpu.instructions_executed() > 0);
        let mut state = DebugUiState::new(false);
        state.queued_steps = 7;

        state.reset_cpu(&mut cpu);

        assert_eq!(cpu.pc(), 0);
        assert_eq!(cpu.cycles(), 0);
        assert_eq!(cpu.running_cycles(), 0);
        assert_eq!(cpu.instructions_executed(), 0);
        assert_eq!(cpu.read_mem_u8(0x100), 0xA5);
        assert!(!state.running);
        assert_eq!(state.queued_steps, 0);
    }

    #[test]
    fn maps_physical_keyboard_usage_ids_without_discriminant_casts() {
        assert_eq!(keyboard_hid_usage(KeyCode::KeyA), Some(0x04));
        assert_eq!(keyboard_hid_usage(KeyCode::Digit0), Some(0x27));
        assert_eq!(keyboard_hid_usage(KeyCode::BracketLeft), Some(0x2F));
        assert_eq!(keyboard_hid_usage(KeyCode::Escape), Some(0x29));
        assert_eq!(keyboard_hid_usage(KeyCode::F24), Some(0x73));
        assert_eq!(keyboard_hid_usage(KeyCode::F12), Some(0x45));
        assert_eq!(keyboard_hid_usage(KeyCode::NumpadComma), Some(0x85));
        assert_eq!(keyboard_hid_usage(KeyCode::ControlRight), Some(0xE4));
        assert_eq!(keyboard_hid_usage(KeyCode::F25), None);
    }

    #[test]
    fn maps_mouse_coordinates_through_centered_viewport_and_clamps() {
        let size = PhysicalSize::new(render::PRESENT_WIDTH * 2, render::PRESENT_HEIGHT * 2);
        assert_eq!(
            mouse_position_to_guest(PhysicalPosition::new(16.0, 16.0), size, 0.0),
            (0, 0)
        );
        assert_eq!(
            mouse_position_to_guest(PhysicalPosition::new(1294.0, 974.0), size, 0.0),
            (639, 479)
        );
        assert_eq!(
            mouse_position_to_guest(PhysicalPosition::new(-100.0, 4000.0), size, 0.0),
            (0, 479)
        );

        let wide = PhysicalSize::new(2000, 600);
        let (scale, _, viewport_y) = render::game_viewport(wide, 0.0);
        let content_top = viewport_y + f64::from(render::BORDER_SIZE) * scale;
        assert_eq!(
            mouse_position_to_guest(PhysicalPosition::new(1000.0, content_top), wide, 0.0).1,
            0
        );
    }

    #[test]
    fn reserves_debug_menu_bar_above_centered_viewport() {
        let size = PhysicalSize::new(800, 600);
        let reserve = 24.0;
        let (scale, viewport_x, viewport_y) = render::game_viewport(size, reserve);
        assert!(scale > 0.0);
        assert!(viewport_y >= reserve);
        let content_top = viewport_y + f64::from(render::BORDER_SIZE) * scale;
        assert_eq!(
            mouse_position_to_guest(
                PhysicalPosition::new(viewport_x, content_top),
                size,
                reserve
            )
            .1,
            0
        );
        assert_eq!(
            mouse_position_to_guest(PhysicalPosition::new(viewport_x, 0.0), size, reserve).1,
            0
        );
    }

    #[test]
    fn normalizes_mouse_and_gamepad_ranges() {
        assert_eq!(normalize_wheel(1.0, false), 120);
        assert_eq!(normalize_wheel(100.0, true), 120);
        assert_eq!(normalize_wheel(100_000.0, true), i16::MAX);
        assert_eq!(normalize_stick(-1.0), i16::MIN);
        assert_eq!(normalize_stick(1.0), i16::MAX);
        assert_eq!(normalize_trigger(0.0), 0);
        assert_eq!(normalize_trigger(1.0), u16::MAX);
    }

    #[test]
    fn cpu_profile_option_enables_debug_gui_but_default_does_not_profile() {
        let normal = parse_gui_args(&["guest.wax".into()]).unwrap();
        assert!(!normal.profile_cpu);
        assert!(!normal.enable_debug_gui);

        let debug = parse_gui_args(&["guest.wax".into(), "--debug-gui".into()]).unwrap();
        assert!(debug.enable_debug_gui);
        assert!(!debug.profile_cpu);

        let profile = parse_gui_args(&["guest.wax".into(), "--profile-cpu".into()]).unwrap();
        assert!(profile.profile_cpu);
        assert!(profile.enable_debug_gui);
    }

    #[test]
    fn focus_key_snapshot_is_not_guest_input() {
        // winit reports the key state of a window focus change as a synthetic
        // event. On an IME-enabled desktop the OS reports the IME's own virtual
        // keys as pressed and Windows maps them onto Backquote / Caps Lock, so
        // focusing the window pushed a phantom backquote (usage 0x35) into the
        // PeC FIFO and every guest echoed it.
        let backquote = PhysicalKey::Code(KeyCode::Backquote);
        assert_eq!(
            guest_key_input(&backquote, ElementState::Pressed, true),
            None
        );
        assert_eq!(
            guest_key_input(&backquote, ElementState::Released, true),
            None
        );
        let caps_lock = PhysicalKey::Code(KeyCode::CapsLock);
        assert_eq!(
            guest_key_input(&caps_lock, ElementState::Pressed, true),
            None
        );

        assert_eq!(
            guest_key_input(&backquote, ElementState::Pressed, false),
            Some((0x35, true))
        );
        assert_eq!(
            guest_key_input(&backquote, ElementState::Released, false),
            Some((0x35, false))
        );
        assert_eq!(
            guest_key_input(&caps_lock, ElementState::Pressed, false),
            Some((0x39, true))
        );
        assert_eq!(
            guest_key_input(
                &PhysicalKey::Code(KeyCode::F25),
                ElementState::Pressed,
                false
            ),
            None
        );
    }

    #[test]
    fn releasing_focus_lost_inputs_enqueues_key_and_button_ups() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut keys = HashSet::from([0x04, 0xE1]);
        let mut mouse_buttons = (1 << 0) | (1 << 2) | (1 << 4);
        enqueue_held_input_releases(&sender, &mut keys, &mut mouse_buttons);

        assert!(keys.is_empty());
        assert_eq!(mouse_buttons, 0);
        let mut released_keys = HashSet::new();
        let mut released_buttons = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            match event {
                HostInputEvent::Key { usage, down: false } => {
                    released_keys.insert(usage);
                }
                HostInputEvent::MouseButton {
                    button,
                    down: false,
                } => released_buttons.push(button),
                other => panic!("unexpected focus-loss event: {other:?}"),
            }
        }
        released_buttons.sort_unstable();
        assert_eq!(released_keys, HashSet::from([0x04, 0xE1]));
        assert_eq!(released_buttons, vec![0, 2, 4]);
    }

    #[test]
    fn calculates_utilization_and_runtime_ratio_from_distinct_cycle_counts() {
        let stats = PerformanceStats::from_deltas(
            100_000,
            24_000_000,
            5_000,
            Duration::from_millis(500),
            200_000,
        );

        assert_eq!(stats.running_cycles, 100_000);
        assert_eq!(stats.cycle_budget, 200_000);
        assert_eq!(stats.instructions, 5_000);
        assert_eq!(stats.cpu_utilization_percent, 50.0);
        assert_eq!(stats.runtime_percent, Some(100.0));
        assert_eq!(realtime_multiplier(100.0), 1.0);
        assert_eq!(stats.kips, 10.0);
        assert_eq!(stats.ipc, 0.05);
    }

    #[test]
    fn runtime_ratio_and_multiplier_cover_both_sides_of_realtime() {
        let slower = PerformanceStats::from_deltas(
            200_000,
            12_000_000,
            5_000,
            Duration::from_millis(500),
            200_000,
        );
        assert_eq!(slower.runtime_percent, Some(200.0));
        assert_eq!(realtime_multiplier(200.0), 0.5);

        let faster = PerformanceStats::from_deltas(
            200_000,
            36_000_000,
            5_000,
            Duration::from_millis(500),
            200_000,
        );
        let runtime_percent = faster.runtime_percent.unwrap();
        assert!((realtime_multiplier(runtime_percent) - 1.5).abs() < 1e-12);
    }

    #[test]
    fn runtime_bar_caps_at_100_and_turns_red_above_limit() {
        assert_eq!(runtime_bar_state(75.0), (0.75, false));
        assert_eq!(runtime_bar_state(100.0), (1.0, false));
        assert_eq!(runtime_bar_state(150.0), (1.0, true));
    }

    #[test]
    fn zero_elapsed_or_paused_sample_has_no_runtime_ratio() {
        let zero_elapsed = PerformanceStats::from_deltas(0, 0, 100, Duration::ZERO, 200_000);
        assert_eq!(zero_elapsed.kips, 0.0);
        assert_eq!(zero_elapsed.ipc, 0.0);
        assert_eq!(zero_elapsed.runtime_percent, None);

        let paused = PerformanceStats::from_deltas(0, 0, 0, Duration::from_millis(500), 200_000);
        assert_eq!(paused.runtime_percent, None);
    }

    #[test]
    fn frame_profiler_averages_recent_samples_and_evicts_oldest() {
        let mut profiler = FrameProfiler::new();
        let mut summary = None;
        for millis in 1..=121 {
            summary = Some(profiler.record(FrameProfile {
                stages: [
                    Duration::from_millis(millis),
                    Duration::from_millis(2),
                    Duration::from_millis(3),
                    Duration::from_millis(4),
                    Duration::from_millis(5),
                ],
                cpu: CpuProfile::default(),
            }));
        }

        let summary = summary.unwrap();
        assert_eq!(summary.samples, FrameProfiler::HISTORY_SAMPLES);
        assert_eq!(summary.latest.stages[0], Duration::from_millis(121));
        assert_eq!(summary.average.stages[0], Duration::from_micros(61_500));
        assert_eq!(summary.average.total(), Duration::from_micros(75_500));
    }

    #[test]
    fn package_guest_builder_loads_program_at_address_zero() {
        let package_path = std::env::temp_dir().join(format!(
            "cpt32-main-package-{}.cpt32pkg",
            std::process::id()
        ));
        let file = std::fs::File::create(&package_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("manifest.yaml", options).unwrap();
        archive
            .write_all(
                b"format: cpt32pkg\nformat_version: 1\nid: 00000000-0000-4000-8000-000000000001\ntitle: Unit test\nsystem:\n  architecture: SRC32\nentrypoint:\n  type: program\n  path: program.bin\n",
            )
            .unwrap();
        archive.start_file("program.bin", options).unwrap();
        archive.write_all(&[0x12, 0x34, 0x56]).unwrap();
        archive.finish().unwrap();

        let mut machine = build_guest_machine(&package_path, None, None, CpuKind::Scalar).unwrap();
        assert_eq!(machine.cpu.pc(), 0);
        assert_eq!(machine.cpu.read_mem_u8(0), 0x12);
        assert_eq!(machine.cpu.read_mem_u8(1), 0x34);
        drop(machine);
        std::fs::remove_file(package_path).unwrap();
    }

    #[test]
    fn package_guest_builder_uses_injected_data_root_lazily() {
        const ID: &str = "00000000-0000-4000-8000-000000000001";
        let package_path = std::env::temp_dir().join(format!(
            "cpt32-main-injected-root-{}.cpt32pkg",
            std::process::id()
        ));
        let file = std::fs::File::create(&package_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("manifest.yaml", options).unwrap();
        archive
            .write_all(
                format!(
                    "format: cpt32pkg\nformat_version: 1\nid: {ID}\ntitle: Writable disk\nsystem:\n  architecture: SRC32\nentrypoint:\n  type: program\n  path: program.bin\ndisks:\n  - slot: 0\n    type: hdd\n    path: disk.img\n    mode: read_write\n"
                )
                .as_bytes(),
            )
            .unwrap();
        archive.start_file("program.bin", options).unwrap();
        archive.write_all(&[0x12]).unwrap();
        archive.start_file("disk.img", options).unwrap();
        archive.write_all(&[0xA6; 512]).unwrap();
        archive.finish().unwrap();

        let data_root =
            std::env::temp_dir().join(format!("cpt32-main-data-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data_root);
        let machine =
            build_guest_machine(&package_path, None, Some(&data_root), CpuKind::Scalar).unwrap();
        let sidecar = data_root.join(ID).join("dd0.cow");
        assert!(!sidecar.exists());
        assert!(!data_root.exists());
        drop(machine);

        std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
        std::fs::write(&sidecar, b"invalid sidecar").unwrap();
        let before = std::fs::read(&sidecar).unwrap();
        let error =
            match build_guest_machine(&package_path, None, Some(&data_root), CpuKind::Scalar) {
                Ok(_) => panic!("invalid injected sidecar was accepted"),
                Err(error) => error,
            };
        assert!(error.contains("COW"));
        assert_eq!(std::fs::read(&sidecar).unwrap(), before);

        std::fs::remove_file(package_path).unwrap();
        std::fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn package_drop_replaces_guest_transactionally() {
        let package_path = std::env::temp_dir().join(format!(
            "cpt32-drop-package-{}.CPT32PKG",
            std::process::id()
        ));
        let file = std::fs::File::create(&package_path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("manifest.yaml", options).unwrap();
        archive
            .write_all(
                b"format: cpt32pkg\nformat_version: 1\nid: 00000000-0000-4000-8000-000000000001\ntitle: Drop test\nsystem:\n  architecture: SRC32\nentrypoint:\n  type: program\n  path: program.bin\n",
            )
            .unwrap();
        archive.start_file("program.bin", options).unwrap();
        archive.write_all(&[0x12, 0x34]).unwrap();
        archive.finish().unwrap();

        let mut old_bus = Bus::new();
        connect_ram(&mut old_bus);
        old_bus.write_u8(0, 0xA5);
        let old_sgu = connect_sgu(&mut old_bus);
        let old_vdp = connect_vdp(&mut old_bus);
        let old_cpu = AnyCpu::new(CpuKind::Scalar, old_bus);
        let (old_sender, mut old_receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut app = GuiApp {
            cpu: old_cpu,
            vdp: old_vdp,
            sgu: old_sgu,
            idc: None,
            audio_host: None,
            audio_left: Vec::new(),
            audio_right: Vec::new(),
            input_sender: old_sender,
            held_keys: HashSet::from([0x04]),
            held_mouse_buttons: 1,
            mouse_captured: false,
            focused: true,
            gilrs: None,
            gamepad_slots: [None; 4],
            display_handle: None,
            window: None,
            presenter: None,
            debug_gui: None,
            frame_profiler: None,
            cpu_kind: CpuKind::Scalar,
            enable_debug_gui: true,
            start_paused: true,
            music_stream: None,
            title: "CPT32".to_owned(),
            next_frame_deadline: Instant::now(),
        };

        app.handle_dropped_file(&package_path);
        assert_eq!(app.title, "CPT32 — Drop test");
        assert_eq!(app.cpu.pc(), 0);
        assert_eq!(app.cpu.read_mem_u8(0), 0x12);
        assert!(app.held_keys.is_empty());
        assert_eq!(app.held_mouse_buttons, 0);
        assert!(matches!(
            old_receiver.try_recv(),
            Ok(HostInputEvent::Key {
                usage: 0x04,
                down: false
            })
        ));
        assert!(matches!(
            old_receiver.try_recv(),
            Ok(HostInputEvent::MouseButton {
                button: 0,
                down: false
            })
        ));
        assert!(app.enable_debug_gui);
        assert!(app.start_paused);

        let old_pc = app.cpu.pc();
        let old_byte = app.cpu.read_mem_u8(0);
        app.held_keys.insert(0x05);
        let invalid_path = std::env::temp_dir().join(format!(
            "cpt32-drop-invalid-{}.cpt32pkg",
            std::process::id()
        ));
        std::fs::write(&invalid_path, b"invalid package").unwrap();
        app.handle_dropped_file(&invalid_path);
        assert!(app.title.contains("Failed to load"));
        assert_eq!(app.cpu.pc(), old_pc);
        assert_eq!(app.cpu.read_mem_u8(0), old_byte);
        assert_eq!(app.held_keys, HashSet::from([0x05]));
        let failure_title = app.title.clone();
        app.handle_dropped_file(Path::new("ignored.bin"));
        assert_eq!(app.title, failure_title);

        std::fs::remove_file(package_path).unwrap();
        std::fs::remove_file(invalid_path).unwrap();
    }
}
