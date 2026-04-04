use crate::bus::Device;

pub struct VdpRegs {
    pub display_enable: bool,
}

impl VdpRegs {
    pub fn new() -> Self {
        Self {
            display_enable: true,
        }
    }
}

impl Device for VdpRegs {
    fn read(&self, addr: u32) -> u8 {
        match addr {
            0x00 => self.display_enable as u8,
            _ => 0,
        }
    }

    fn write(&mut self, addr: u32, value: u8) {
        match addr {
            0x00 => self.display_enable = (value & 1) != 0,
            _ => {}
        }
    }

    fn size(&self) -> u32 {
        0x100
    }
}