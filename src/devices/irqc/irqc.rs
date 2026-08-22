//! IRQC: external interrupt controller for SRC32.
//!
//! IRQC arbitrates up to sixteen interrupt sources and presents one selected
//! level plus a 4-bit source number to the CPU. The CPU remains responsible for
//! edge detection at its input and for interrupt entry/return.

use crate::bus::Device;

pub const IRQC_MMIO_SIZE: u32 = 0x10;
pub const REG_PENDING: u32 = 0x00;
pub const REG_ENABLE: u32 = 0x04;

const SOURCE_COUNT: usize = 16;
const SOURCE_MASK: u16 = 0xFFFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrqOutput {
    pub valid: bool,
    pub number: u8,
}

/// Simple level output controller with edge-latched source requests.
///
/// A source request is latched by [`IrqController::set_source`] on a rising
/// edge. The lowest numbered enabled pending source is selected. Software
/// clears a pending source by writing its bit to `REG_PENDING`; this is
/// intentionally independent of CPU IRET so an ISR can acknowledge a device
/// before returning.
pub struct IrqController {
    pending: u16,
    enable: u16,
    source_level: u16,
}

impl IrqController {
    pub fn new() -> Self {
        Self {
            pending: 0,
            enable: 0,
            source_level: 0,
        }
    }

    pub fn pending(&self) -> u16 {
        self.pending
    }

    pub fn enable_mask(&self) -> u16 {
        self.enable
    }

    pub fn set_enable_mask(&mut self, mask: u16) {
        self.enable = mask;
    }

    /// Set one source level. Only a low-to-high transition latches pending.
    pub fn set_source(&mut self, number: u8, level: bool) {
        if number >= SOURCE_COUNT as u8 {
            return;
        }
        let bit = 1u16 << number;
        if level && self.source_level & bit == 0 {
            self.pending |= bit;
        }
        if level {
            self.source_level |= bit;
        } else {
            self.source_level &= !bit;
        }
    }

    pub fn clear_pending(&mut self, mask: u16) {
        self.pending &= !(mask & SOURCE_MASK);
    }

    pub fn irq_output(&self) -> IrqOutput {
        let active = self.pending & self.enable;
        match active.trailing_zeros() {
            0..=15 => IrqOutput {
                valid: true,
                number: active.trailing_zeros() as u8,
            },
            _ => IrqOutput {
                valid: false,
                number: 0,
            },
        }
    }
}

impl Default for IrqController {
    fn default() -> Self {
        Self::new()
    }
}

impl Device for IrqController {
    fn read(&mut self, addr: u32) -> u8 {
        let value = match addr {
            0x00..=0x01 => self.pending,
            0x04..=0x05 => self.enable,
            _ => 0,
        };
        let shift = match addr {
            0x00 | 0x04 => 8,
            0x01 | 0x05 => 0,
            _ => 0,
        };
        (value >> shift) as u8
    }

    fn write(&mut self, addr: u32, value: u8) {
        match addr {
            0x00 => self.clear_pending(u16::from(value) << 8),
            0x01 => self.clear_pending(u16::from(value)),
            0x04 => self.enable = (self.enable & 0x00FF) | (u16::from(value) << 8),
            0x05 => self.enable = (self.enable & 0xFF00) | u16::from(value),
            _ => {}
        }
    }

    fn size(&self) -> u32 {
        IRQC_MMIO_SIZE
    }
}

pub const IRQC_BASE: u32 = 0xFFFF_0040;

/// Attach an IRQC instance to the bus. The returned controller is the same
/// state object exposed through MMIO, allowing the platform to drive sources.
pub fn connect_irqc(bus: &mut crate::bus::Bus) -> std::rc::Rc<std::cell::RefCell<IrqController>> {
    let irqc = std::rc::Rc::new(std::cell::RefCell::new(IrqController::new()));
    bus.add_device(IRQC_BASE, Box::new(SharedIrqController(irqc.clone())));
    irqc
}

struct SharedIrqController(std::rc::Rc<std::cell::RefCell<IrqController>>);

impl Device for SharedIrqController {
    fn read(&mut self, addr: u32) -> u8 {
        self.0.borrow_mut().read(addr)
    }

    fn write(&mut self, addr: u32, value: u8) {
        self.0.borrow_mut().write(addr, value);
    }

    fn size(&self) -> u32 {
        IRQC_MMIO_SIZE
    }
}
