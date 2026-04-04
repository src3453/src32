use crate::bus::Device;

pub struct Gp0 {
    vram: Vec<u8>,
}

impl Gp0 {
    pub fn new(size: usize) -> Self {
        Self {
            vram: vec![0; size],
        }
    }

    pub fn get_vram(&self) -> &[u8] {
        &self.vram
    }
}

impl Device for Gp0 {
    fn read(&self, addr: u32) -> u8 {
        self.vram[addr as usize]
    }

    fn write(&mut self, addr: u32, value: u8) {
        self.vram[addr as usize] = value;
    }

    fn size(&self) -> u32 {
        self.vram.len() as u32
    }
}