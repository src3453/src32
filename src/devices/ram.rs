use crate::bus::Device;

pub struct Ram {
    data: Vec<u8>,
}

impl Ram {
    pub fn new(size: usize) -> Self {
        Self {
            data: vec![0; size],
        }
    }
}

impl Device for Ram {
    fn read(&self, addr: u32) -> u8 {
        self.data[addr as usize]
    }

    fn write(&mut self, addr: u32, value: u8) {
        self.data[addr as usize] = value;
    }

    fn size(&self) -> u32 {
        self.data.len() as u32
    }
}

const RAM_SIZE: usize = 0x1000000; // 16MB 
pub fn connect_ram(bus: &mut crate::bus::Bus) {
    bus.add_device(0, Box::new(Ram::new(RAM_SIZE)));
}
