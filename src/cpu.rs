// CPU: Central Processing Unit implementation
// This module defines the SRC32 CPU struct, instruction set, and execution logic for the CPT32 emulator.

// Features: SRC32-ALMSI

use crate::bus::Bus;

pub const CPU_CLOCK: u32 = crate::sys::MASTER_CLOCK; // 48MHz
pub const CYCLES_PER_FRAME: u32 = crate::cpu::CPU_CLOCK / crate::sys::FRAME_RATE; // 800,000 cycles/frame
pub const CYCLES_PER_SCANLINE: u32 = CYCLES_PER_FRAME / 240; // 3,333 cycles/scanline

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionMode {
    Normal,
    Short,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedInstruction {
    pub text: String,
    pub size: u8,
    pub mode: InstructionMode,
    pub next_mode: InstructionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Instruction {
    Nop,
    Ld { rd: u8, base: u8, offset: i16 },
    St { rd: u8, base: u8, offset: i16 },
    Add { rd: u8, rs1: u8, rs2: u8 },
    Addi { rd: u8, rs1: u8, imm: i16 },
    Sub { rd: u8, rs1: u8, rs2: u8 },
    Slt { rd: u8, rs1: u8, rs2: u8 },
    Beq { rs1: u8, rs2: u8, offset: i16 },
    Bne { rs1: u8, rs2: u8, offset: i16 },
    Jmp { offset: i16 },
    Jal { offset: i16 },
    Jr { rd: u8 },
    Jmps { offset: i16 },
    Jals { offset: i16 },
    Jrs { rd: u8 },
    Cpuid,
    Halt,
    And { rd: u8, rs1: u8, rs2: u8 },
    Or { rd: u8, rs1: u8, rs2: u8 },
    Xor { rd: u8, rs1: u8, rs2: u8 },
    Sll { rd: u8, rs1: u8, rs2: u8 },
    Srl { rd: u8, rs1: u8, rs2: u8 },
    Sla { rd: u8, rs1: u8, rs2: u8 },
    Sra { rd: u8, rs1: u8, rs2: u8 },
    Sltu { rd: u8, rs1: u8, rs2: u8 },
    Mul { rd: u8, rs1: u8, rs2: u8 },
    Div { rd: u8, rs1: u8, rs2: u8 },
    Mod { rd: u8, rs1: u8, rs2: u8 },
    Mulh { rd: u8, rs1: u8, rs2: u8 },
    Divu { rd: u8, rs1: u8, rs2: u8 },
    Ldb { rd: u8, base: u8, offset: i16 },
    Ldh { rd: u8, base: u8, offset: i16 },
    Stb { rd: u8, base: u8, offset: i16 },
    Sth { rd: u8, base: u8, offset: i16 },
    Ldil { rd: u8, imm: u16 },
    Ldih { rd: u8, imm: u16 },
    Unknown(u32),
    Iret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortInstruction {
    Mov { rd: u8, rs1: u8 },
    Add { rd: u8, rs1: u8, rs2: u8 },
    Addi { rd: u8, imm: i8 },
    Ld { rd: u8, rs1: u8 },
    St { rd: u8, rs1: u8 },
    Bz { rd: u8, offset: i8 },
    Bnz { rd: u8, offset: i8 },
    Jr { rd: u8 },
    Jal { offset: i16 },
    Ldi { rd: u8, imm: u8 },
    Ret,
    Unknown(u16),
}

const REG_ZERO: usize = 0;
const REG_CPUID: usize = 1;
const REG_FEATURES: usize = 2;
const REG_LR: usize = 31;

const EXT_BASE: u32 = 0x01; // Base extension: includes basic arithmetic and logic instructions (ADD, SUB, AND, OR, etc.)
const EXT_A: u32 = 0x02; // Extension A (Arithmetic): adds more arithmetic and logic instructions
const EXT_L: u32 = 0x04; // Extension L (Load/Store): adds byte/halfword load/store instructions
const EXT_M: u32 = 0x08; // Extension M (Mul/Div): adds multiplication and division instructions
const EXT_S: u32 = 0x10; // Extension S (Short Mode)
const EXT_I: u32 = 0x20; // Extension I (Interrupts)
const CPU_FEATURES: u32 = EXT_BASE | EXT_A | EXT_L | EXT_M | EXT_S | EXT_I; // CPUID features bitfield
const CPU_ID: u32 = 0x5352_4332; // "SRC2" style tag
pub const IRQ_VECTOR_BASE: u32 = 0xFFFF_0080;

const INSN_SIZE: u32 = 4;
const SHORT_INSN_SIZE: u32 = 2;

pub struct Cpu {
    reg: [u32; 32],
    pc: u32,
    running: bool,
    bus: Bus,
    cycles: u128,
    instr_mode: InstructionMode,
    epc: u32,
    cause: u8,
    irq_enable: bool,
    irq_pending: bool,
    irq_pending_number: u8,
    irq_line: bool,
}

impl Cpu {
    pub fn new(bus: Bus) -> Self {
        Self {
            reg: [0; 32],
            pc: 0,
            running: true,
            bus,
            cycles: 0,
            instr_mode: InstructionMode::Normal,
            epc: 0,
            cause: 0,
            irq_enable: true,
            irq_pending: false,
            irq_pending_number: 0,
            irq_line: false,
        }
    }

    pub fn reset(&mut self, pc: u32) {
        self.reg = [0; 32];
        self.pc = pc;
        self.running = true;
        self.instr_mode = InstructionMode::Normal;
        self.epc = 0;
        self.cause = 0;
        self.irq_enable = true;
        self.irq_pending = false;
        self.irq_pending_number = 0;
        self.irq_line = false;
    }

    pub fn load_program(&mut self, base: u32, image: &[u8]) {
        for (idx, &byte) in image.iter().enumerate() {
            self.bus.write_u8(base.wrapping_add(idx as u32), byte);
        }
    }

    pub fn pc(&self) -> u32 {
        self.pc
    }

    pub fn set_pc(&mut self, pc: u32) {
        self.pc = pc;
    }

    pub fn instruction_mode(&self) -> InstructionMode {
        self.instr_mode
    }

    pub fn set_instruction_mode(&mut self, mode: InstructionMode) {
        self.instr_mode = mode;
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn cycles(&self) -> u128 {
        self.cycles
    }

    pub fn epc(&self) -> u32 {
        self.epc
    }

    pub fn irq_cause(&self) -> u8 {
        self.cause
    }

    pub fn irq_enabled(&self) -> bool {
        self.irq_enable
    }

    /// Drive the CPU's external IRQ input. A rising edge latches one request.
    /// IRQC owns source arbitration; the CPU only receives the selected number.
    pub fn set_irq_input(&mut self, level: bool, number: u8) {
        if level && !self.irq_line {
            self.irq_pending = true;
            self.irq_pending_number = number & 0x0F;
        }
        self.irq_line = level;
    }

    pub fn read_mem_u8(&mut self, addr: u32) -> u8 {
        self.bus.read_u8(addr)
    }

    pub fn read_mem_u32_be(&mut self, addr: u32) -> u32 {
        self.bus.read_u32_be(addr)
    }

    pub fn write_mem_u8(&mut self, addr: u32, value: u8) {
        self.bus.write_u8(addr, value);
    }

    pub fn write_mem_u32_be(&mut self, addr: u32, value: u32) {
        self.bus.write_u32_be(addr, value);
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

    pub fn write_reg(&mut self, reg: usize, value: u32) -> Result<String, String> {
        if reg >= self.reg.len() {
            return Err(format!("Invalid register index: {reg}"));
        }
        if reg == REG_ZERO {
            return Ok("Warning: Writing to R0 has no effect".into());
        }
        self.reg[reg] = value;
        Ok("".into())
    }

    fn fetch_u32(&mut self) -> u32 {
        self.bus.read_u32_be(self.pc)
    }

    fn fetch_u32_at(&mut self, addr: u32) -> u32 {
        self.bus.read_u32_be(addr)
    }

    fn fetch_u16_at(&mut self, addr: u32) -> u16 {
        let hi = self.bus.read_u8(addr) as u16;
        let lo = self.bus.read_u8(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    fn decode(raw: u32) -> Instruction {
        let op = ((raw >> 26) & 0x3F) as u8;
        let rd = ((raw >> 21) & 0x1F) as u8;
        let rs1 = ((raw >> 16) & 0x1F) as u8;
        let rs2 = ((raw >> 11) & 0x1F) as u8;
        let imm16 = (raw & 0xFFFF) as i16;
        let imm_u16 = (raw & 0xFFFF) as u16;

        match op {
            0x00 => Instruction::Nop,
            0x01 => Instruction::Ld {
                rd,
                base: rs1,
                offset: imm16,
            },
            0x02 => Instruction::St {
                rd,
                base: rs1,
                offset: imm16,
            },
            0x03 => Instruction::Add { rd, rs1, rs2 },
            0x04 => Instruction::Addi {
                rd,
                rs1,
                imm: imm16,
            },
            0x05 => Instruction::Sub { rd, rs1, rs2 },
            0x06 => Instruction::Slt { rd, rs1, rs2 },
            0x07 => Instruction::Beq {
                rs1,
                rs2: rd,
                offset: imm16,
            },
            0x08 => Instruction::Bne {
                rs1,
                rs2: rd,
                offset: imm16,
            },
            0x09 => Instruction::Jmp { offset: imm16 },
            0x0A => Instruction::Jal { offset: imm16 },
            0x0B => Instruction::Jr { rd },
            0x0C => Instruction::And { rd, rs1, rs2 },
            0x0D => Instruction::Or { rd, rs1, rs2 },
            0x0E => Instruction::Xor { rd, rs1, rs2 },
            0x0F => Instruction::Sll { rd, rs1, rs2 },
            0x10 => Instruction::Srl { rd, rs1, rs2 },
            0x11 => Instruction::Sla { rd, rs1, rs2 },
            0x12 => Instruction::Sra { rd, rs1, rs2 },
            0x13 => Instruction::Ldb {
                rd,
                base: rs1,
                offset: imm16,
            },
            0x14 => Instruction::Ldh {
                rd,
                base: rs1,
                offset: imm16,
            },
            0x15 => Instruction::Stb {
                rd,
                base: rs1,
                offset: imm16,
            },
            0x16 => Instruction::Sth {
                rd,
                base: rs1,
                offset: imm16,
            },
            0x17 => Instruction::Sltu { rd, rs1, rs2 },
            0x18 => Instruction::Mul { rd, rs1, rs2 },
            0x19 => Instruction::Div { rd, rs1, rs2 },
            0x1A => Instruction::Mod { rd, rs1, rs2 },
            0x1B => Instruction::Mulh { rd, rs1, rs2 },
            0x1C => Instruction::Divu { rd, rs1, rs2 },
            0x1D => Instruction::Jmps { offset: imm16 },
            0x1E => Instruction::Jals { offset: imm16 },
            0x1F => Instruction::Jrs { rd },
            0x3C => Instruction::Ldil { rd, imm: imm_u16 },
            0x3D => Instruction::Ldih { rd, imm: imm_u16 },
            0x3E => Instruction::Cpuid,
            0x3F => Instruction::Halt,
            0x20 => Instruction::Iret,
            _ => Instruction::Unknown(raw),
        }
    }

    fn decode_short(raw: u16) -> ShortInstruction {
        let op = ((raw >> 12) & 0x0F) as u8;
        let rd = ((raw >> 8) & 0x0F) as u8;
        let rs1 = ((raw >> 4) & 0x0F) as u8;
        let rs2 = (raw & 0x0F) as u8;
        let imm8 = (raw & 0x00FF) as u8;
        let imm12 = ((raw & 0x0FFF) as i16) << 4 >> 4;

        match op {
            0x0 => ShortInstruction::Mov { rd, rs1 },
            0x1 => ShortInstruction::Add { rd, rs1, rs2 },
            0x2 => ShortInstruction::Addi {
                rd,
                imm: imm8 as i8,
            },
            0x3 => ShortInstruction::Ld { rd, rs1 },
            0x4 => ShortInstruction::St { rd, rs1 },
            0x5 => ShortInstruction::Bz {
                rd,
                offset: imm8 as i8,
            },
            0x6 => ShortInstruction::Bnz {
                rd,
                offset: imm8 as i8,
            },
            0x7 => ShortInstruction::Jr { rd },
            0x8 => ShortInstruction::Jal { offset: imm12 },
            0x9 => ShortInstruction::Ldi { rd, imm: imm8 },
            0xF => ShortInstruction::Ret,
            _ => ShortInstruction::Unknown(raw),
        }
    }

    fn format_instruction(insn: Instruction) -> String {
        match insn {
            Instruction::Nop => "NOP".to_string(),
            Instruction::Ld { rd, base, offset } => {
                format!("LD R{}, [R{} + {}]", rd, base, offset)
            }
            Instruction::St { rd, base, offset } => {
                format!("ST [R{} + {}], R{}", base, offset, rd)
            }
            Instruction::Add { rd, rs1, rs2 } => format!("ADD R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Addi { rd, rs1, imm } => format!("ADDI R{}, R{}, {}", rd, rs1, imm),
            Instruction::Sub { rd, rs1, rs2 } => format!("SUB R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Slt { rd, rs1, rs2 } => format!("SLT R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Beq { rs1, rs2, offset } => {
                format!("BEQ R{}, R{}, {}", rs1, rs2, offset)
            }
            Instruction::Bne { rs1, rs2, offset } => {
                format!("BNE R{}, R{}, {}", rs1, rs2, offset)
            }
            Instruction::Jmp { offset } => format!("JMP {}", offset),
            Instruction::Jal { offset } => format!("JAL {}", offset),
            Instruction::Jr { rd } => format!("JR R{}", rd),
            Instruction::Jmps { offset } => format!("JMPS {}", offset),
            Instruction::Jals { offset } => format!("JALS {}", offset),
            Instruction::Jrs { rd } => format!("JRS R{}", rd),
            Instruction::Cpuid => "CPUID".to_string(),
            Instruction::Halt => "HALT".to_string(),
            Instruction::Iret => "IRET".to_string(),
            Instruction::And { rd, rs1, rs2 } => format!("AND R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Or { rd, rs1, rs2 } => format!("OR R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Xor { rd, rs1, rs2 } => format!("XOR R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Sll { rd, rs1, rs2 } => format!("SLL R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Srl { rd, rs1, rs2 } => format!("SRL R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Sla { rd, rs1, rs2 } => format!("SLA R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Sra { rd, rs1, rs2 } => format!("SRA R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Sltu { rd, rs1, rs2 } => format!("SLTU R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Mul { rd, rs1, rs2 } => format!("MUL R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Div { rd, rs1, rs2 } => format!("DIV R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Mod { rd, rs1, rs2 } => format!("MOD R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Mulh { rd, rs1, rs2 } => format!("MULH R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Divu { rd, rs1, rs2 } => format!("DIVU R{}, R{}, R{}", rd, rs1, rs2),
            Instruction::Ldb { rd, base, offset } => {
                format!("LDB R{}, [R{} + {}]", rd, base, offset)
            }
            Instruction::Ldh { rd, base, offset } => {
                format!("LDH R{}, [R{} + {}]", rd, base, offset)
            }
            Instruction::Stb { rd, base, offset } => {
                format!("STB [R{} + {}], R{}", base, offset, rd)
            }
            Instruction::Sth { rd, base, offset } => {
                format!("STH [R{} + {}], R{}", base, offset, rd)
            }
            Instruction::Ldil { rd, imm } => format!("LDIL R{}, 0x{:04X}", rd, imm),
            Instruction::Ldih { rd, imm } => format!("LDIH R{}, 0x{:04X}", rd, imm),
            Instruction::Unknown(raw) => format!(".dword 0x{:08X}", raw),
        }
    }

    fn format_short_instruction(insn: ShortInstruction) -> String {
        match insn {
            ShortInstruction::Mov { rd, rs1 } => format!("S.MOV R{}, R{}", rd, rs1),
            ShortInstruction::Add { rd, rs1, rs2 } => {
                format!("S.ADD R{}, R{}, R{}", rd, rs1, rs2)
            }
            ShortInstruction::Addi { rd, imm } => format!("S.ADDI R{}, {}", rd, imm),
            ShortInstruction::Ld { rd, rs1 } => format!("S.LD R{}, R{}", rd, rs1),
            ShortInstruction::St { rd, rs1 } => format!("S.ST R{}, R{}", rd, rs1),
            ShortInstruction::Bz { rd, offset } => format!("S.BZ R{}, {}", rd, offset),
            ShortInstruction::Bnz { rd, offset } => format!("S.BNZ R{}, {}", rd, offset),
            ShortInstruction::Jr { rd } => format!("S.JR R{}", rd),
            ShortInstruction::Jal { offset } => format!("S.JAL {}", offset),
            ShortInstruction::Ldi { rd, imm } => format!("S.LDI R{}, 0x{:02X}", rd, imm),
            ShortInstruction::Ret => "S.RET".to_string(),
            ShortInstruction::Unknown(raw) => format!(".word 0x{:04X}", raw),
        }
    }

    pub fn decode_at(&mut self, addr: u32, mode: InstructionMode) -> DecodedInstruction {
        match mode {
            InstructionMode::Normal => {
                let raw = self.fetch_u32_at(addr);
                let insn = Self::decode(raw);
                let next_mode = match insn {
                    Instruction::Jmps { .. } | Instruction::Jals { .. } | Instruction::Jrs { .. } => {
                        InstructionMode::Short
                    }
                    _ => InstructionMode::Normal,
                };
                DecodedInstruction {
                    text: Self::format_instruction(insn),
                    size: INSN_SIZE as u8,
                    mode,
                    next_mode,
                }
            }
            InstructionMode::Short => {
                let raw = self.fetch_u16_at(addr);
                let insn = Self::decode_short(raw);
                let next_mode = match insn {
                    ShortInstruction::Ret => InstructionMode::Normal,
                    _ => InstructionMode::Short,
                };
                DecodedInstruction {
                    text: Self::format_short_instruction(insn),
                    size: SHORT_INSN_SIZE as u8,
                    mode,
                    next_mode,
                }
            }
        }
    }

    pub fn decode_current(&mut self) -> DecodedInstruction {
        self.decode_at(self.pc, self.instr_mode)
    }

    pub fn disassemble_at(&mut self, addr: u32) -> String {
        self.decode_at(addr, InstructionMode::Normal).text
    }

    pub fn read_u32(&mut self, addr: u32) -> u32 {
        self.fetch_u32_at(addr)
    }

    pub fn read_u16_be(&mut self, addr: u32) -> u16 {
        self.fetch_u16_at(addr)
    }

    pub fn read_u40(&mut self, addr: u32) -> u32 {
        self.fetch_u32_at(addr)
    }

    fn add_signed(base: u32, offset: i16) -> u32 {
        base.wrapping_add((offset as i32) as u32)
    }

    fn branch_target(next_pc: u32, offset: i16) -> u32 {
        next_pc.wrapping_add((offset as i32) as u32)
    }

    fn short_reg_to_gpr(sr: u8) -> usize {
        if sr < 15 {
            sr as usize
        } else {
            REG_LR
        }
    }

    fn read_short_reg(&self, sr: u8) -> u32 {
        self.read_reg(Self::short_reg_to_gpr(sr))
    }

    fn write_short_reg(&mut self, sr: u8, value: u32) {
        let _ = self.write_reg(Self::short_reg_to_gpr(sr), value);
    }

    fn execute_normal(&mut self, insn: Instruction) {
        let next_pc = self.pc.wrapping_add(INSN_SIZE);
        self.pc = next_pc;

        match insn {
            Instruction::Nop => {}
            Instruction::Ld { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.bus.read_u32_be(addr);
                let _ = self.write_reg(rd as usize, value);
            }
            Instruction::St { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.read_reg(rd as usize);
                self.bus.write_u32_be(addr, value);
            }
            Instruction::Ldil { rd, imm } => {
                let current = self.read_reg(rd as usize);
                let value = (current & 0xFFFF_0000) | u32::from(imm);
                let _ = self.write_reg(rd as usize, value);
            }
            Instruction::Ldih { rd, imm } => {
                let current = self.read_reg(rd as usize);
                let value = (current & 0x0000_FFFF) | (u32::from(imm) << 16);
                let _ = self.write_reg(rd as usize, value);
            }
            Instruction::Add { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = self.read_reg(rs2 as usize);
                let _ = self.write_reg(rd as usize, lhs.wrapping_add(rhs));
            }
            Instruction::Addi { rd, rs1, imm } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = (imm as i32) as u32;
                let _ = self.write_reg(rd as usize, lhs.wrapping_add(rhs));
            }
            Instruction::Sub { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = self.read_reg(rs2 as usize);
                let _ = self.write_reg(rd as usize, lhs.wrapping_sub(rhs));
            }
            Instruction::Slt { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize) as i32;
                let rhs = self.read_reg(rs2 as usize) as i32;
                let _ = self.write_reg(rd as usize, u32::from(lhs < rhs));
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
                let _ = self.write_reg(REG_LR, next_pc);
                self.pc = Self::branch_target(next_pc, offset);
            }
            Instruction::Jr { rd } => {
                self.pc = self.read_reg(rd as usize);
            }
            Instruction::Jmps { offset } => {
                self.pc = Self::branch_target(next_pc, offset);
                self.instr_mode = InstructionMode::Short;
            }
            Instruction::Jals { offset } => {
                let _ = self.write_reg(REG_LR, next_pc);
                self.pc = Self::branch_target(next_pc, offset);
                self.instr_mode = InstructionMode::Short;
            }
            Instruction::Jrs { rd } => {
                self.pc = self.read_reg(rd as usize);
                self.instr_mode = InstructionMode::Short;
            }
            Instruction::Cpuid => {
                let _ = self.write_reg(REG_CPUID, CPU_ID);
                let _ = self.write_reg(REG_FEATURES, CPU_FEATURES);
            }
            Instruction::Halt => {
                self.running = false;
            }
            Instruction::Iret => {
                self.pc = self.epc;
                self.instr_mode = InstructionMode::Normal;
                self.irq_enable = true;
            }
            Instruction::And { rd, rs1, rs2 } => {
                let _ = self.write_reg(
                    rd as usize,
                    self.read_reg(rs1 as usize) & self.read_reg(rs2 as usize),
                );
            }
            Instruction::Or { rd, rs1, rs2 } => {
                let _ = self.write_reg(
                    rd as usize,
                    self.read_reg(rs1 as usize) | self.read_reg(rs2 as usize),
                );
            }
            Instruction::Xor { rd, rs1, rs2 } => {
                let _ = self.write_reg(
                    rd as usize,
                    self.read_reg(rs1 as usize) ^ self.read_reg(rs2 as usize),
                );
            }
            Instruction::Sll { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                let _ = self.write_reg(rd as usize, self.read_reg(rs1 as usize).wrapping_shl(sh));
            }
            Instruction::Srl { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                let _ = self.write_reg(rd as usize, self.read_reg(rs1 as usize).wrapping_shr(sh));
            }
            Instruction::Sla { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                let _ = self.write_reg(rd as usize, self.read_reg(rs1 as usize).wrapping_shl(sh));
            }
            Instruction::Sra { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                let value = self.read_reg(rs1 as usize) as i32;
                let _ = self.write_reg(rd as usize, (value >> sh) as u32);
            }
            Instruction::Sltu { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = self.read_reg(rs2 as usize);
                let _ = self.write_reg(rd as usize, u32::from(lhs < rhs));
            }
            Instruction::Mul { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize) as u64;
                let rhs = self.read_reg(rs2 as usize) as u64;
                let _ = self.write_reg(rd as usize, lhs.wrapping_mul(rhs) as u32);
            }
            Instruction::Div { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize) as i32;
                let rhs = self.read_reg(rs2 as usize) as i32;
                let result = if rhs == 0 {
                    0
                } else if lhs == i32::MIN && rhs == -1 {
                    i32::MIN
                } else {
                    lhs / rhs
                };
                let _ = self.write_reg(rd as usize, result as u32);
            }
            Instruction::Mod { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize) as i32;
                let rhs = self.read_reg(rs2 as usize) as i32;
                let result = if rhs == 0 {
                    0
                } else if lhs == i32::MIN && rhs == -1 {
                    0
                } else {
                    lhs % rhs
                };
                let _ = self.write_reg(rd as usize, result as u32);
            }
            Instruction::Mulh { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize) as i32 as i64;
                let rhs = self.read_reg(rs2 as usize) as i32 as i64;
                let _ = self.write_reg(rd as usize, ((lhs.wrapping_mul(rhs)) >> 32) as u32);
            }
            Instruction::Divu { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = self.read_reg(rs2 as usize);
                let result = if rhs == 0 { 0 } else { lhs / rhs };
                let _ = self.write_reg(rd as usize, result);
            }
            Instruction::Ldb { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.bus.read_u8(addr) as u32;
                let _ = self.write_reg(rd as usize, value);
            }
            Instruction::Ldh { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let b0 = self.bus.read_u8(addr) as u32;
                let b1 = self.bus.read_u8(addr.wrapping_add(1)) as u32;
                let value = (b0 << 8) | b1;
                let _ = self.write_reg(rd as usize, value);
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
                panic!("Illegal instruction at PC=0x{:08X}: 0x{:08X}", next_pc - INSN_SIZE, raw);
            }
        }
    }

    fn execute_short(&mut self, insn: ShortInstruction) {
        let next_pc = self.pc.wrapping_add(SHORT_INSN_SIZE);
        self.pc = next_pc;

        match insn {
            ShortInstruction::Mov { rd, rs1 } => {
                self.write_short_reg(rd, self.read_short_reg(rs1));
            }
            ShortInstruction::Add { rd, rs1, rs2 } => {
                let lhs = self.read_short_reg(rs1);
                let rhs = self.read_short_reg(rs2);
                self.write_short_reg(rd, lhs.wrapping_add(rhs));
            }
            ShortInstruction::Addi { rd, imm } => {
                let lhs = self.read_short_reg(rd);
                let rhs = (imm as i32) as u32;
                self.write_short_reg(rd, lhs.wrapping_add(rhs));
            }
            ShortInstruction::Ld { rd, rs1 } => {
                let addr = self.read_short_reg(rs1);
                let value = self.bus.read_u32_be(addr);
                self.write_short_reg(rd, value);
            }
            ShortInstruction::St { rd, rs1 } => {
                let addr = self.read_short_reg(rd);
                let value = self.read_short_reg(rs1);
                self.bus.write_u32_be(addr, value);
            }
            ShortInstruction::Bz { rd, offset } => {
                if self.read_short_reg(rd) == 0 {
                    self.pc = Self::branch_target(next_pc, offset as i16);
                }
            }
            ShortInstruction::Bnz { rd, offset } => {
                if self.read_short_reg(rd) != 0 {
                    self.pc = Self::branch_target(next_pc, offset as i16);
                }
            }
            ShortInstruction::Jr { rd } => {
                self.pc = self.read_short_reg(rd);
            }
            ShortInstruction::Jal { offset } => {
                let _ = self.write_reg(REG_LR, next_pc);
                self.pc = Self::branch_target(next_pc, offset);
            }
            ShortInstruction::Ldi { rd, imm } => {
                self.write_short_reg(rd, u32::from(imm));
            }
            ShortInstruction::Ret => {
                self.instr_mode = InstructionMode::Normal;
            }
            ShortInstruction::Unknown(raw) => {
                panic!(
                    "Illegal short instruction at PC=0x{:08X}: 0x{:04X}",
                    next_pc - SHORT_INSN_SIZE,
                    raw
                );
            }
        }
    }

    pub fn return_state_text(&mut self) -> String {
        let pc = self.pc;
        let decoded = self.decode_current();
        let mut txt = format!(
            "PC=0x{:08X} MODE={:?} OP={}\n",
            pc,
            self.instr_mode,
            decoded.text
        );
        for i in 0..32 {
            txt.push_str(&format!(" R{:<2}=0x{:08X}", i, self.read_reg(i)));
            if i % 4 == 3 {
                txt.push('\n');
            }
        }
        txt
    }

    fn step(&mut self) {
        if !self.running {
            return;
        }

        self.cycles += 1;
        match self.instr_mode {
            InstructionMode::Normal => {
                let raw = self.fetch_u32();
                let insn = Self::decode(raw);
                self.cycles += 1; // decode
                self.execute_normal(insn);
            }
            InstructionMode::Short => {
                let raw = self.fetch_u16_at(self.pc);
                let insn = Self::decode_short(raw);
                self.cycles += 1; // decode
                self.execute_short(insn);
            }
        }
        self.cycles += 1; // execute

        // Interrupts are sampled only after the complete instruction has
        // committed. This also makes the saved EPC the next instruction.
        if self.running && self.irq_enable && self.irq_pending {
            self.epc = self.pc;
            self.cause = self.irq_pending_number;
            self.irq_pending = false;
            self.irq_enable = false;
            self.instr_mode = InstructionMode::Normal;
            self.pc = IRQ_VECTOR_BASE.wrapping_add(u32::from(self.cause) * 4);
        }
    }

    pub fn step_once(&mut self) -> bool {
        if !self.running {
            return false;
        }
        self.step();
        true
    }

    pub fn run(&mut self, max_cycles: usize) {
        let start_cycles = self.cycles;
        while (self.cycles < start_cycles + max_cycles as u128) && self.running {
            self.step();
        }
    }
}
