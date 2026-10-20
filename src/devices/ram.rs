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

    fn debug_read(&mut self, addr: u32) -> Option<u8> {
        self.data.get(addr as usize).copied()
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

    fn is_plain_memory(&self) -> bool {
        true
    }
}

pub const DEFAULT_RAM_SIZE: usize = 0x1000000; // 16 MiB
pub const MAX_MEMORY_MIB: u64 = 256;

pub fn connect_ram(bus: &mut crate::bus::Bus) {
    connect_ram_with_size(bus, DEFAULT_RAM_SIZE)
}

pub fn connect_ram_with_size(bus: &mut crate::bus::Bus, size: usize) {
    bus.add_device(0, "Main RAM", Box::new(Ram::new(size)));
}
