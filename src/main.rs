// Main entry point for the CPT32 emulator

use std::cell::RefCell;
use std::collections::HashSet;
use std::env;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cpt32::bus::Bus;
use cpt32::cpu::{Cpu, InstructionMode};
use cpt32::devices::pec::idc::idc::connect_idc_from_config;
use cpt32::devices::pec::pec::{HostInputEvent, connect_pec_input};
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::pec::serial::connect_uart;
use cpt32::devices::ram::connect_ram;
use cpt32::devices::sgu::s3w2::S3w2Sound;
use cpt32::devices::sgu::sgu::connect_sgu;
use cpt32::devices::vdp::vdp::{Vdp, connect_vdp_with_font};
use gilrs::{
    Axis as GilrsAxis, Button as GilrsButton, EventType as GilrsEventType, GamepadId, Gilrs,
};
use imgui::{Condition, Ui};
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

fn load_binary_data(path: &str, bus: &mut Bus) {
    // Utility used to load programs and datas into the bus memory
    let data = std::fs::read(path).expect("Failed to read binary file");
    for (i, byte) in data.iter().enumerate() {
        bus.write_u8(i as u32, *byte);
    }
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

fn viewport_scale(size: PhysicalSize<u32>) -> f64 {
    if size.width == 0 || size.height == 0 {
        return 0.0;
    }
    (f64::from(size.width) / f64::from(render::PRESENT_WIDTH))
        .min(f64::from(size.height) / f64::from(render::PRESENT_HEIGHT))
}

fn mouse_position_to_guest(position: PhysicalPosition<f64>, size: PhysicalSize<u32>) -> (u16, u16) {
    let scale = viewport_scale(size);
    if scale == 0.0 {
        return (0, 0);
    }
    let viewport_x = (f64::from(size.width) - f64::from(render::PRESENT_WIDTH) * scale) * 0.5;
    let viewport_y = (f64::from(size.height) - f64::from(render::PRESENT_HEIGHT) * scale) * 0.5;
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
                eprintln!("Warning: Failed to capture mouse cursor: {error}");
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
    cpu: Cpu,
    vdp: Rc<RefCell<Vdp>>,
    sgu: [Rc<RefCell<S3w2Sound>>; 2],
    audio_host: Option<audio::AudioHost>,
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
        let input_sender = connect_pec_input(&mut bus);
        connect_ram(&mut bus);
        cpt32::bus::connect_bmc_dmac(&mut bus);
        connect_uart(&mut bus);
        connect_rng(&mut bus);
        connect_idc_from_config(&mut bus)
            .unwrap_or_else(|error| panic!("Invalid disk configuration: {error}"));
        let sgu = connect_sgu(&mut bus);

        load_binary_data(program_path, &mut bus);
        let vdp = connect_vdp_with_font(&mut bus, font_path.map(Path::new));

        let (gilrs, gamepad_slots) = match Gilrs::new() {
            Ok(gilrs) => {
                let slots = initial_gamepad_slots(&gilrs, &input_sender);
                (Some(gilrs), slots)
            }
            Err(error) => {
                eprintln!("Warning: Failed to initialize gamepad input: {error}");
                (None, [None; 4])
            }
        };

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
            input_sender,
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
            enable_debug_gui,
            start_paused,
            next_frame_deadline: Instant::now(),
        }
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
    instructions: u128,
    cpu_utilization_percent: f64,
    kips: f64,
}

impl PerformanceStats {
    fn from_deltas(running_cycles: u128, instructions: u128, elapsed: Duration) -> Self {
        let cpu_utilization_percent =
            running_cycles as f64 * 100.0 / f64::from(cpt32::cpu::CYCLES_PER_FRAME);
        let kips = if elapsed.is_zero() {
            0.0
        } else {
            instructions as f64 / elapsed.as_secs_f64() / 1_000.0
        };

        Self {
            running_cycles,
            instructions,
            cpu_utilization_percent,
            kips,
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
    performance: Option<PerformanceStats>,
}

#[derive(Clone, Copy)]
enum DebugWindow {
    Controls,
    Disassembly,
    Memory,
    Compositor,
    Performance,
}

impl DebugWindow {
    const ALL: [Self; Self::COUNT] = [
        Self::Controls,
        Self::Disassembly,
        Self::Memory,
        Self::Compositor,
        Self::Performance,
    ];
    const COUNT: usize = 5;

    fn label(self) -> &'static str {
        match self {
            Self::Controls => "Debug Controls",
            Self::Disassembly => "Realtime Disassembly",
            Self::Memory => "Realtime Memory Monitor",
            Self::Compositor => "Compositor Debug",
            Self::Performance => "Performance",
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
            performance: None,
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
        if self.visible_windows[DebugWindow::Performance as usize] {
            self.draw_performance(ui);
        }
    }

    fn draw_performance(&self, ui: &Ui) {
        ui.window("Performance")
            .size([360.0, 130.0], Condition::FirstUseEver)
            .build(|| {
                if let Some(stats) = self.performance {
                    ui.text(format!(
                        "CPU utilization: {:.1}% ({} / {} running cycles per VSync)",
                        stats.cpu_utilization_percent,
                        stats.running_cycles,
                        cpt32::cpu::CYCLES_PER_FRAME
                    ));
                    ui.text(format!("Instruction throughput: {:.1} KIPS", stats.kips));
                    ui.text(format!("Instructions this VSync: {}", stats.instructions));
                } else {
                    ui.text("Waiting for the first VSync sample...");
                }
            });
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
    last_performance_sample: Option<(Instant, u128, u128)>,
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
            last_performance_sample: None,
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
        let sample_time = Instant::now();
        let running_cycles = cpu.running_cycles();
        let instructions = cpu.instructions_executed();
        if let Some((last_time, last_running_cycles, last_instructions)) =
            self.last_performance_sample
        {
            self.state.performance = Some(PerformanceStats::from_deltas(
                running_cycles.saturating_sub(last_running_cycles),
                instructions.saturating_sub(last_instructions),
                sample_time.duration_since(last_time),
            ));
        }
        self.last_performance_sample = Some((sample_time, running_cycles, instructions));

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
        self.honor_imgui_mouse_capture();

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) {
                    event_loop.exit();
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    if code == KeyCode::F1 {
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
                    } else if let Some(usage) = keyboard_hid_usage(code) {
                        let down = event.state == ElementState::Pressed;
                        if down {
                            self.held_keys.insert(usage);
                        } else {
                            self.held_keys.remove(&usage);
                        }
                        let _ = self.input_sender.send(HostInputEvent::Key { usage, down });
                    }
                }
            }
            WindowEvent::Focused(focused) => {
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
                        let (x, y) = mouse_position_to_guest(position, window.inner_size());
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
                    let scale = viewport_scale(window.inner_size());
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

#[cfg(test)]
mod input_tests {
    use super::*;

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
            mouse_position_to_guest(PhysicalPosition::new(16.0, 16.0), size),
            (0, 0)
        );
        assert_eq!(
            mouse_position_to_guest(PhysicalPosition::new(1294.0, 974.0), size),
            (639, 479)
        );
        assert_eq!(
            mouse_position_to_guest(PhysicalPosition::new(-100.0, 4000.0), size),
            (0, 479)
        );

        let wide = PhysicalSize::new(2000, 600);
        let scale = viewport_scale(wide);
        let viewport_y = (600.0 - f64::from(render::PRESENT_HEIGHT) * scale) * 0.5;
        let content_top = viewport_y + f64::from(render::BORDER_SIZE) * scale;
        assert_eq!(
            mouse_position_to_guest(PhysicalPosition::new(1000.0, content_top), wide).1,
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
    fn calculates_vsync_cpu_utilization_and_real_time_kips() {
        let stats = PerformanceStats::from_deltas(400_000, 6_000, Duration::from_millis(500));

        assert_eq!(stats.running_cycles, 400_000);
        assert_eq!(stats.instructions, 6_000);
        assert_eq!(stats.cpu_utilization_percent, 50.0);
        assert_eq!(stats.kips, 12.0);
    }

    #[test]
    fn zero_elapsed_time_produces_zero_kips() {
        let stats = PerformanceStats::from_deltas(0, 100, Duration::ZERO);
        assert_eq!(stats.kips, 0.0);
    }
}
