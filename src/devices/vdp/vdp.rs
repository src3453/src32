// VDP: Video Display Processor
// This module implements the VDP, which is responsible for rendering graphics in the CPT32 emulator.
// It includes VRAM for storing pixel data and registers for controlling display settings.
// VRAM: 0x10000000 - 0x103FFFFF
// Memory-mapped I/O at 0x80030000 - 0x8003FFFF

use std::cell::RefCell;
use std::rc::Rc;

use crate::bus::{Bus, Device};
use crate::devices::vdp::gp::Gp0;
use crate::devices::vdp::reg::VdpRegs;

pub const VDP_VRAM_BASE: u32 = 0x10000000; 
pub const VDP_VRAM_SIZE: u32 = 0x00400000; // 4MB
pub const VDP_REG_BASE: u32 = 0x80030000;
pub const VDP_REG_SIZE: u32 = 0x00000100;

pub const VDP_CLOCK_DIVIDER: u32 = 4; // VDP runs at 1/4 of master clock
pub const PIXEL_CLOCK_DIVIDER: u32 = 8; // Pixel clock is 1/8 of master clock
pub const VDP_CLOCK: u32 = crate::sys::MASTER_CLOCK / VDP_CLOCK_DIVIDER; // 12MHz
pub const PIXEL_CLOCK: u32 = crate::sys::MASTER_CLOCK / PIXEL_CLOCK_DIVIDER; // 6MHz

pub struct VdpState {
    tick_count: u64,
    rendered_frames: u64,
    scanline: u16,
}

pub struct Vdp {
    gp0: Gp0,
    regs: VdpRegs,
    state: VdpState,
}

impl Vdp {
    pub fn new() -> Self {
        Self {
            gp0: Gp0::new(VDP_VRAM_SIZE as usize),
            regs: VdpRegs::new(),
            state: VdpState { tick_count: 0, rendered_frames: 0, scanline: 0 },
        }   
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.gp0.vram
    }

    pub fn framebuffer_mut(&mut self) -> &mut [u8] {
        &mut self.gp0.vram
    }

    pub fn tick(&mut self) {
        // Placeholder for VDP logic (e.g., rendering, timing)
        self.state.tick_count += 1;
    }
}

#[derive(Clone, Copy)]
enum VdpPort {
    Vram,
    Regs,
}

struct VdpDevice {
    vdp: Rc<RefCell<Vdp>>,
    port: VdpPort,
}

impl VdpDevice {
    fn new(vdp: Rc<RefCell<Vdp>>, port: VdpPort) -> Self {
        Self { vdp, port }
    }
}

impl Device for VdpDevice {
    fn read(&self, addr: u32) -> u8 {
        match self.port {
            VdpPort::Vram => self.vdp.borrow().gp0.vram[addr as usize],
            VdpPort::Regs => {
                let vdp = self.vdp.borrow();
                match addr {
                    0x00 => vdp.regs.display_enable as u8,
                    _ => 0,
                }
            }
        }
    }

    fn write(&mut self, addr: u32, value: u8) {
        match self.port {
            VdpPort::Vram => self.vdp.borrow_mut().gp0.vram[addr as usize] = value,
            VdpPort::Regs => {
                if addr == 0x00 {
                    self.vdp.borrow_mut().regs.display_enable = (value & 1) != 0;
                }
            }
        }
    }

    fn size(&self) -> u32 {
        match self.port {
            VdpPort::Vram => VDP_VRAM_SIZE,
            VdpPort::Regs => VDP_REG_SIZE,
        }
    }
}

pub fn connect_vdp(bus: &mut Bus) -> Rc<RefCell<Vdp>> {
    let vdp = Rc::new(RefCell::new(Vdp::new()));
    bus.add_device(VDP_VRAM_BASE, Box::new(VdpDevice::new(Rc::clone(&vdp), VdpPort::Vram)));
    bus.add_device(VDP_REG_BASE, Box::new(VdpDevice::new(Rc::clone(&vdp), VdpPort::Regs)));
    vdp
}