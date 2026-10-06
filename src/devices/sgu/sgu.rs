// SGU: Sound Generator Unit
// This module provides the bus device wrapper for the 3WS8PN (S3W2) sound generator.
// SGU MMIO: 0x80020000 - 0x800208FF (2304 bytes)
// PCMRAM:   0x18000000 - 0x180FFFFF (1MB)

use std::cell::RefCell;
use std::rc::Rc;

use crate::bus::{Bus, Device};
use crate::devices::sgu::s3w2::{S3w2Sound, PCM_RAM_SIZE};

pub const SGU_REG_BASE: u32 = 0x8002_0000;
pub const SGU_REG_SIZE: u32 = 0x0000_0900; // 0x900 bytes (0x800 wavetable SRAM + 0x100 registers)

pub const PCM_RAM_BASE: u32 = 0x1800_0000;
pub const PCM_RAM_TOTAL_SIZE: u32 = PCM_RAM_SIZE as u32; // 1MB

pub enum SguPort {
    Regs,
    PcmRam,
}

pub struct SguDevice {
    sgu: Rc<RefCell<S3w2Sound>>,
    port: SguPort,
}

impl SguDevice {
    pub fn new(sgu: Rc<RefCell<S3w2Sound>>, port: SguPort) -> Self {
        Self { sgu, port }
    }
}

impl Device for SguDevice {
    fn read(&mut self, addr: u32) -> u8 {
        match self.port {
            SguPort::Regs => self.sgu.borrow_mut().read_register(addr),
            SguPort::PcmRam => self.sgu.borrow().read_pcm_ram(addr),
        }
    }

    fn write(&mut self, addr: u32, value: u8) {
        match self.port {
            SguPort::Regs => self.sgu.borrow_mut().write_register(addr, value),
            SguPort::PcmRam => self.sgu.borrow_mut().write_pcm_ram(addr, value),
        }
    }

    fn size(&self) -> u32 {
        match self.port {
            SguPort::Regs => SGU_REG_SIZE,
            SguPort::PcmRam => PCM_RAM_TOTAL_SIZE,
        }
    }
}

pub fn connect_sgu(bus: &mut Bus) -> Rc<RefCell<S3w2Sound>> {
    let sgu = Rc::new(RefCell::new(S3w2Sound::new()));
    bus.add_device(
        SGU_REG_BASE,
        Box::new(SguDevice::new(Rc::clone(&sgu), SguPort::Regs)),
    );
    bus.add_device(
        PCM_RAM_BASE,
        Box::new(SguDevice::new(Rc::clone(&sgu), SguPort::PcmRam)),
    );
    sgu
}
