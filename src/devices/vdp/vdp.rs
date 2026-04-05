
// VDP: Video Display Processor
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

pub struct Vdp {
    gp0: Gp0,
    regs: VdpRegs,
}

impl Vdp {
    pub fn new() -> Self {
        Self {
            gp0: Gp0::new(VDP_VRAM_SIZE as usize),
            regs: VdpRegs::new(),
        }
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.gp0.vram
    }

    pub fn framebuffer_mut(&mut self) -> &mut [u8] {
        &mut self.gp0.vram
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