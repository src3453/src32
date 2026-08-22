//! CPU-reserved address space devices.

use std::cell::RefCell;
use std::rc::Rc;

use crate::bus::Device;

pub const VECTOR_BASE: u32 = 0xFFFF_0000;
pub const VECTOR_SIZE: u32 = 0x104;
pub const RESET_VECTOR: u32 = VECTOR_BASE;
pub const ILLEGAL_INSTRUCTION_VECTOR: u32 = VECTOR_BASE + 0x04;
pub const BUS_ERROR_VECTOR: u32 = VECTOR_BASE + 0x08;
pub const INTERRUPT_VECTOR: u32 = VECTOR_BASE + 0x100;

/// CPU state exposed through the CPU-reserved MMIO area.
///
/// All multi-byte values use the bus' big-endian byte order.  The vector
/// table is writable so firmware can install handler addresses.
#[derive(Debug)]
pub struct CpuRegisterBlock {
    pub regs: [u32; 32],
    pub pc: u32,
    pub epc: u32,
    pub cause: u32,
    pub status: u32,
    pub instr_mode: u32,
    vectors: [u32; 65],
}

impl CpuRegisterBlock {
    pub fn new() -> Self {
        let mut vectors = [0; 65];
        vectors[0] = RESET_VECTOR;
        vectors[1] = ILLEGAL_INSTRUCTION_VECTOR;
        vectors[2] = BUS_ERROR_VECTOR;
        vectors[64] = INTERRUPT_VECTOR;
        Self {
            regs: [0; 32],
            pc: 0,
            epc: 0,
            cause: 0,
            status: 1,
            instr_mode: 0,
            vectors,
        }
    }

    fn read_word(&self, offset: u32) -> u32 {
        if offset < VECTOR_SIZE && offset % 4 == 0 {
            return self.vectors[(offset / 4) as usize];
        }
        match offset {
            0x200..=0x27F if offset % 4 == 0 => self.regs[((offset - 0x200) / 4) as usize],
            0x280 => self.pc,
            0x284 => self.epc,
            0x288 => self.cause,
            0x28C => self.status,
            0x290 => self.instr_mode,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u32, value: u32) {
        if offset < VECTOR_SIZE && offset % 4 == 0 {
            self.vectors[(offset / 4) as usize] = value;
            return;
        }
        match offset {
            0x200..=0x27F if offset % 4 == 0 => {
                let index = ((offset - 0x200) / 4) as usize;
                if index != 0 {
                    self.regs[index] = value;
                }
            }
            0x280 => self.pc = value,
            0x284 => self.epc = value,
            0x288 => self.cause = value,
            0x28C => self.status = value,
            0x290 => self.instr_mode = value & 1,
            _ => {}
        }
    }
}

impl Default for CpuRegisterBlock {
    fn default() -> Self {
        Self::new()
    }
}

impl Device for CpuRegisterBlock {
    fn read(&mut self, addr: u32) -> u8 {
        let aligned = addr & !3;
        let shift = 8 * (3 - (addr & 3));
        (self.read_word(aligned) >> shift) as u8
    }

    fn write(&mut self, addr: u32, value: u8) {
        let aligned = addr & !3;
        let shift = 8 * (3 - (addr & 3));
        let old = self.read_word(aligned);
        let mask = 0xFFu32 << shift;
        self.write_word(aligned, (old & !mask) | (u32::from(value) << shift));
    }

    fn size(&self) -> u32 {
        0x300
    }
}

pub const CPU_REG_BASE: u32 = 0xFFFF_0200;

pub fn connect_cpu_registers(bus: &mut crate::bus::Bus) -> Rc<RefCell<CpuRegisterBlock>> {
    let regs = Rc::new(RefCell::new(CpuRegisterBlock::new()));
    bus.add_device(VECTOR_BASE, Box::new(SharedCpuDevice(regs.clone())));
    bus.add_device(
        CPU_REG_BASE,
        Box::new(SharedCpuRegisterDevice(regs.clone())),
    );
    regs
}

struct SharedCpuDevice(Rc<RefCell<CpuRegisterBlock>>);
impl Device for SharedCpuDevice {
    fn read(&mut self, addr: u32) -> u8 {
        self.0.borrow_mut().read(addr)
    }
    fn write(&mut self, addr: u32, value: u8) {
        self.0.borrow_mut().write(addr, value);
    }
    fn size(&self) -> u32 {
        VECTOR_SIZE
    }
}

struct SharedCpuRegisterDevice(Rc<RefCell<CpuRegisterBlock>>);
impl Device for SharedCpuRegisterDevice {
    fn read(&mut self, addr: u32) -> u8 {
        self.0.borrow_mut().read(addr + 0x200)
    }
    fn write(&mut self, addr: u32, value: u8) {
        self.0.borrow_mut().write(addr + 0x200, value);
    }
    fn size(&self) -> u32 {
        0x94
    }
}
