// RAM: Random Access Memory device implementation
// This module defines a simple RAM device that can be connected to the bus in the CPT32 emulator.

use crate::bus::Device;

const INSTRUCTION_PAGE_SIZE: usize = 4096;

pub struct Ram {
    data: Vec<u8>,
    instruction_generations: Option<Vec<u64>>,
}

impl Ram {
    pub fn new(size: usize) -> Self {
        Self {
            data: vec![0; size],
            instruction_generations: None,
        }
    }
}

impl Device for Ram {
    fn read(&mut self, addr: u32) -> u8 {
        self.data[addr as usize]
    }

    fn write(&mut self, addr: u32, value: u8) {
        let index = addr as usize;
        if let Some(generations) = self.instruction_generations.as_mut() {
            if self.data[index] != value {
                let page = index / INSTRUCTION_PAGE_SIZE;
                generations[page] = generations[page].wrapping_add(1);
            }
        }
        self.data[index] = value;
    }

    fn instruction_cache_generation(&self, addr: u32) -> Option<u64> {
        self.instruction_generations
            .as_ref()
            .map(|generations| generations[addr as usize / INSTRUCTION_PAGE_SIZE])
    }

    fn peek_instruction_u8(&self, addr: u32) -> Option<u8> {
        self.data.get(addr as usize).copied()
    }

    fn enable_instruction_cache_tracking(&mut self) {
        self.instruction_generations
            .get_or_insert_with(|| vec![0; self.data.len().div_ceil(INSTRUCTION_PAGE_SIZE)]);
    }

    fn size(&self) -> u32 {
        self.data.len() as u32
    }
}

const RAM_SIZE: usize = 0x1000000; // 16MB
pub fn connect_ram(bus: &mut crate::bus::Bus) {
    bus.add_device(0, "Main RAM", Box::new(Ram::new(RAM_SIZE)));
}
