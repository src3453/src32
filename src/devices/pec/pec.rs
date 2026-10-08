use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc::{self, UnboundedSender};

use crate::bus::{Bus, Device};

pub const PEC_INPUT_BASE: u32 = 0x8004_0030;
pub const PEC_INPUT_SIZE: u32 = 0x150;
pub const EVENT_CAPACITY: usize = 256;

const EVENT_DATA: u32 = 0x30;
const EVENT_STATUS: u32 = 0x34;
const EVENT_POP: u32 = 0x38;
const MOUSE_X: u32 = 0x40;
const MOUSE_Y: u32 = 0x44;
const MOUSE_DX: u32 = 0x48;
const MOUSE_DY: u32 = 0x4C;
const MOUSE_WHEEL: u32 = 0x50;
const MOUSE_BUTTONS: u32 = 0x54;
const PAD_BASE: u32 = 0x80;
const PAD_STRIDE: u32 = 0x40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostInputEvent {
    Key { usage: u8, down: bool },
    MouseButton { button: u8, down: bool },
    MousePosition { x: u16, y: u16 },
    MouseRelative { dx: i16, dy: i16 },
    MouseWheel { vertical: i16, horizontal: i16 },
    PadConnected { slot: u8 },
    PadDisconnected { slot: u8 },
    PadButton { slot: u8, button: u8, down: bool },
    PadAxis { slot: u8, axis: u8, value: i16 },
    PadTrigger { slot: u8, trigger: u8, value: u16 },
}

#[derive(Clone, Copy, Default)]
struct PadState {
    connected: bool,
    buttons: u16,
    axes: [i16; 4],
    triggers: [u16; 2],
}

pub struct PeCState {
    events: [u32; EVENT_CAPACITY],
    event_head: usize,
    event_len: usize,
    overflow: bool,
    mouse_x: u32,
    mouse_position_valid: bool,
    mouse_y: u32,
    mouse_dx: i32,
    mouse_dy: i32,
    mouse_wheel: i32,
    mouse_buttons: u8,
    pads: [PadState; 4],
}

impl Default for PeCState {
    fn default() -> Self {
        Self {
            events: [0; EVENT_CAPACITY],
            event_head: 0,
            event_len: 0,
            overflow: false,
            mouse_x: 0,
            mouse_y: 0,
            mouse_position_valid: false,
            mouse_dx: 0,
            mouse_dy: 0,
            mouse_wheel: 0,
            mouse_buttons: 0,
            pads: [PadState::default(); 4],
        }
    }
}

impl PeCState {
    fn push_event(&mut self, kind: u8, device: u8, code: u8, value: u16) {
        if self.event_len == EVENT_CAPACITY {
            self.overflow = true;
            return;
        }
        let tail = (self.event_head + self.event_len) % EVENT_CAPACITY;
        self.events[tail] = (u32::from(kind) << 28)
            | (u32::from(device) << 24)
            | (u32::from(code) << 16)
            | u32::from(value);
        self.event_len += 1;
    }

    fn pop_event(&mut self) {
        if self.event_len != 0 {
            self.event_head = (self.event_head + 1) % EVENT_CAPACITY;
            self.event_len -= 1;
        }
    }

    pub fn has_pending_events(&self) -> bool {
        self.event_len != 0
    }

    pub fn apply(&mut self, event: HostInputEvent) {
        match event {
            HostInputEvent::Key { usage, down } if usage != 0 => {
                self.push_event(0, 0, usage, u16::from(down));
            }
            HostInputEvent::MouseButton { button, down } if button < 5 => {
                let mask = 1 << button;
                if down {
                    self.mouse_buttons |= mask;
                } else {
                    self.mouse_buttons &= !mask;
                }
                self.push_event(1, 1, button, u16::from(down));
            }
            HostInputEvent::MousePosition { x, y } => {
                let next_x = u32::from(x).min(639);
                let next_y = u32::from(y).min(479);
                if self.mouse_position_valid {
                    self.mouse_dx = next_x as i32 - self.mouse_x as i32;
                    self.mouse_dy = next_y as i32 - self.mouse_y as i32;
                } else {
                    self.mouse_dx = 0;
                    self.mouse_dy = 0;
                    self.mouse_position_valid = true;
                }
                self.mouse_x = next_x;
                self.mouse_y = next_y;
                self.push_event(2, 1, 0, self.mouse_dx as i16 as u16);
                self.push_event(2, 1, 1, self.mouse_dy as i16 as u16);
            }
            HostInputEvent::MouseRelative { dx, dy } => {
                self.mouse_dx = i32::from(dx);
                self.mouse_dy = i32::from(dy);
                self.push_event(2, 1, 0, dx as u16);
                self.push_event(2, 1, 1, dy as u16);
            }
            HostInputEvent::MouseWheel {
                vertical,
                horizontal,
            } => {
                self.mouse_wheel = i32::from(vertical);
                self.push_event(3, 1, 0, vertical as u16);
                self.push_event(3, 1, 1, horizontal as u16);
            }
            HostInputEvent::PadConnected { slot } if slot < 4 => {
                self.pads[usize::from(slot)] = PadState {
                    connected: true,
                    ..PadState::default()
                };
            }
            HostInputEvent::PadDisconnected { slot } if slot < 4 => {
                let pad = self.pads[usize::from(slot)];
                for button in 0..14 {
                    if pad.buttons & (1 << button) != 0 {
                        self.push_event(4, slot + 2, button, 0);
                    }
                }
                for (axis, value) in pad.axes.into_iter().enumerate() {
                    if value != 0 {
                        self.push_event(5, slot + 2, axis as u8, 0);
                    }
                }
                for (trigger, value) in pad.triggers.into_iter().enumerate() {
                    if value != 0 {
                        self.push_event(5, slot + 2, trigger as u8 + 4, 0);
                    }
                }
                self.pads[usize::from(slot)] = PadState::default();
            }
            HostInputEvent::PadButton { slot, button, down } if slot < 4 && button < 14 => {
                let pad = &mut self.pads[usize::from(slot)];
                if !pad.connected {
                    return;
                }
                let mask = 1 << button;
                if down {
                    pad.buttons |= mask;
                } else {
                    pad.buttons &= !mask;
                }
                self.push_event(4, slot + 2, button, u16::from(down));
            }
            HostInputEvent::PadAxis { slot, axis, value } if slot < 4 && axis < 4 => {
                let pad = &mut self.pads[usize::from(slot)];
                if !pad.connected {
                    return;
                }
                pad.axes[usize::from(axis)] = value;
                self.push_event(5, slot + 2, axis, value as u16);
            }
            HostInputEvent::PadTrigger {
                slot,
                trigger,
                value,
            } if slot < 4 && trigger < 2 => {
                let pad = &mut self.pads[usize::from(slot)];
                if !pad.connected {
                    return;
                }
                pad.triggers[usize::from(trigger)] = value;
                self.push_event(5, slot + 2, trigger + 4, value);
            }
            _ => {}
        }
    }

    fn event_status(&self) -> u32 {
        u32::from(self.event_len != 0) | (u32::from(self.overflow) << 1)
    }

    fn pad_register(&self, offset: u32) -> u32 {
        let slot = ((offset - PAD_BASE) / PAD_STRIDE) as usize;
        let register = (offset - PAD_BASE) % PAD_STRIDE;
        let pad = self.pads[slot];
        match register {
            0x00 => {
                u32::from(pad.connected)
                    | (u32::from(pad.connected) << 1)
                    | (u32::from(pad.connected) << 2)
            }
            0x04 => u32::from(pad.buttons),
            0x08 => pad.axes[0] as i32 as u32,
            0x0C => pad.axes[1] as i32 as u32,
            0x10 => pad.axes[2] as i32 as u32,
            0x14 => pad.axes[3] as i32 as u32,
            0x18 => u32::from(pad.triggers[0]) | (u32::from(pad.triggers[1]) << 16),
            0x1C => {
                if pad.connected {
                    (slot + 1) as u32
                } else {
                    0
                }
            }
            0x20 => {
                if pad.connected {
                    7
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    fn read_register(&self, offset: u32) -> u32 {
        match offset {
            EVENT_DATA => {
                if self.event_len == 0 {
                    0
                } else {
                    self.events[self.event_head]
                }
            }
            EVENT_STATUS => self.event_status(),
            MOUSE_X => self.mouse_x,
            MOUSE_Y => self.mouse_y,
            MOUSE_DX => self.mouse_dx as u32,
            MOUSE_DY => self.mouse_dy as u32,
            MOUSE_WHEEL => self.mouse_wheel as u32,
            MOUSE_BUTTONS => u32::from(self.mouse_buttons),
            PAD_BASE..=0x17F => self.pad_register(offset),
            _ => 0,
        }
    }
}

fn lock_state(state: &Mutex<PeCState>) -> MutexGuard<'_, PeCState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct PeCInputDevice {
    state: Arc<Mutex<PeCState>>,
    pop_latch: [u8; 4],
}

impl Device for PeCInputDevice {
    fn read(&mut self, addr: u32) -> u8 {
        let offset = addr + 0x30;
        let register = offset & !3;
        let shift = (3 - (offset & 3)) * 8;
        (lock_state(&self.state).read_register(register) >> shift) as u8
    }

    fn write(&mut self, addr: u32, value: u8) {
        let offset = addr + 0x30;
        if offset == EVENT_STATUS + 3 && value & 2 != 0 {
            lock_state(&self.state).overflow = false;
        }
        if (EVENT_POP..EVENT_POP + 4).contains(&offset) {
            self.pop_latch[(offset - EVENT_POP) as usize] = value;
            if offset == EVENT_POP + 3 {
                if u32::from_be_bytes(self.pop_latch) == 1 {
                    lock_state(&self.state).pop_event();
                }
                self.pop_latch = [0; 4];
            }
        }
    }

    fn size(&self) -> u32 {
        PEC_INPUT_SIZE
    }
}

/// Register PeC keyboard, mouse, and controller MMIO and process host input off-thread.
/// The returned sender may be used by a non-`Send` GUI thread; CPU and Bus stay there.
pub fn connect_pec_input(bus: &mut Bus) -> UnboundedSender<HostInputEvent> {
    let state = Arc::new(Mutex::new(PeCState::default()));
    bus.attach_pec_input(Arc::clone(&state));
    bus.add_device(
        PEC_INPUT_BASE,
        "PeC Input MMIO",
        Box::new(PeCInputDevice {
            state: Arc::clone(&state),
            pop_latch: [0; 4],
        }),
    );

    let (sender, mut receiver) = mpsc::unbounded_channel::<HostInputEvent>();
    std::thread::Builder::new()
        .name("pec-input".to_string())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
                eprintln!("Warning: Failed to create PeC input runtime");
                return;
            };
            runtime.block_on(async move {
                while let Some(event) = receiver.recv().await {
                    lock_state(&state).apply(event);
                }
            });
        })
        .expect("Failed to start PeC input thread");
    sender
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::irqc::irqc::connect_irqc;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    fn device_with_state(state: PeCState) -> PeCInputDevice {
        PeCInputDevice {
            state: Arc::new(Mutex::new(state)),
            pop_latch: [0; 4],
        }
    }

    fn read_be(device: &mut PeCInputDevice, register: u32) -> u32 {
        (0..4).fold(0, |value, byte| {
            (value << 8) | u32::from(device.read(register - 0x30 + byte))
        })
    }

    fn write_be(device: &mut PeCInputDevice, register: u32, value: u32) {
        for byte in 0..4 {
            device.write(register - 0x30 + byte, (value >> ((3 - byte) * 8)) as u8);
        }
    }
    fn bus_addr(register: u32) -> u32 {
        PEC_INPUT_BASE + register - 0x30
    }

    #[test]
    fn fifo_peeks_pops_and_sticky_overflow_clears_w1c() {
        let mut state = PeCState::default();
        for _ in 0..EVENT_CAPACITY {
            state.apply(HostInputEvent::Key {
                usage: 1,
                down: true,
            });
        }
        state.apply(HostInputEvent::Key {
            usage: 0x04,
            down: false,
        });
        let mut device = device_with_state(state);

        assert_eq!(read_be(&mut device, EVENT_STATUS), 3);
        assert_eq!(read_be(&mut device, EVENT_DATA), 0x0001_0001);
        assert_eq!(read_be(&mut device, EVENT_DATA), 0x0001_0001);
        write_be(&mut device, EVENT_POP, 1);
        assert_eq!(read_be(&mut device, EVENT_DATA), 0x0001_0001);
        device.write((EVENT_STATUS + 3) - 0x30, 2);
        assert_eq!(read_be(&mut device, EVENT_STATUS), 1);

        for _ in 1..EVENT_CAPACITY {
            write_be(&mut device, EVENT_POP, 1);
        }
        assert_eq!(read_be(&mut device, EVENT_STATUS), 0);
        assert_eq!(read_be(&mut device, EVENT_DATA), 0);
    }

    #[test]
    fn mouse_registers_track_clamped_coordinates_and_latest_deltas() {
        let mut state = PeCState::default();
        state.apply(HostInputEvent::MousePosition {
            x: u16::MAX,
            y: u16::MAX,
        });
        state.apply(HostInputEvent::MouseButton {
            button: 0,
            down: true,
        });
        state.apply(HostInputEvent::MouseWheel {
            vertical: -120,
            horizontal: 60,
        });
        let mut device = device_with_state(state);

        assert_eq!(read_be(&mut device, MOUSE_X), 639);
        assert_eq!(read_be(&mut device, MOUSE_Y), 479);
        assert_eq!(read_be(&mut device, MOUSE_DX), 0);
        assert_eq!(read_be(&mut device, MOUSE_DY), 0);
        assert_eq!(read_be(&mut device, MOUSE_WHEEL), (-120_i32) as u32);
        assert_eq!(read_be(&mut device, MOUSE_BUTTONS), 1);

        lock_state(&device.state).apply(HostInputEvent::MousePosition { x: 100, y: 120 });
        assert_eq!(read_be(&mut device, MOUSE_DX), (-539_i32) as u32);
        assert_eq!(read_be(&mut device, MOUSE_DY), (-359_i32) as u32);
        assert_eq!(read_be(&mut device, MOUSE_DX), (-539_i32) as u32);

        lock_state(&device.state).apply(HostInputEvent::MouseRelative { dx: -2, dy: 3 });
        assert_eq!(read_be(&mut device, MOUSE_X), 100);
        assert_eq!(read_be(&mut device, MOUSE_Y), 120);
        assert_eq!(read_be(&mut device, MOUSE_DX), (-2_i32) as u32);
        assert_eq!(read_be(&mut device, MOUSE_DY), 3);
    }

    #[test]
    fn pad_registers_normalize_signed_axes_triggers_and_buttons() {
        let mut state = PeCState::default();
        for slot in 0..4 {
            state.apply(HostInputEvent::PadConnected { slot });
        }
        state.apply(HostInputEvent::PadButton {
            slot: 2,
            button: 0,
            down: true,
        });
        state.apply(HostInputEvent::PadButton {
            slot: 2,
            button: 13,
            down: true,
        });
        state.apply(HostInputEvent::PadAxis {
            slot: 2,
            axis: 1,
            value: -1234,
        });
        state.apply(HostInputEvent::PadTrigger {
            slot: 2,
            trigger: 0,
            value: 0x1234,
        });
        state.apply(HostInputEvent::PadTrigger {
            slot: 2,
            trigger: 1,
            value: 0xABCD,
        });
        let mut device = device_with_state(state);

        for slot in 0_u8..4 {
            let base = PAD_BASE + u32::from(slot) * PAD_STRIDE;
            assert_eq!(read_be(&mut device, base), 7);
            assert_eq!(read_be(&mut device, base + 0x1C), u32::from(slot + 1));
            assert_eq!(read_be(&mut device, base + 0x20), 7);
        }
        let slot = PAD_BASE + 2 * PAD_STRIDE;
        assert_eq!(read_be(&mut device, slot + 4), (1 << 13) | 1);
        assert_eq!(read_be(&mut device, slot + 0x0C), (-1234_i32) as u32);
        assert_eq!(read_be(&mut device, slot + 0x18), 0xABCD_1234);
        assert_eq!(read_be(&mut device, slot + 0x24), 0);
        assert_eq!(read_be(&mut device, 0x17C), 0);
    }

    #[test]
    fn pad_disconnect_releases_non_neutral_controls() {
        let mut state = PeCState::default();
        state.apply(HostInputEvent::PadConnected { slot: 0 });
        state.apply(HostInputEvent::PadButton {
            slot: 0,
            button: 3,
            down: true,
        });
        state.apply(HostInputEvent::PadAxis {
            slot: 0,
            axis: 2,
            value: -123,
        });
        state.apply(HostInputEvent::PadTrigger {
            slot: 0,
            trigger: 1,
            value: 456,
        });
        state.apply(HostInputEvent::PadDisconnected { slot: 0 });
        let mut device = device_with_state(state);

        assert_eq!(read_be(&mut device, PAD_BASE), 0);
        assert_eq!(read_be(&mut device, PAD_BASE + 4), 0);
        assert_eq!(read_be(&mut device, PAD_BASE + 0x10), 0);
        assert_eq!(read_be(&mut device, PAD_BASE + 0x18), 0);
        for event in [
            0x4203_0001,
            0x5202_FF85,
            0x5205_01C8,
            0x4203_0000,
            0x5202_0000,
            0x5205_0000,
        ] {
            assert_eq!(read_be(&mut device, EVENT_DATA), event);
            write_be(&mut device, EVENT_POP, 1);
        }
        assert_eq!(read_be(&mut device, EVENT_STATUS), 0);
    }

    #[test]
    fn tokio_keyboard_event_reaches_mmio_and_irq() {
        let mut bus = Bus::new();
        let sender = connect_pec_input(&mut bus);
        let irq = connect_irqc(&mut bus);
        bus.attach_irq_controller(Rc::clone(&irq));
        sender
            .send(HostInputEvent::Key {
                usage: 0x04,
                down: true,
            })
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while bus.read_u32_be(bus_addr(EVENT_STATUS)) & 1 == 0 {
            assert!(
                Instant::now() < deadline,
                "PeC event task did not process input"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(bus.read_u32_be(bus_addr(EVENT_DATA)), 0x0004_0001);
        assert_eq!(bus.read_u32_be(bus_addr(EVENT_DATA)), 0x0004_0001);

        bus.tick_devices();
        assert_ne!(irq.borrow().pending() & (1 << 3), 0);
        bus.write_u32_be(bus_addr(EVENT_POP), 1);
        bus.tick_devices();
        assert_eq!(bus.read_u32_be(bus_addr(EVENT_STATUS)) & 1, 0);
        irq.borrow_mut().clear_pending(1 << 3);
        bus.tick_devices();
        assert_eq!(irq.borrow().pending() & (1 << 3), 0);
    }
}
