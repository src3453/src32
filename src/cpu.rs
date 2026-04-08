// CPU: Central Processing Unit implementation
// This module defines the SRC32 CPU struct, instruction set, and execution logic for the CPT32 emulator.

use crate::bus::Bus;

pub const CPU_CLOCK: u32 = crate::sys::MASTER_CLOCK; // 48MHz
pub const CYCLES_PER_FRAME: u32 = crate::cpu::CPU_CLOCK / crate::sys::FRAME_RATE; // 800,000 cycles/frame
pub const CYCLES_PER_SCANLINE: u32 = CYCLES_PER_FRAME / 240; // 3,333 cycles/scanline

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddrMode {
    Register,
    Immediate,
    Memory,
    Extension,
}

impl AddrMode {
    fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::Register,
            1 => Self::Immediate,
            2 => Self::Memory,
            3 => Self::Extension,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Instruction {
    Nop,
    Ld { rd: u8, base: u8, offset: i16 },
    St { rd: u8, base: u8, offset: i16 },
    Ldi { rd: u8, imm: u32 },
    Add { rd: u8, rs1: u8, rs2: u8 },
    Addi { rd: u8, rs1: u8, imm: i16 },
    Sub { rd: u8, rs1: u8, rs2: u8 },
    Slt { rd: u8, rs1: u8, rs2: u8 },
    Beq { rs1: u8, rs2: u8, offset: i16 },
    Bne { rs1: u8, rs2: u8, offset: i16 },
    Jmp { offset: i16 },
    Jal { offset: i16 },
    Jr { rd: u8 },
    Cpuid,
    Halt,
    And { rd: u8, rs1: u8, rs2: u8 },
    Or { rd: u8, rs1: u8, rs2: u8 },
    Xor { rd: u8, rs1: u8, rs2: u8 },
    Sll { rd: u8, rs1: u8, rs2: u8 },
    Srl { rd: u8, rs1: u8, rs2: u8 },
    Sla { rd: u8, rs1: u8, rs2: u8 },
    Sra { rd: u8, rs1: u8, rs2: u8 },
    Ldb { rd: u8, base: u8, offset: i16 },
    Ldh { rd: u8, base: u8, offset: i16 },
    Stb { rd: u8, base: u8, offset: i16 },
    Sth { rd: u8, base: u8, offset: i16 },
    Unknown(u64),
}

const REG_ZERO: usize = 0;
const REG_CPUID: usize = 1;
const REG_FEATURES: usize = 2;
const REG_LR: usize = 31;

const EXT_BASE: u32 = 0x01; // Base extension: includes basic arithmetic and logic instructions (ADD, SUB, AND, OR, etc.)
const EXT_A: u32 = 0x02; // Extension A (Arithmetic): adds more arithmetic and logic instructions
const EXT_L: u32 = 0x04; // Extension L (Load/Store): adds byte/halfword load/store instructions
const CPU_FEATURES: u32 = EXT_BASE | EXT_A | EXT_L; // CPUID features bitfield
const CPU_ID: u32 = 0x5352_4332; // "SRC2" style tag

const INSN_SIZE: u32 = 5;

pub struct Cpu {
    reg: [u32; 32],
    pc: u32,
    running: bool,
    bus: Bus,
    cycles: u128,
}
    
impl Cpu {
    pub fn new(bus: Bus) -> Self {
        Self {
            reg: [0; 32],
            pc: 0,
            running: true,
            bus,
            cycles: 0,
        }
    }

    pub fn reset(&mut self, pc: u32) {
        self.reg = [0; 32];
        self.pc = pc;
        self.running = true;
    }

    pub fn load_program(&mut self, base: u32, image: &[u8]) {
        for (idx, &byte) in image.iter().enumerate() {
            self.bus.write_u8(base.wrapping_add(idx as u32), byte);
        }
    }

    pub fn read_reg(&self, reg: usize) -> u32 {
        if reg >= self.reg.len() {
            panic!("Invalid register index: {reg}");
        }
        if reg == REG_ZERO {
            0
        } else {
            self.reg[reg]
        }
    }

    fn write_reg(&mut self, reg: usize, value: u32) {
        if reg >= self.reg.len() {
            panic!("Invalid register index: {reg}");
        }
        if reg == REG_ZERO {
            return;
        }
        self.reg[reg] = value;
    }

    fn fetch_u40(&self) -> u64 {
        let b0 = self.bus.read_u8(self.pc) as u64;
        let b1 = self.bus.read_u8(self.pc.wrapping_add(1)) as u64;
        let b2 = self.bus.read_u8(self.pc.wrapping_add(2)) as u64;
        let b3 = self.bus.read_u8(self.pc.wrapping_add(3)) as u64;
        let b4 = self.bus.read_u8(self.pc.wrapping_add(4)) as u64;
        (b0 << 32) | (b1 << 24) | (b2 << 16) | (b3 << 8) | b4
    }

    fn decode(raw: u64) -> Instruction {
        let op = ((raw >> 32) & 0xFF) as u8;

        if (0xE0..=0xFF).contains(&op) {
            let rd = op & 0x1F;
            let imm = (raw & 0xFFFF_FFFF) as u32;
            return Instruction::Ldi { rd, imm };
        }

        let mode = AddrMode::from_bits(((raw >> 30) & 0x03) as u8);
        let rd = ((raw >> 25) & 0x1F) as u8;
        let rs1 = ((raw >> 20) & 0x1F) as u8;
        let rs2 = ((raw >> 15) & 0x1F) as u8;
        let imm16 = ((raw >> 4) & 0xFFFF) as i16;

        match (op, mode) {
            (0x00, AddrMode::Register) => Instruction::Nop,
            (0x01, AddrMode::Memory) => Instruction::Ld {
                rd,
                base: rs1,
                offset: imm16,
            },
            (0x02, AddrMode::Memory) => Instruction::St {
                rd,
                base: rs1,
                offset: imm16,
            },
            (0x03, AddrMode::Register) => Instruction::Add { rd, rs1, rs2 },
            (0x04, AddrMode::Immediate) => Instruction::Addi {
                rd,
                rs1,
                imm: imm16,
            },
            (0x05, AddrMode::Register) => Instruction::Sub { rd, rs1, rs2 },
            (0x06, AddrMode::Register) => Instruction::Slt { rd, rs1, rs2 },
            (0x07, AddrMode::Immediate) => Instruction::Beq {
                rs1,
                rs2: rd,
                offset: imm16,
            },
            (0x08, AddrMode::Immediate) => Instruction::Bne {
                rs1,
                rs2: rd,
                offset: imm16,
            },
            (0x09, AddrMode::Immediate) => Instruction::Jmp { offset: imm16 },
            (0x0A, AddrMode::Immediate) => Instruction::Jal { offset: imm16 },
            (0x0B, AddrMode::Register) => Instruction::Jr { rd },
            (0x0C, AddrMode::Register) => Instruction::And { rd, rs1, rs2 },
            (0x0D, AddrMode::Register) => Instruction::Or { rd, rs1, rs2 },
            (0x0E, AddrMode::Register) => Instruction::Xor { rd, rs1, rs2 },
            (0x0F, AddrMode::Register) => Instruction::Sll { rd, rs1, rs2 },
            (0x10, AddrMode::Register) => Instruction::Srl { rd, rs1, rs2 },
            (0x11, AddrMode::Register) => Instruction::Sla { rd, rs1, rs2 },
            (0x12, AddrMode::Register) => Instruction::Sra { rd, rs1, rs2 },
            (0x13, AddrMode::Memory) => Instruction::Ldb {
                rd,
                base: rs1,
                offset: imm16,
            },
            (0x14, AddrMode::Memory) => Instruction::Ldh {
                rd,
                base: rs1,
                offset: imm16,
            },
            (0x15, AddrMode::Memory) => Instruction::Stb {
                rd,
                base: rs1,
                offset: imm16,
            },
            (0x16, AddrMode::Memory) => Instruction::Sth {
                rd,
                base: rs1,
                offset: imm16,
            },
            (0xDE, AddrMode::Register) => Instruction::Cpuid,
            (0xDF, AddrMode::Register) => Instruction::Halt,
            _ => Instruction::Unknown(raw),
        }
    }

    fn add_signed(base: u32, offset: i16) -> u32 {
        base.wrapping_add((offset as i32) as u32)
    }

    fn branch_target(next_pc: u32, offset: i16) -> u32 {
        next_pc.wrapping_add((offset as i32) as u32)
    }

    fn execute(&mut self, insn: Instruction) {
        let next_pc = self.pc.wrapping_add(INSN_SIZE);
        self.pc = next_pc;

        match insn {
            Instruction::Nop => {}
            Instruction::Ld { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.bus.read_u32_be(addr);
                self.write_reg(rd as usize, value);
            }
            Instruction::St { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.read_reg(rd as usize);
                self.bus.write_u32_be(addr, value);
            }
            Instruction::Ldi { rd, imm } => {
                self.write_reg(rd as usize, imm);
            }
            Instruction::Add { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = self.read_reg(rs2 as usize);
                self.write_reg(rd as usize, lhs.wrapping_add(rhs));
            }
            Instruction::Addi { rd, rs1, imm } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = (imm as i32) as u32;
                self.write_reg(rd as usize, lhs.wrapping_add(rhs));
            }
            Instruction::Sub { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = self.read_reg(rs2 as usize);
                self.write_reg(rd as usize, lhs.wrapping_sub(rhs));
            }
            Instruction::Slt { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize) as i32;
                let rhs = self.read_reg(rs2 as usize) as i32;
                self.write_reg(rd as usize, u32::from(lhs < rhs));
            }
            Instruction::Beq { rs1, rs2, offset } => {
                if self.read_reg(rs1 as usize) == self.read_reg(rs2 as usize) {
                    self.pc = Self::branch_target(next_pc, offset);
                }
            }
            Instruction::Bne { rs1, rs2, offset } => {
                if self.read_reg(rs1 as usize) != self.read_reg(rs2 as usize) {
                    self.pc = Self::branch_target(next_pc, offset);
                }
            }
            Instruction::Jmp { offset } => {
                self.pc = Self::branch_target(next_pc, offset);
            }
            Instruction::Jal { offset } => {
                self.write_reg(REG_LR, next_pc);
                self.pc = Self::branch_target(next_pc, offset);
            }
            Instruction::Jr { rd } => {
                self.pc = self.read_reg(rd as usize);
            }
            Instruction::Cpuid => {
                self.write_reg(REG_CPUID, CPU_ID);
                self.write_reg(REG_FEATURES, CPU_FEATURES);
            }
            Instruction::Halt => {
                self.running = false;
            }
            Instruction::And { rd, rs1, rs2 } => {
                self.write_reg(
                    rd as usize,
                    self.read_reg(rs1 as usize) & self.read_reg(rs2 as usize),
                );
            }
            Instruction::Or { rd, rs1, rs2 } => {
                self.write_reg(
                    rd as usize,
                    self.read_reg(rs1 as usize) | self.read_reg(rs2 as usize),
                );
            }
            Instruction::Xor { rd, rs1, rs2 } => {
                self.write_reg(
                    rd as usize,
                    self.read_reg(rs1 as usize) ^ self.read_reg(rs2 as usize),
                );
            }
            Instruction::Sll { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                self.write_reg(rd as usize, self.read_reg(rs1 as usize).wrapping_shl(sh));
            }
            Instruction::Srl { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                self.write_reg(rd as usize, self.read_reg(rs1 as usize).wrapping_shr(sh));
            }
            Instruction::Sla { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                self.write_reg(rd as usize, self.read_reg(rs1 as usize).wrapping_shl(sh));
            }
            Instruction::Sra { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                let value = self.read_reg(rs1 as usize) as i32;
                self.write_reg(rd as usize, (value >> sh) as u32);
            }
            Instruction::Ldb { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.bus.read_u8(addr) as u32;
                self.write_reg(rd as usize, value);
            }
            Instruction::Ldh { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let b0 = self.bus.read_u8(addr) as u32;
                let b1 = self.bus.read_u8(addr.wrapping_add(1)) as u32;
                let value = (b0 << 8) | b1;
                self.write_reg(rd as usize, value);
            }
            Instruction::Stb { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.read_reg(rd as usize) as u8;
                self.bus.write_u8(addr, value);
            }
            Instruction::Sth { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.read_reg(rd as usize);
                self.bus.write_u8(addr, (value >> 8) as u8);
                self.bus.write_u8(addr.wrapping_add(1), (value & 0xFF) as u8);
            }
            Instruction::Unknown(raw) => {
                panic!("Illegal instruction at PC=0x{:08X}: 0x{:010X}", next_pc - INSN_SIZE, raw);
            }
        }
    }

    pub fn return_state_text(&self) -> String {
        let mut txt = format!(
            "PC=0x{:08X} LR=0x{:08X} OP=0x{:010X}",
            self.pc,
            self.read_reg(REG_LR),
            self.fetch_u40()
        );
        for i in 0..32 {
            txt.push_str(&format!(" R{}=0x{:08X}", i, self.read_reg(i)));
        }
        txt
    }

    fn step(&mut self) {
        if !self.running {
            return;
        }
        let raw = self.fetch_u40();
        let insn = Self::decode(raw);
        self.execute(insn);
        //println!("\x1b[1;1H{}", self.return_state_text());
    }

    pub fn run(&mut self, max_cycles: usize) {
        for _ in 0..max_cycles {
            if !self.running {
                return;
            }
            self.step();
            self.cycles += 1;
        }
        println!("CPU: Ran for {} cycles (total: {}, last PC: 0x{:08X}, op: 0x{:010X})", max_cycles, self.cycles, self.pc, self.fetch_u40());
    }
}
