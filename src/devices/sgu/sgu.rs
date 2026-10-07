// SGU: Sound Generator Unit
// This module provides the bus device wrapper for the 3WS8PN (S3W2) sound generator.
// SGU MMIO: 0x80020000 - 0x800208FF and 0x80021000 - 0x800218FF
// PCMRAM:   0x18000000 - 0x180FFFFF (1MB)

use std::cell::RefCell;
use std::rc::Rc;

use crate::bus::{Bus, Device};
use crate::devices::sgu::s3w2::{PCM_RAM_SIZE, S3w2Sound};

pub const SGU_REG_BASE: u32 = 0x8002_0000;
pub const SGU2_REG_BASE: u32 = 0x8002_1000;
pub const SGU_REG_SIZE: u32 = 0x0000_0900; // 0x900 bytes (0x800 wavetable SRAM + 0x100 registers)

pub const PCM_RAM_BASE: u32 = 0x1800_0000;
pub const PCM_RAM_TOTAL_SIZE: u32 = PCM_RAM_SIZE as u32; // 1MB

pub enum SguPort {
    Regs(usize),
    PcmRam,
}

pub struct SguDevice {
    sgus: [Rc<RefCell<S3w2Sound>>; 2],
    port: SguPort,
}

impl SguDevice {
    pub fn new(sgus: [Rc<RefCell<S3w2Sound>>; 2], port: SguPort) -> Self {
        Self { sgus, port }
    }
}

impl Device for SguDevice {
    fn read(&mut self, addr: u32) -> u8 {
        match self.port {
            SguPort::Regs(chip) => self.sgus[chip].borrow_mut().read_register(addr),
            SguPort::PcmRam => self.sgus[0].borrow().read_pcm_ram(addr),
        }
    }

    fn write(&mut self, addr: u32, value: u8) {
        match self.port {
            SguPort::Regs(chip) => self.sgus[chip].borrow_mut().write_register(addr, value),
            SguPort::PcmRam => {
                // Both cores reference the same RAM; notify each core so cached sample data is invalidated.
                for sgu in &self.sgus {
                    sgu.borrow_mut().write_pcm_ram(addr, value);
                }
            }
        }
    }

    fn size(&self) -> u32 {
        match self.port {
            SguPort::Regs(_) => SGU_REG_SIZE,
            SguPort::PcmRam => PCM_RAM_TOTAL_SIZE,
        }
    }
}

pub fn connect_sgu(bus: &mut Bus) -> [Rc<RefCell<S3w2Sound>>; 2] {
    let pcm_ram = Rc::new(RefCell::new(vec![0; PCM_RAM_SIZE]));
    let sgus = [
        Rc::new(RefCell::new(S3w2Sound::with_pcm_ram(Rc::clone(&pcm_ram)))),
        Rc::new(RefCell::new(S3w2Sound::with_pcm_ram(pcm_ram))),
    ];
    bus.add_device(
        SGU_REG_BASE,
        Box::new(SguDevice::new(sgus.clone(), SguPort::Regs(0))),
    );
    bus.add_device(
        SGU2_REG_BASE,
        Box::new(SguDevice::new(sgus.clone(), SguPort::Regs(1))),
    );
    bus.add_device(
        PCM_RAM_BASE,
        Box::new(SguDevice::new(sgus.clone(), SguPort::PcmRam)),
    );
    sgus
}
