// Pipelined SRC32 CPU (Extension p, revision 0).
//
// 5-stage in-order pipeline: IF / ID / EX / MEM / WB.
// ISA is unchanged from `crate::cpu::Cpu`; only the micro-architecture differs.
// The scalar `Cpu` remains the reference model: same programs must produce the
// same architectural state (registers, PC, mode, memory).

use crate::bus::{Bus, BusAccessSource, BusRegion};
use crate::devices::cpu::{BUS_ERROR_VECTOR, CpuRegisterBlock};
use crate::devices::irqc::irqc::IrqController;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use crate::cpu::{
    CpuProfile, CpuProfileStage, DecodedInstruction, IRQ_VECTOR_BASE, InstructionMode,
};

const REG_ZERO: usize = 0;
const REG_CPUID: usize = 1;
const REG_FEATURES: usize = 2;
const REG_LR: usize = 31;

const EXT_BASE: u32 = 0x01;
const EXT_A: u32 = 0x02;
const EXT_L: u32 = 0x04;
const EXT_M: u32 = 0x08;
const EXT_S: u32 = 0x10;
const EXT_I: u32 = 0x20;
const EXT_P_SMALL: u32 = 0x80; // Extension p (lowercase): pipelining (micro-arch only, no ISA change)
const CPU_FEATURES: u32 = EXT_BASE | EXT_A | EXT_L | EXT_M | EXT_S | EXT_I | EXT_P_SMALL;
const CPU_ID: u32 = 0x5352_4332;

const INSN_SIZE: u32 = 4;
const SHORT_INSN_SIZE: u32 = 2;
const MUL_LATENCY: u32 = 3;
const DIV_LATENCY: u32 = 16;

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
    Jalr { rd: u8 },
    Jalrs { rd: u8 },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawInstr {
    Normal(u32),
    Short(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decoded {
    Normal(Instruction),
    Short(ShortInstruction),
}

#[derive(Debug, Clone, Copy)]
struct IfId {
    valid: bool,
    pc: u32,
    mode: InstructionMode,
    raw: RawInstr,
}

#[derive(Debug, Clone, Copy)]
struct IdEx {
    valid: bool,
    pc: u32,
    mode: InstructionMode,
    decoded: Decoded,
    // GPR numbers (already short-mapped). [src, valid-count]
    srcs: [usize; 3],
    n_srcs: u8,
    dest: Option<usize>,
    // Register values sampled at ID (before forwarding correction at EX).
    op_a: u32,
    op_b: u32,
    is_load: bool,
    is_store: bool,
}

#[derive(Debug, Clone, Copy)]
struct ExMem {
    valid: bool,
    pc: u32,
    mode: InstructionMode,
    decoded: Decoded,
    dest: Option<usize>,
    writeback: bool,
    is_load: bool,
    is_store: bool,
    alu_result: u32,
    store_value: u32,
    mem_addr: u32,
    mem_size: u8, // 0 = no mem op, 1/2/4 = access width
    is_mem_read: bool,
    branch_taken: bool,
    branch_target: u32,
    branch_new_mode: Option<InstructionMode>,
    halt: bool,
    iret: bool,
    cpuid: bool,
    link_value: Option<(usize, u32)>,
}

#[derive(Debug, Clone, Copy)]
struct MemWb {
    valid: bool,
    pc: u32,
    mode: InstructionMode,
    dest: Option<usize>,
    writeback: bool,
    value: u32,
    halt: bool,
    iret: bool,
    cpuid: bool,
    link_value: Option<(usize, u32)>,
    branch_taken: bool,
    branch_target: u32,
    branch_new_mode: Option<InstructionMode>,
}

pub struct PipelineCpu {
    reg: [u32; 32],
    fetch_pc: u32,
    fetch_mode: InstructionMode,
    retire_pc: u32,
    retire_mode: InstructionMode,
    running: bool,
    bus: Bus,
    cycles: u128,
    running_cycles: u128,
    instructions_executed: u128,
    stall_cycles: u128,
    flush_events: u128,
    epc: u32,
    irq_return_mode: InstructionMode,
    cause: u8,
    irq_enable: bool,
    irq_pending: bool,
    irq_pending_number: u8,
    irq_line: bool,
    external_irq_line: bool,
    external_irq_number: u8,
    irqc_line: bool,
    irqc_number: u8,
    register_block: Rc<RefCell<CpuRegisterBlock>>,
    irq_controller: Rc<RefCell<IrqController>>,
    if_id: IfId,
    id_ex: IdEx,
    ex_mem: ExMem,
    mem_wb: MemWb,
    ex_cycles_left: u32,
    ex_armed: bool,
    profile: Option<CpuProfile>,
}

fn invalid_id_ex() -> IdEx {
    IdEx {
        valid: false,
        pc: 0,
        mode: InstructionMode::Normal,
        decoded: Decoded::Normal(Instruction::Nop),
        srcs: [0; 3],
        n_srcs: 0,
        dest: None,
        op_a: 0,
        op_b: 0,
        is_load: false,
        is_store: false,
    }
}

fn invalid_ex_mem() -> ExMem {
    ExMem {
        valid: false,
        pc: 0,
        mode: InstructionMode::Normal,
        decoded: Decoded::Normal(Instruction::Nop),
        dest: None,
        writeback: false,
        is_load: false,
        is_store: false,
        alu_result: 0,
        store_value: 0,
        mem_addr: 0,
        mem_size: 0,
        is_mem_read: false,
        branch_taken: false,
        branch_target: 0,
        branch_new_mode: None,
        halt: false,
        iret: false,
        cpuid: false,
        link_value: None,
    }
}

fn invalid_mem_wb() -> MemWb {
    MemWb {
        valid: false,
        pc: 0,
        mode: InstructionMode::Normal,
        dest: None,
        writeback: false,
        value: 0,
        halt: false,
        iret: false,
        cpuid: false,
        link_value: None,
        branch_taken: false,
        branch_target: 0,
        branch_new_mode: None,
    }
}

impl PipelineCpu {
    pub fn new(bus: Bus) -> Self {
        let mut bus = bus;
        let register_block = crate::devices::cpu::connect_cpu_registers(&mut bus);
        let irq_controller = crate::devices::irqc::irqc::connect_irqc(&mut bus);
        bus.attach_irq_controller(Rc::clone(&irq_controller));
        let mut cpu = Self {
            reg: [0; 32],
            fetch_pc: 0,
            fetch_mode: InstructionMode::Normal,
            retire_pc: 0,
            retire_mode: InstructionMode::Normal,
            running: true,
            bus,
            cycles: 0,
            running_cycles: 0,
            instructions_executed: 0,
            stall_cycles: 0,
            flush_events: 0,
            epc: 0,
            irq_return_mode: InstructionMode::Normal,
            cause: 0,
            irq_enable: true,
            irq_pending: false,
            irq_pending_number: 0,
            irq_line: false,
            external_irq_line: false,
            external_irq_number: 0,
            irqc_line: false,
            irqc_number: 0,
            register_block,
            irq_controller,
            if_id: IfId {
                valid: false,
                pc: 0,
                mode: InstructionMode::Normal,
                raw: RawInstr::Normal(0),
            },
            id_ex: invalid_id_ex(),
            ex_mem: invalid_ex_mem(),
            mem_wb: invalid_mem_wb(),
            ex_cycles_left: 0,
            ex_armed: false,
            profile: None,
        };
        cpu.sync_register_block();
        cpu
    }

    pub fn reset(&mut self, pc: u32) {
        self.reg = [0; 32];
        self.fetch_pc = pc;
        self.fetch_mode = InstructionMode::Normal;
        self.running = true;
        self.cycles = 0;
        self.running_cycles = 0;
        self.instructions_executed = 0;
        self.epc = 0;
        self.irq_return_mode = InstructionMode::Normal;
        self.cause = 0;
        self.irq_enable = true;
        self.irq_pending = false;
        self.irq_pending_number = 0;
        self.irq_line = false;
        self.external_irq_line = false;
        self.external_irq_number = 0;
        self.irqc_line = false;
        self.irqc_number = 0;
        self.if_id.valid = false;
        self.id_ex = invalid_id_ex();
        self.ex_mem = invalid_ex_mem();
        self.mem_wb = invalid_mem_wb();
        self.ex_cycles_left = 0;
        self.ex_armed = false;
        self.retire_pc = pc;
        self.retire_mode = InstructionMode::Normal;
        self.bus.reset_expansion_cards();
        self.irq_controller.borrow_mut().reset();
        self.sync_register_block();
    }

    fn sync_register_block(&mut self) {
        let mut block = self.register_block.borrow_mut();
        block.regs = self.reg;
        block.regs[0] = 0;
        block.pc = self.retire_pc;
        block.epc = self.epc;
        block.cause = u32::from(self.cause);
        block.status = u32::from(self.irq_enable);
        block.instr_mode = match self.fetch_mode {
            InstructionMode::Normal => 0,
            InstructionMode::Short => 1,
        };
    }

    fn sync_from_register_block(&mut self) {
        let block = self.register_block.borrow();
        let irq_enable = block.status & 1 != 0;
        if self.irq_enable != irq_enable && crate::devices::pec::pec::pec_trace_enabled() {
            tracing::info!(
                target: "pec_debug",
                "[PEC-DBG] CPU status resync cpu=pipeline old_irq_enabled={} block_irq_enabled={} block_pc=0x{:08X} block_epc=0x{:08X} cause={}",
                self.irq_enable,
                irq_enable,
                block.pc,
                block.epc,
                block.cause
            );
        }
        self.reg = block.regs;
        self.reg[REG_ZERO] = 0;
        self.fetch_pc = block.pc;
        self.retire_pc = block.pc;
        self.epc = block.epc;
        self.cause = block.cause as u8;
        self.irq_enable = irq_enable;
        self.fetch_mode = if block.instr_mode & 1 != 0 {
            InstructionMode::Short
        } else {
            InstructionMode::Normal
        };
        self.retire_mode = self.fetch_mode;
    }

    fn pipeline_empty(&self) -> bool {
        !self.if_id.valid && !self.id_ex.valid && !self.ex_mem.valid && !self.mem_wb.valid
    }

    pub fn load_program(&mut self, base: u32, image: &[u8]) {
        for (idx, &byte) in image.iter().enumerate() {
            self.bus.write_u8(base.wrapping_add(idx as u32), byte);
        }
    }

    /// Architectural PC: address after the last committed instruction
    /// (prefetch may run ahead in fetch_pc while the pipeline is full).
    pub fn pc(&self) -> u32 {
        self.retire_pc
    }

    pub fn set_pc(&mut self, pc: u32) {
        self.fetch_pc = pc;
        self.retire_pc = pc;
        self.if_id.valid = false;
        self.id_ex = invalid_id_ex();
        self.ex_mem = invalid_ex_mem();
        self.mem_wb = invalid_mem_wb();
        self.ex_cycles_left = 0;
        self.ex_armed = false;
        self.sync_register_block();
    }

    pub fn instruction_mode(&self) -> InstructionMode {
        self.fetch_mode
    }

    pub fn set_instruction_mode(&mut self, mode: InstructionMode) {
        self.fetch_mode = mode;
        self.sync_register_block();
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn cycles(&self) -> u128 {
        self.cycles
    }

    pub fn running_cycles(&self) -> u128 {
        self.running_cycles
    }

    pub fn instructions_executed(&self) -> u128 {
        self.instructions_executed
    }

    pub fn enable_profiling(&mut self, enabled: bool) {
        self.profile = enabled.then(CpuProfile::default);
    }

    pub fn take_profile(&mut self) -> Option<CpuProfile> {
        self.profile.as_mut().map(std::mem::take)
    }

    pub fn stall_cycles(&self) -> u128 {
        self.stall_cycles
    }

    pub fn flush_events(&self) -> u128 {
        self.flush_events
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

    pub fn set_irq_input(&mut self, level: bool, number: u8) {
        self.external_irq_line = level;
        self.external_irq_number = number & 0x0F;
        self.refresh_irq_input();
    }

    fn refresh_irq_input(&mut self) {
        let level = self.external_irq_line || self.irqc_line;
        let number = if self.external_irq_line {
            self.external_irq_number
        } else {
            self.irqc_number
        };
        if level && !self.irq_line {
            self.irq_pending = true;
            self.irq_pending_number = number & 0x0F;
        }
        self.irq_line = level;
    }

    pub fn set_irq_source(&mut self, number: u8, level: bool) {
        self.irq_controller.borrow_mut().set_source(number, level);
    }

    pub fn read_mem_u8(&mut self, addr: u32) -> u8 {
        self.bus.set_access_source(BusAccessSource::Debugger);
        let value = self.bus.read_u8(addr);
        self.bus.set_access_source(BusAccessSource::Cpu);
        value
    }

    pub fn read_mem_u32_be(&mut self, addr: u32) -> u32 {
        self.bus.set_access_source(BusAccessSource::Debugger);
        let value = self.bus.read_u32_be(addr);
        self.bus.set_access_source(BusAccessSource::Cpu);
        value
    }

    pub fn write_mem_u8(&mut self, addr: u32, value: u8) {
        self.bus.set_access_source(BusAccessSource::Debugger);
        self.bus.write_u8(addr, value);
        self.bus.set_access_source(BusAccessSource::Cpu);
    }

    pub fn write_mem_u32_be(&mut self, addr: u32, value: u32) {
        self.bus.set_access_source(BusAccessSource::Debugger);
        self.bus.write_u32_be(addr, value);
        self.bus.set_access_source(BusAccessSource::Cpu);
    }

    pub fn bus_memory_map(&self) -> impl Iterator<Item = BusRegion> + '_ {
        self.bus.memory_map()
    }

    pub fn read_debug_mem_u8(&mut self, addr: u32) -> Option<u8> {
        self.bus.set_access_source(BusAccessSource::Debugger);
        let value = self.bus.read_debug_u8(addr);
        self.bus.set_access_source(BusAccessSource::Cpu);
        value
    }

    pub fn read_u32(&mut self, addr: u32) -> u32 {
        self.read_mem_u32_be(addr)
    }

    pub fn read_u16_be(&mut self, addr: u32) -> u16 {
        self.bus.set_access_source(BusAccessSource::Debugger);
        let hi = self.bus.read_u8(addr) as u16;
        let lo = self.bus.read_u8(addr.wrapping_add(1)) as u16;
        self.bus.set_access_source(BusAccessSource::Cpu);
        (hi << 8) | lo
    }

    pub fn decode_at(&mut self, addr: u32, mode: InstructionMode) -> DecodedInstruction {
        match mode {
            InstructionMode::Normal => {
                let raw = self.read_mem_u32_be(addr);
                let insn = Self::decode(raw);
                let next_mode = match insn {
                    Instruction::Jmps { .. }
                    | Instruction::Jals { .. }
                    | Instruction::Jrs { .. }
                    | Instruction::Jalrs { .. } => InstructionMode::Short,
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
                let raw = self.read_u16_be(addr);
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
        let pc = self.retire_pc;
        let mode = self.retire_mode;
        self.decode_at(pc, mode)
    }

    pub fn disassemble_at(&mut self, addr: u32) -> String {
        self.decode_at(addr, InstructionMode::Normal).text
    }

    pub fn return_state_text(&mut self) -> String {
        let pc = self.retire_pc;
        let decoded = self.decode_current();
        let mut txt = format!(
            "PC=0x{pc:08X} MODE={:?} OP={}\n",
            self.retire_mode, decoded.text
        );
        txt.push_str(&format!(
            "EPC=0x{:08X} CAUSE=0x{:02X} STATUS=0x{:08X} (IRQ {})\n",
            self.epc,
            self.cause,
            u32::from(self.irq_enable),
            if self.irq_enable {
                "enabled"
            } else {
                "disabled"
            },
        ));
        for i in 0..32 {
            txt.push_str(&format!(" R{i:<2}=0x{:08X}", self.read_reg(i)));
            if i % 4 == 3 {
                txt.push('\n');
            }
        }
        txt
    }

    fn format_instruction(insn: Instruction) -> String {
        match insn {
            Instruction::Nop => "NOP".to_string(),
            Instruction::Ld { rd, base, offset } => format!("LD R{rd}, [R{base} + {offset}]"),
            Instruction::St { rd, base, offset } => format!("ST [R{base} + {offset}], R{rd}"),
            Instruction::Add { rd, rs1, rs2 } => format!("ADD R{rd}, R{rs1}, R{rs2}"),
            Instruction::Addi { rd, rs1, imm } => format!("ADDI R{rd}, R{rs1}, {imm}"),
            Instruction::Sub { rd, rs1, rs2 } => format!("SUB R{rd}, R{rs1}, R{rs2}"),
            Instruction::Slt { rd, rs1, rs2 } => format!("SLT R{rd}, R{rs1}, R{rs2}"),
            Instruction::Beq { rs1, rs2, offset } => format!("BEQ R{rs1}, R{rs2}, {offset}"),
            Instruction::Bne { rs1, rs2, offset } => format!("BNE R{rs1}, R{rs2}, {offset}"),
            Instruction::Jmp { offset } => format!("JMP {offset}"),
            Instruction::Jal { offset } => format!("JAL {offset}"),
            Instruction::Jr { rd } => format!("JR R{rd}"),
            Instruction::Jmps { offset } => format!("JMPS {offset}"),
            Instruction::Jals { offset } => format!("JALS {offset}"),
            Instruction::Jrs { rd } => format!("JRS R{rd}"),
            Instruction::Jalr { rd } => format!("JALR R{rd}"),
            Instruction::Jalrs { rd } => format!("JALRS R{rd}"),
            Instruction::Cpuid => "CPUID".to_string(),
            Instruction::Halt => "HALT".to_string(),
            Instruction::Iret => "IRET".to_string(),
            Instruction::And { rd, rs1, rs2 } => format!("AND R{rd}, R{rs1}, R{rs2}"),
            Instruction::Or { rd, rs1, rs2 } => format!("OR R{rd}, R{rs1}, R{rs2}"),
            Instruction::Xor { rd, rs1, rs2 } => format!("XOR R{rd}, R{rs1}, R{rs2}"),
            Instruction::Sll { rd, rs1, rs2 } => format!("SLL R{rd}, R{rs1}, R{rs2}"),
            Instruction::Srl { rd, rs1, rs2 } => format!("SRL R{rd}, R{rs1}, R{rs2}"),
            Instruction::Sla { rd, rs1, rs2 } => format!("SLA R{rd}, R{rs1}, R{rs2}"),
            Instruction::Sra { rd, rs1, rs2 } => format!("SRA R{rd}, R{rs1}, R{rs2}"),
            Instruction::Sltu { rd, rs1, rs2 } => format!("SLTU R{rd}, R{rs1}, R{rs2}"),
            Instruction::Mul { rd, rs1, rs2 } => format!("MUL R{rd}, R{rs1}, R{rs2}"),
            Instruction::Div { rd, rs1, rs2 } => format!("DIV R{rd}, R{rs1}, R{rs2}"),
            Instruction::Mod { rd, rs1, rs2 } => format!("MOD R{rd}, R{rs1}, R{rs2}"),
            Instruction::Mulh { rd, rs1, rs2 } => format!("MULH R{rd}, R{rs1}, R{rs2}"),
            Instruction::Divu { rd, rs1, rs2 } => format!("DIVU R{rd}, R{rs1}, R{rs2}"),
            Instruction::Ldb { rd, base, offset } => format!("LDB R{rd}, [R{base} + {offset}]"),
            Instruction::Ldh { rd, base, offset } => format!("LDH R{rd}, [R{base} + {offset}]"),
            Instruction::Stb { rd, base, offset } => format!("STB [R{base} + {offset}], R{rd}"),
            Instruction::Sth { rd, base, offset } => format!("STH [R{base} + {offset}], R{rd}"),
            Instruction::Ldil { rd, imm } => format!("LDIL R{rd}, 0x{imm:04X}"),
            Instruction::Ldih { rd, imm } => format!("LDIH R{rd}, 0x{imm:04X}"),
            Instruction::Unknown(raw) => format!(".dword 0x{raw:08X}"),
        }
    }

    fn format_short_instruction(insn: ShortInstruction) -> String {
        match insn {
            ShortInstruction::Mov { rd, rs1 } => format!("S.MOV R{rd}, R{rs1}"),
            ShortInstruction::Add { rd, rs1, rs2 } => format!("S.ADD R{rd}, R{rs1}, R{rs2}"),
            ShortInstruction::Addi { rd, imm } => format!("S.ADDI R{rd}, {imm}"),
            ShortInstruction::Ld { rd, rs1 } => format!("S.LD R{rd}, R{rs1}"),
            ShortInstruction::St { rd, rs1 } => format!("S.ST R{rd}, R{rs1}"),
            ShortInstruction::Bz { rd, offset } => format!("S.BZ R{rd}, {offset}"),
            ShortInstruction::Bnz { rd, offset } => format!("S.BNZ R{rd}, {offset}"),
            ShortInstruction::Jr { rd } => format!("S.JR R{rd}"),
            ShortInstruction::Jal { offset } => format!("S.JAL {offset}"),
            ShortInstruction::Ldi { rd, imm } => format!("S.LDI R{rd}, 0x{imm:02X}"),
            ShortInstruction::Ret => "S.RET".to_string(),
            ShortInstruction::Unknown(raw) => format!(".word 0x{raw:04X}"),
        }
    }

    pub fn read_reg(&self, reg: usize) -> u32 {
        if reg >= self.reg.len() {
            panic!("Invalid register index: {reg}");
        }
        if reg == REG_ZERO { 0 } else { self.reg[reg] }
    }

    pub fn write_reg(&mut self, reg: usize, value: u32) -> Result<String, String> {
        if reg >= self.reg.len() {
            return Err(format!("Invalid register index: {reg}"));
        }
        if reg == REG_ZERO {
            return Ok("Warning: Writing to R0 has no effect".into());
        }
        self.reg[reg] = value;
        self.sync_register_block();
        Ok("".into())
    }

    // ---- decode (mirrors crate::cpu::Cpu) ----

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
            0x21 => Instruction::Jalr { rd },
            0x22 => Instruction::Jalrs { rd },
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

    fn short_reg_to_gpr(sr: u8) -> usize {
        if sr < 15 { sr as usize } else { REG_LR }
    }

    fn add_signed(base: u32, offset: i16) -> u32 {
        base.wrapping_add((offset as i32) as u32)
    }

    fn branch_target(next_pc: u32, offset: i16) -> u32 {
        next_pc.wrapping_add((offset as i32) as u32)
    }

    fn read_arch(&self, reg: usize) -> u32 {
        if reg == REG_ZERO { 0 } else { self.reg[reg] }
    }

    /// Decode IF/ID into an ID/EX entry, sampling architectural registers.
    fn decode_id(&self, if_id: &IfId) -> IdEx {
        let mut out = invalid_id_ex();
        out.valid = true;
        out.pc = if_id.pc;
        out.mode = if_id.mode;
        match if_id.raw {
            RawInstr::Normal(raw) => {
                let insn = Self::decode(raw);
                out.decoded = Decoded::Normal(insn);
                match insn {
                    Instruction::Nop => {}
                    Instruction::Ld { rd, base, .. }
                    | Instruction::Ldb { rd, base, .. }
                    | Instruction::Ldh { rd, base, .. } => {
                        out.srcs[0] = base as usize;
                        out.n_srcs = 1;
                        out.op_a = self.read_arch(base as usize);
                        out.dest = Some(rd as usize);
                        out.is_load = true;
                    }
                    Instruction::St { rd, base, .. }
                    | Instruction::Stb { rd, base, .. }
                    | Instruction::Sth { rd, base, .. } => {
                        out.srcs[0] = base as usize;
                        out.srcs[1] = rd as usize;
                        out.n_srcs = 2;
                        out.op_a = self.read_arch(base as usize);
                        out.op_b = self.read_arch(rd as usize);
                        out.is_store = true;
                    }
                    Instruction::Add { rd, rs1, rs2 }
                    | Instruction::Sub { rd, rs1, rs2 }
                    | Instruction::Slt { rd, rs1, rs2 }
                    | Instruction::Sltu { rd, rs1, rs2 }
                    | Instruction::And { rd, rs1, rs2 }
                    | Instruction::Or { rd, rs1, rs2 }
                    | Instruction::Xor { rd, rs1, rs2 }
                    | Instruction::Sll { rd, rs1, rs2 }
                    | Instruction::Srl { rd, rs1, rs2 }
                    | Instruction::Sla { rd, rs1, rs2 }
                    | Instruction::Sra { rd, rs1, rs2 }
                    | Instruction::Mul { rd, rs1, rs2 }
                    | Instruction::Div { rd, rs1, rs2 }
                    | Instruction::Mod { rd, rs1, rs2 }
                    | Instruction::Mulh { rd, rs1, rs2 }
                    | Instruction::Divu { rd, rs1, rs2 } => {
                        out.srcs[0] = rs1 as usize;
                        out.srcs[1] = rs2 as usize;
                        out.n_srcs = 2;
                        out.op_a = self.read_arch(rs1 as usize);
                        out.op_b = self.read_arch(rs2 as usize);
                        out.dest = Some(rd as usize);
                    }
                    Instruction::Addi { rd, rs1, .. } => {
                        out.srcs[0] = rs1 as usize;
                        out.n_srcs = 1;
                        out.op_a = self.read_arch(rs1 as usize);
                        out.dest = Some(rd as usize);
                    }
                    Instruction::Beq { rs1, rs2, .. } | Instruction::Bne { rs1, rs2, .. } => {
                        out.srcs[0] = rs1 as usize;
                        out.srcs[1] = rs2 as usize;
                        out.n_srcs = 2;
                        out.op_a = self.read_arch(rs1 as usize);
                        out.op_b = self.read_arch(rs2 as usize);
                    }
                    Instruction::Jmp { .. } | Instruction::Jal { .. } => {
                        if matches!(insn, Instruction::Jal { .. }) {
                            out.dest = Some(REG_LR);
                        }
                    }
                    Instruction::Jr { rd } | Instruction::Jrs { rd } => {
                        out.srcs[0] = rd as usize;
                        out.n_srcs = 1;
                        out.op_a = self.read_arch(rd as usize);
                        if matches!(insn, Instruction::Jrs { .. }) {
                            // JRS writes no register.
                        }
                    }
                    Instruction::Jmps { .. } => {}
                    Instruction::Jals { .. } => {
                        out.dest = Some(REG_LR);
                    }
                    Instruction::Jalr { rd } | Instruction::Jalrs { rd } => {
                        out.srcs[0] = rd as usize;
                        out.n_srcs = 1;
                        out.op_a = self.read_arch(rd as usize);
                        out.dest = Some(REG_LR);
                    }
                    Instruction::Ldil { rd, .. } | Instruction::Ldih { rd, .. } => {
                        // Reads old value of rd, then merges the half.
                        out.srcs[0] = rd as usize;
                        out.n_srcs = 1;
                        out.op_a = self.read_arch(rd as usize);
                        out.dest = Some(rd as usize);
                    }
                    Instruction::Cpuid => {}
                    Instruction::Halt | Instruction::Iret | Instruction::Unknown(_) => {}
                }
            }
            RawInstr::Short(raw) => {
                let insn = Self::decode_short(raw);
                out.decoded = Decoded::Short(insn);
                match insn {
                    ShortInstruction::Mov { rd, rs1 } => {
                        let r = Self::short_reg_to_gpr(rd);
                        let s = Self::short_reg_to_gpr(rs1);
                        out.srcs[0] = s;
                        out.n_srcs = 1;
                        out.op_a = self.read_arch(s);
                        out.dest = Some(r);
                    }
                    ShortInstruction::Add { rd, rs1, rs2 } => {
                        let r = Self::short_reg_to_gpr(rd);
                        let s1 = Self::short_reg_to_gpr(rs1);
                        let s2 = Self::short_reg_to_gpr(rs2);
                        out.srcs[0] = s1;
                        out.srcs[1] = s2;
                        out.n_srcs = 2;
                        out.op_a = self.read_arch(s1);
                        out.op_b = self.read_arch(s2);
                        out.dest = Some(r);
                    }
                    ShortInstruction::Addi { rd, .. } => {
                        let r = Self::short_reg_to_gpr(rd);
                        out.srcs[0] = r;
                        out.n_srcs = 1;
                        out.op_a = self.read_arch(r);
                        out.dest = Some(r);
                    }
                    ShortInstruction::Ld { rd, rs1 } => {
                        let r = Self::short_reg_to_gpr(rd);
                        let s = Self::short_reg_to_gpr(rs1);
                        out.srcs[0] = s;
                        out.n_srcs = 1;
                        out.op_a = self.read_arch(s);
                        out.dest = Some(r);
                        out.is_load = true;
                    }
                    ShortInstruction::St { rd, rs1 } => {
                        let r = Self::short_reg_to_gpr(rd);
                        let s = Self::short_reg_to_gpr(rs1);
                        out.srcs[0] = r;
                        out.srcs[1] = s;
                        out.n_srcs = 2;
                        out.op_a = self.read_arch(r);
                        out.op_b = self.read_arch(s);
                        out.is_store = true;
                    }
                    ShortInstruction::Bz { rd, .. } | ShortInstruction::Bnz { rd, .. } => {
                        let r = Self::short_reg_to_gpr(rd);
                        out.srcs[0] = r;
                        out.n_srcs = 1;
                        out.op_a = self.read_arch(r);
                    }
                    ShortInstruction::Jr { rd } => {
                        let r = Self::short_reg_to_gpr(rd);
                        out.srcs[0] = r;
                        out.n_srcs = 1;
                        out.op_a = self.read_arch(r);
                    }
                    ShortInstruction::Jal { .. } => {
                        out.dest = Some(REG_LR);
                    }
                    ShortInstruction::Ldi { rd, .. } => {
                        out.dest = Some(Self::short_reg_to_gpr(rd));
                    }
                    ShortInstruction::Ret => {}
                    ShortInstruction::Unknown(_) => {}
                }
            }
        }
        out
    }

    /// Forwarding mux for EX inputs. Returns the newest in-flight value when a
    /// producer still occupies EX/MEM or MEM/WB, and otherwise reads the live
    /// register file. The live read (not the value sampled at ID) is required
    /// for correctness when an instruction waits several ticks between ID and
    /// EX (MUL/DIV delay): producers may commit to the register file after ID
    /// sampled it, and by EX time their pipeline entries are already gone.
    /// In-order commit guarantees no younger instruction can have committed
    /// ahead of the instruction in EX, so the live read is either the
    /// producer's committed value or an unrelated (unread) register.
    fn forwarded(&self, reg: usize, _id_value: u32) -> u32 {
        if reg == REG_ZERO {
            return 0;
        }
        // EX/MEM has the newest value, but a load there is not readable yet
        // (its data arrives in MEM). The load-use stall guarantees we never
        // need it here; guard anyway so we never forward a stale address.
        if self.ex_mem.valid
            && self.ex_mem.writeback
            && !self.ex_mem.is_load
            && self.ex_mem.dest == Some(reg)
        {
            return self.ex_mem.alu_result;
        }
        if self.mem_wb.valid && self.mem_wb.writeback && self.mem_wb.dest == Some(reg) {
            return self.mem_wb.value;
        }
        // Link values (JAL/JALR) are also forwarded via dest/value above
        // because EX sets dest=LR with alu_result=return address.
        // Otherwise the producer has committed already (or there is none):
        // read the live register file. See the method docs above.
        self.read_arch(reg)
    }

    /// Execute ID/EX, producing the next EX/MEM entry.
    /// Returns (ex_mem, branch_redirect) where redirect is (target, mode).
    fn execute_ex(&self, id_ex: &IdEx) -> (ExMem, Option<(u32, InstructionMode)>) {
        let mut out = invalid_ex_mem();
        out.valid = true;
        out.pc = id_ex.pc;
        out.mode = id_ex.mode;
        out.decoded = id_ex.decoded;
        out.dest = id_ex.dest;
        // Default: ALU op writes back its result.
        out.writeback = id_ex.dest.is_some();
        out.is_load = id_ex.is_load;
        out.is_store = id_ex.is_store;

        let next_pc_normal = id_ex.pc.wrapping_add(INSN_SIZE);
        let next_pc_short = id_ex.pc.wrapping_add(SHORT_INSN_SIZE);
        let fwd_a = self.forwarded(id_ex.srcs[0], id_ex.op_a);
        let fwd_b = if id_ex.n_srcs >= 2 {
            self.forwarded(id_ex.srcs[1], id_ex.op_b)
        } else {
            id_ex.op_b
        };

        match id_ex.decoded {
            Decoded::Normal(insn) => match insn {
                Instruction::Nop => {
                    out.valid = true;
                    out.writeback = false;
                    out.dest = None;
                }
                Instruction::Add { .. } => out.alu_result = fwd_a.wrapping_add(fwd_b),
                Instruction::Addi { imm, .. } => {
                    out.alu_result = fwd_a.wrapping_add((imm as i32) as u32);
                }
                Instruction::Sub { .. } => out.alu_result = fwd_a.wrapping_sub(fwd_b),
                Instruction::Slt { .. } => {
                    out.alu_result = u32::from((fwd_a as i32) < (fwd_b as i32));
                }
                Instruction::Sltu { .. } => out.alu_result = u32::from(fwd_a < fwd_b),
                Instruction::And { .. } => out.alu_result = fwd_a & fwd_b,
                Instruction::Or { .. } => out.alu_result = fwd_a | fwd_b,
                Instruction::Xor { .. } => out.alu_result = fwd_a ^ fwd_b,
                Instruction::Sll { .. } | Instruction::Sla { .. } => {
                    out.alu_result = fwd_a.wrapping_shl(fwd_b & 0x1F);
                }
                Instruction::Srl { .. } => out.alu_result = fwd_a.wrapping_shr(fwd_b & 0x1F),
                Instruction::Sra { .. } => {
                    out.alu_result = ((fwd_a as i32) >> (fwd_b & 0x1F)) as u32;
                }
                Instruction::Mul { .. } => {
                    out.alu_result = (fwd_a as u64).wrapping_mul(fwd_b as u64) as u32;
                }
                Instruction::Mulh { .. } => {
                    out.alu_result =
                        (((fwd_a as i32 as i64).wrapping_mul(fwd_b as i32 as i64)) >> 32) as u32;
                }
                Instruction::Div { .. } => {
                    let lhs = fwd_a as i32;
                    let rhs = fwd_b as i32;
                    out.alu_result = (if rhs == 0 {
                        0
                    } else if lhs == i32::MIN && rhs == -1 {
                        i32::MIN
                    } else {
                        lhs / rhs
                    }) as u32;
                }
                Instruction::Mod { .. } => {
                    let lhs = fwd_a as i32;
                    let rhs = fwd_b as i32;
                    out.alu_result = (if rhs == 0 {
                        0
                    } else if lhs == i32::MIN && rhs == -1 {
                        0
                    } else {
                        lhs % rhs
                    }) as u32;
                }
                Instruction::Divu { .. } => {
                    out.alu_result = if fwd_b == 0 { 0 } else { fwd_a / fwd_b };
                }
                Instruction::Ldil { imm, .. } => {
                    // op_a is the forwarded old value of rd.
                    out.alu_result = (fwd_a & 0xFFFF_0000) | u32::from(imm);
                }
                Instruction::Ldih { imm, .. } => {
                    out.alu_result = (fwd_a & 0x0000_FFFF) | (u32::from(imm) << 16);
                }
                Instruction::Ld {
                    base: _, offset, ..
                }
                | Instruction::Ldb {
                    base: _, offset, ..
                }
                | Instruction::Ldh {
                    base: _, offset, ..
                } => {
                    out.mem_addr = Self::add_signed(fwd_a, offset);
                    out.mem_size = match insn {
                        Instruction::Ld { .. } => 4,
                        Instruction::Ldb { .. } => 1,
                        _ => 2,
                    };
                    out.is_mem_read = true;
                    out.writeback = true;
                }
                Instruction::St {
                    base: _, offset, ..
                }
                | Instruction::Stb {
                    base: _, offset, ..
                }
                | Instruction::Sth {
                    base: _, offset, ..
                } => {
                    out.mem_addr = Self::add_signed(fwd_a, offset);
                    out.store_value = fwd_b;
                    out.mem_size = match insn {
                        Instruction::St { .. } => 4,
                        Instruction::Stb { .. } => 1,
                        _ => 2,
                    };
                    out.is_mem_read = false;
                    out.writeback = false;
                    out.dest = None;
                }
                Instruction::Beq { offset, .. } => {
                    out.writeback = false;
                    out.dest = None;
                    if fwd_a == fwd_b {
                        out.branch_taken = true;
                        out.branch_target = Self::branch_target(next_pc_normal, offset);
                        return (out, Some((out.branch_target, InstructionMode::Normal)));
                    }
                }
                Instruction::Bne { offset, .. } => {
                    out.writeback = false;
                    out.dest = None;
                    if fwd_a != fwd_b {
                        out.branch_taken = true;
                        out.branch_target = Self::branch_target(next_pc_normal, offset);
                        return (out, Some((out.branch_target, InstructionMode::Normal)));
                    }
                }
                Instruction::Jmp { offset } => {
                    out.writeback = false;
                    out.dest = None;
                    out.branch_taken = true;
                    out.branch_target = Self::branch_target(next_pc_normal, offset);
                    return (out, Some((out.branch_target, InstructionMode::Normal)));
                }
                Instruction::Jal { offset } => {
                    out.alu_result = next_pc_normal;
                    out.writeback = true;
                    out.branch_taken = true;
                    out.branch_target = Self::branch_target(next_pc_normal, offset);
                    return (out, Some((out.branch_target, InstructionMode::Normal)));
                }
                Instruction::Jr { .. } => {
                    out.writeback = false;
                    out.dest = None;
                    out.branch_taken = true;
                    out.branch_target = fwd_a;
                    return (out, Some((out.branch_target, InstructionMode::Normal)));
                }
                Instruction::Jmps { offset } => {
                    out.writeback = false;
                    out.dest = None;
                    out.branch_taken = true;
                    out.branch_target = Self::branch_target(next_pc_normal, offset);
                    out.branch_new_mode = Some(InstructionMode::Short);
                    return (out, Some((out.branch_target, InstructionMode::Short)));
                }
                Instruction::Jals { offset } => {
                    out.alu_result = next_pc_normal;
                    out.writeback = true;
                    out.branch_taken = true;
                    out.branch_target = Self::branch_target(next_pc_normal, offset);
                    out.branch_new_mode = Some(InstructionMode::Short);
                    return (out, Some((out.branch_target, InstructionMode::Short)));
                }
                Instruction::Jrs { .. } => {
                    out.writeback = false;
                    out.dest = None;
                    out.branch_taken = true;
                    out.branch_target = fwd_a;
                    out.branch_new_mode = Some(InstructionMode::Short);
                    return (out, Some((out.branch_target, InstructionMode::Short)));
                }
                Instruction::Jalr { .. } => {
                    out.alu_result = next_pc_normal;
                    out.writeback = true;
                    out.branch_taken = true;
                    out.branch_target = fwd_a;
                    return (out, Some((out.branch_target, InstructionMode::Normal)));
                }
                Instruction::Jalrs { .. } => {
                    out.alu_result = next_pc_normal;
                    out.writeback = true;
                    out.branch_taken = true;
                    out.branch_target = fwd_a;
                    out.branch_new_mode = Some(InstructionMode::Short);
                    return (out, Some((out.branch_target, InstructionMode::Short)));
                }
                Instruction::Cpuid => {
                    out.writeback = false;
                    out.dest = None;
                    out.cpuid = true;
                }
                Instruction::Halt => {
                    out.writeback = false;
                    out.dest = None;
                    out.halt = true;
                }
                Instruction::Iret => {
                    out.writeback = false;
                    out.dest = None;
                    out.iret = true;
                    out.branch_taken = true;
                    out.branch_target = self.epc;
                    return (out, Some((self.epc, self.irq_return_mode)));
                }
                Instruction::Unknown(raw) => {
                    panic!(
                        "Illegal instruction at PC=0x{:08X}: 0x{:08X}",
                        id_ex.pc, raw
                    );
                }
            },
            Decoded::Short(sinsn) => match sinsn {
                ShortInstruction::Mov { .. } => out.alu_result = fwd_a,
                ShortInstruction::Add { .. } => out.alu_result = fwd_a.wrapping_add(fwd_b),
                ShortInstruction::Addi { imm, .. } => {
                    out.alu_result = fwd_a.wrapping_add((imm as i32) as u32);
                }
                ShortInstruction::Ldi { imm, .. } => {
                    out.alu_result = u32::from(imm);
                }
                ShortInstruction::Ld { .. } => {
                    out.mem_addr = fwd_a;
                    out.mem_size = 4;
                    out.is_mem_read = true;
                    out.writeback = true;
                }
                ShortInstruction::St { .. } => {
                    // op_a = address (rd), op_b = value (rs1).
                    out.mem_addr = fwd_a;
                    out.store_value = fwd_b;
                    out.mem_size = 4;
                    out.is_mem_read = false;
                    out.writeback = false;
                    out.dest = None;
                }
                ShortInstruction::Bz { offset, .. } => {
                    out.writeback = false;
                    out.dest = None;
                    if fwd_a == 0 {
                        out.branch_taken = true;
                        out.branch_target = Self::branch_target(next_pc_short, offset as i16);
                        return (out, Some((out.branch_target, InstructionMode::Short)));
                    }
                }
                ShortInstruction::Bnz { offset, .. } => {
                    out.writeback = false;
                    out.dest = None;
                    if fwd_a != 0 {
                        out.branch_taken = true;
                        out.branch_target = Self::branch_target(next_pc_short, offset as i16);
                        return (out, Some((out.branch_target, InstructionMode::Short)));
                    }
                }
                ShortInstruction::Jr { .. } => {
                    out.writeback = false;
                    out.dest = None;
                    out.branch_taken = true;
                    out.branch_target = fwd_a;
                    return (out, Some((out.branch_target, InstructionMode::Short)));
                }
                ShortInstruction::Jal { offset } => {
                    out.alu_result = next_pc_short;
                    out.writeback = true;
                    out.branch_taken = true;
                    out.branch_target = Self::branch_target(next_pc_short, offset);
                    return (out, Some((out.branch_target, InstructionMode::Short)));
                }
                ShortInstruction::Ret => {
                    out.writeback = false;
                    out.dest = None;
                    out.branch_taken = true;
                    out.branch_target = next_pc_short;
                    out.branch_new_mode = Some(InstructionMode::Normal);
                    return (out, Some((next_pc_short, InstructionMode::Normal)));
                }
                ShortInstruction::Unknown(raw) => {
                    panic!(
                        "Illegal short instruction at PC=0x{:08X}: 0x{:04X}",
                        id_ex.pc, raw
                    );
                }
            },
        }
        (out, None)
    }

    fn is_mul_div(decoded: &Decoded) -> Option<u32> {
        match decoded {
            Decoded::Normal(Instruction::Mul { .. } | Instruction::Mulh { .. }) => {
                Some(MUL_LATENCY)
            }
            Decoded::Normal(
                Instruction::Div { .. } | Instruction::Divu { .. } | Instruction::Mod { .. },
            ) => Some(DIV_LATENCY),
            _ => None,
        }
    }

    fn load_use_hazard(id_ex: &IdEx, id_srcs: &[usize]) -> bool {
        if !id_ex.valid || !id_ex.is_load {
            return false;
        }
        let Some(dest) = id_ex.dest else {
            return false;
        };
        if dest == REG_ZERO {
            return false;
        }
        id_srcs.contains(&dest)
    }

    fn id_sources(&self, if_id: &IfId) -> [usize; 2] {
        // At most two sources; R0 pads unused entries and cannot hazard.
        match if_id.raw {
            RawInstr::Normal(raw) => match Self::decode(raw) {
                Instruction::Ld { base, .. }
                | Instruction::Ldb { base, .. }
                | Instruction::Ldh { base, .. } => [base as usize, REG_ZERO],
                Instruction::St { rd, base, .. }
                | Instruction::Stb { rd, base, .. }
                | Instruction::Sth { rd, base, .. } => [base as usize, rd as usize],
                Instruction::Add { rs1, rs2, .. }
                | Instruction::Sub { rs1, rs2, .. }
                | Instruction::Slt { rs1, rs2, .. }
                | Instruction::Sltu { rs1, rs2, .. }
                | Instruction::And { rs1, rs2, .. }
                | Instruction::Or { rs1, rs2, .. }
                | Instruction::Xor { rs1, rs2, .. }
                | Instruction::Sll { rs1, rs2, .. }
                | Instruction::Srl { rs1, rs2, .. }
                | Instruction::Sla { rs1, rs2, .. }
                | Instruction::Sra { rs1, rs2, .. }
                | Instruction::Mul { rs1, rs2, .. }
                | Instruction::Div { rs1, rs2, .. }
                | Instruction::Mod { rs1, rs2, .. }
                | Instruction::Mulh { rs1, rs2, .. }
                | Instruction::Divu { rs1, rs2, .. } => [rs1 as usize, rs2 as usize],
                Instruction::Addi { rs1, .. } => [rs1 as usize, REG_ZERO],
                Instruction::Beq { rs1, rs2, .. } | Instruction::Bne { rs1, rs2, .. } => {
                    [rs1 as usize, rs2 as usize]
                }
                Instruction::Jr { rd } | Instruction::Jrs { rd } => [rd as usize, REG_ZERO],
                Instruction::Jalr { rd } | Instruction::Jalrs { rd } => [rd as usize, REG_ZERO],
                Instruction::Ldil { rd, .. } | Instruction::Ldih { rd, .. } => {
                    [rd as usize, REG_ZERO]
                }
                _ => [REG_ZERO, REG_ZERO],
            },
            RawInstr::Short(raw) => match Self::decode_short(raw) {
                ShortInstruction::Mov { rs1, .. } => [Self::short_reg_to_gpr(rs1), REG_ZERO],
                ShortInstruction::Add { rs1, rs2, .. } => {
                    [Self::short_reg_to_gpr(rs1), Self::short_reg_to_gpr(rs2)]
                }
                ShortInstruction::Addi { rd, .. } => [Self::short_reg_to_gpr(rd), REG_ZERO],
                ShortInstruction::Ld { rs1, .. } => [Self::short_reg_to_gpr(rs1), REG_ZERO],
                ShortInstruction::St { rd, rs1 } => {
                    [Self::short_reg_to_gpr(rd), Self::short_reg_to_gpr(rs1)]
                }
                ShortInstruction::Bz { rd, .. } | ShortInstruction::Bnz { rd, .. } => {
                    [Self::short_reg_to_gpr(rd), REG_ZERO]
                }
                ShortInstruction::Jr { rd } => [Self::short_reg_to_gpr(rd), REG_ZERO],
                _ => [REG_ZERO, REG_ZERO],
            },
        }
    }

    fn accept_pending_irq(&mut self) -> bool {
        if !self.irq_enable || !self.irq_pending {
            return false;
        }
        if crate::devices::pec::pec::pec_trace_enabled() {
            tracing::info!(
                target: "pec_debug",
                "[PEC-DBG] CPU IRQ accept cpu=pipeline cause={} pc=0x{:08X} running_cycles={} instructions={}",
                self.irq_pending_number,
                self.retire_pc,
                self.running_cycles,
                self.instructions_executed
            );
        }
        // Precise interrupt: resume after the last committed instruction.
        self.epc = self.retire_pc;
        self.irq_return_mode = self.retire_mode;
        self.cause = self.irq_pending_number;
        self.irq_pending = false;
        self.irq_enable = false;
        self.fetch_mode = InstructionMode::Normal;
        let vector_pc = IRQ_VECTOR_BASE.wrapping_add(u32::from(self.cause) * 4);
        self.fetch_pc = vector_pc;
        // The empty-pipeline sync on the next tick reloads fetch_pc from retire_pc.
        // Keep the vector there; EPC already holds the interrupted PC.
        self.retire_pc = vector_pc;
        self.retire_mode = InstructionMode::Normal;
        self.if_id.valid = false;
        self.id_ex = invalid_id_ex();
        self.ex_mem = invalid_ex_mem();
        self.mem_wb = invalid_mem_wb();
        self.ex_cycles_left = 0;
        self.ex_armed = false;
        self.running = true;
        true
    }

    fn enter_bus_error(&mut self, fault_pc: u32) {
        self.epc = fault_pc;
        self.irq_return_mode = self.retire_mode;
        self.cause = 2;
        self.irq_enable = false;
        self.irq_pending = false;
        self.fetch_mode = InstructionMode::Normal;
        self.retire_mode = InstructionMode::Normal;
        let vector = self.bus.read_u32_be(BUS_ERROR_VECTOR);
        self.fetch_pc = vector;
        self.retire_pc = vector;
        self.if_id.valid = false;
        self.id_ex = invalid_id_ex();
        self.ex_mem = invalid_ex_mem();
        self.mem_wb = invalid_mem_wb();
        self.ex_cycles_left = 0;
        self.ex_armed = false;
        self.running = vector != BUS_ERROR_VECTOR;
        self.sync_register_block();
        tracing::error!(pc = format_args!("0x{fault_pc:08X}"), "CPU Bus Error");
    }

    fn commit_mem_wb(&mut self, wb: MemWb) {
        if !wb.valid {
            return;
        }
        self.instructions_executed += 1;
        // Advance the architectural PC. Taken branches (redirected at EX)
        // retire to their target; everything else falls through to the next
        // sequential address in the committed instruction's own mode.
        let fallthrough = wb.pc.wrapping_add(match wb.mode {
            InstructionMode::Normal => INSN_SIZE,
            InstructionMode::Short => SHORT_INSN_SIZE,
        });
        if wb.branch_taken {
            self.retire_pc = wb.branch_target;
        } else if !wb.iret {
            // IRET sets retire_pc = EPC below together with fetch state.
            self.retire_pc = fallthrough;
        }
        if wb.cpuid {
            self.reg[REG_CPUID] = CPU_ID;
            self.reg[REG_FEATURES] = CPU_FEATURES;
            self.reg[REG_ZERO] = 0;
        }
        if let (Some(dest), true) = (wb.dest, wb.writeback) {
            if dest != REG_ZERO {
                self.reg[dest] = wb.value;
            }
            self.reg[REG_ZERO] = 0;
        }
        if let Some((lr, val)) = wb.link_value {
            if lr != REG_ZERO {
                self.reg[lr] = val;
            }
            self.reg[REG_ZERO] = 0;
        }
        if let Some(mode) = wb.branch_new_mode {
            self.fetch_mode = mode;
            self.retire_mode = mode;
        }
        if wb.iret {
            if crate::devices::pec::pec::pec_trace_enabled() {
                tracing::info!(
                    target: "pec_debug",
                    "[PEC-DBG] CPU IRET cpu=pipeline pc=0x{:08X} epc=0x{:08X} cause={} running_cycles={} instructions={}",
                    wb.pc,
                    self.epc,
                    self.cause,
                    self.running_cycles,
                    self.instructions_executed
                );
            }
            // IRET itself was already redirected at EX; the commit restores
            // the architectural PC/mode saved at interrupt entry.
            self.fetch_pc = self.epc;
            self.fetch_mode = self.irq_return_mode;
            self.retire_pc = self.epc;
            self.retire_mode = self.irq_return_mode;
            self.irq_enable = true;
        }
        if wb.halt {
            // Precise HALT: takes effect at commit like the scalar CPU.
            // Over-fetched fall-through instructions are discarded (the
            // scalar CPU never fetched them); resume address is already in
            // retire_pc. Wake-up via IRQ refetches from retire_pc (EPC).
            self.running = false;
            self.if_id.valid = false;
            self.id_ex = invalid_id_ex();
            self.ex_mem = invalid_ex_mem();
            self.ex_armed = false;
            self.ex_cycles_left = 0;
        }
    }

    #[inline(always)]
    fn profile_start<const PROFILE: bool>() -> Option<Instant> {
        if PROFILE { Some(Instant::now()) } else { None }
    }

    #[inline(always)]
    fn record_profile_time<const PROFILE: bool>(
        &mut self,
        stage: CpuProfileStage,
        duration: std::time::Duration,
    ) {
        if PROFILE {
            if let Some(profile) = self.profile.as_mut() {
                profile.record(stage, duration);
            }
        }
    }

    /// Advance exactly one pipeline clock.
    pub fn tick(&mut self) {
        if self.profile.is_some() {
            self.tick_with_profile::<true>();
        } else {
            self.tick_with_profile::<false>();
        }
    }

    #[inline(always)]
    fn tick_with_profile<const PROFILE: bool>(&mut self) {
        let device_start = Self::profile_start::<PROFILE>();
        self.bus.tick_devices();
        let irq_output = self.irq_controller.borrow().irq_output();
        self.irqc_line = irq_output.valid;
        self.irqc_number = irq_output.number;
        self.refresh_irq_input();
        if let Some(start) = device_start {
            self.record_profile_time::<PROFILE>(CpuProfileStage::DeviceIrq, start.elapsed());
        }
        if PROFILE {
            if let Some(profile) = self.profile.as_mut() {
                profile.ticks += 1;
            }
        }
        if self.pipeline_empty() {
            let sync_start = Self::profile_start::<PROFILE>();
            self.sync_from_register_block();
            if let Some(start) = sync_start {
                self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
            }
        }
        if !self.running {
            let idle_start = Self::profile_start::<PROFILE>();
            if !self.accept_pending_irq() {
                self.cycles += 1;
                if let Some(start) = idle_start {
                    self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
                }
                return;
            }
        }
        self.cycles += 1;
        self.running_cycles += 1;
        if self.bus.take_bus_error() {
            self.enter_bus_error(self.retire_pc);
            return;
        }

        // ---- WB (commit oldest first) ----
        // NOTE: do NOT clear mem_wb here. All stages in a tick must read the
        // *old* pipeline registers (simultaneous clock edge); EX needs the old
        // MEM/WB entry for forwarding in this same tick. It is overwritten
        // with next_mem_wb at the end of the tick.
        let wb = self.mem_wb;
        let wb_start = Self::profile_start::<PROFILE>();
        self.commit_mem_wb(wb);
        let halted_by_commit = wb.valid && wb.halt;
        if let Some(start) = wb_start {
            self.record_profile_time::<PROFILE>(CpuProfileStage::Commit, start.elapsed());
        }
        // ---- MEM ----
        let mut next_mem_wb = invalid_mem_wb();
        let memory_start = Self::profile_start::<PROFILE>();
        let mut bus_fault_pc = None;
        if self.ex_mem.valid {
            let em = self.ex_mem;
            next_mem_wb.valid = true;
            next_mem_wb.pc = em.pc;
            next_mem_wb.mode = em.mode;
            next_mem_wb.dest = em.dest;
            next_mem_wb.branch_taken = em.branch_taken;
            next_mem_wb.branch_target = em.branch_target;
            next_mem_wb.halt = em.halt;
            next_mem_wb.iret = em.iret;
            next_mem_wb.cpuid = em.cpuid;
            next_mem_wb.link_value = em.link_value;
            next_mem_wb.branch_new_mode = em.branch_new_mode;
            if em.is_load && em.mem_size != 0 {
                let v = match em.mem_size {
                    4 => self.bus.read_u32_be(em.mem_addr),
                    1 => self.bus.read_u8(em.mem_addr) as u32,
                    2 => {
                        let b0 = self.bus.read_u8(em.mem_addr) as u32;
                        let b1 = self.bus.read_u8(em.mem_addr.wrapping_add(1)) as u32;
                        (b0 << 8) | b1
                    }
                    _ => 0,
                };
                next_mem_wb.value = v;
                next_mem_wb.writeback = true;
            } else if em.is_store && em.mem_size != 0 {
                match em.mem_size {
                    4 => self.bus.write_u32_be(em.mem_addr, em.store_value),
                    1 => self.bus.write_u8(em.mem_addr, em.store_value as u8),
                    2 => {
                        self.bus.write_u8(em.mem_addr, (em.store_value >> 8) as u8);
                        self.bus
                            .write_u8(em.mem_addr.wrapping_add(1), (em.store_value & 0xFF) as u8);
                    }
                    _ => {}
                }
                next_mem_wb.writeback = false;
                next_mem_wb.dest = None;
            } else {
                next_mem_wb.value = em.alu_result;
                next_mem_wb.writeback = em.writeback;
            }
            if self.bus.take_bus_error() {
                bus_fault_pc = Some(em.pc);
            }
        }
        if let Some(start) = memory_start {
            self.record_profile_time::<PROFILE>(CpuProfileStage::Memory, start.elapsed());
        }
        if let Some(fault_pc) = bus_fault_pc {
            self.enter_bus_error(fault_pc);
            return;
        }

        // ---- EX delay countdown (multi-cycle MUL/DIV) ----
        let execute_start = Self::profile_start::<PROFILE>();
        // When armed, the op in ID/EX stays while MEM/WB drain. The counter
        // holds remaining EX ticks including the executing tick; when it
        // reaches 1 the op executes this tick (fall through, no re-arm).
        let mut delay_done = false;
        if self.ex_armed {
            if self.ex_cycles_left > 1 {
                self.ex_cycles_left -= 1;
                self.stall_cycles += 1;
                self.mem_wb = next_mem_wb;
                self.ex_mem = invalid_ex_mem();
                if wb.valid {
                    self.accept_pending_irq();
                }
                // Publish the vector PC and masked status before the next empty-pipeline
                if let Some(start) = execute_start {
                    self.record_profile_time::<PROFILE>(CpuProfileStage::Execute, start.elapsed());
                }
                let sync_start = Self::profile_start::<PROFILE>();
                self.sync_register_block();
                if let Some(start) = sync_start {
                    self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
                }
                if halted_by_commit {
                    self.if_id.valid = false;
                    self.id_ex = invalid_id_ex();
                    self.ex_mem = invalid_ex_mem();
                }
                return;
            }
            self.ex_cycles_left = 0;
            if let Some(start) = execute_start {
                self.record_profile_time::<PROFILE>(CpuProfileStage::Execute, start.elapsed());
            }
            self.ex_armed = false;
            delay_done = true;
        }

        // ---- Hazard detection ----
        // Load-use: the instruction in EX (id_ex) is a load whose destination
        // is read by the waiter in ID (if_id). Hold fetch + IF/ID, inject a
        // bubble into ID/EX, but still advance the load EX -> MEM this tick
        // so the hazard clears.
        let mut load_use_stall = false;
        if self.if_id.valid && self.id_ex.valid && self.id_ex.is_load {
            let srcs = self.id_sources(&self.if_id);
            if Self::load_use_hazard(&self.id_ex, &srcs) {
                load_use_stall = true;
            }
        }
        // Multi-cycle MUL/DIV: on the first tick the op would execute, arm
        // the EX delay counter instead of advancing it to MEM. The op stays
        // in ID/EX while the front end holds and MEM/WB drain.
        if !delay_done && !load_use_stall && self.id_ex.valid {
            if let Some(lat) = Self::is_mul_div(&self.id_ex.decoded) {
                self.ex_cycles_left = lat - 1;
                self.ex_armed = true;
                self.stall_cycles += 1;
                self.mem_wb = next_mem_wb;
                self.ex_mem = invalid_ex_mem();
                // Hold IF/ID and ID/EX; fetch held (no bus re-issue).
                if wb.valid {
                    self.accept_pending_irq();
                }
                // Keep IRQ acceptance ordered before register-block synchronization.
                let sync_start = Self::profile_start::<PROFILE>();
                self.sync_register_block();
                if let Some(start) = execute_start {
                    self.record_profile_time::<PROFILE>(CpuProfileStage::Execute, start.elapsed());
                }
                if let Some(start) = sync_start {
                    self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
                }
                if halted_by_commit {
                    self.if_id.valid = false;
                    self.id_ex = invalid_id_ex();
                    self.ex_mem = invalid_ex_mem();
                }
                return;
            }
        }

        // ---- EX ----
        // NOTE: on a load-use stall the load in ID/EX still advances EX ->
        // MEM; only the front end is held. A delayed MUL/DIV executes when
        // its countdown is done (delay_done) like a normal op.
        let mut next_ex_mem = invalid_ex_mem();
        let mut redirect: Option<(u32, InstructionMode)> = None;
        if self.id_ex.valid {
            let (em, redir) = self.execute_ex(&self.id_ex);
            next_ex_mem = em;
            redirect = redir;
        }
        if let Some(start) = execute_start {
            self.record_profile_time::<PROFILE>(CpuProfileStage::Execute, start.elapsed());
        }

        // ---- ID ----
        let decode_start = Self::profile_start::<PROFILE>();
        let mut next_id_ex = invalid_id_ex();
        if self.if_id.valid && !load_use_stall {
            next_id_ex = self.decode_id(&self.if_id);
        } else if load_use_stall {
            // Hold the waiter in IF/ID; inject a bubble into ID/EX so the
            // load advances and the hazard clears next tick.
            next_id_ex = invalid_id_ex();
            self.stall_cycles += 1;
        }
        if let Some(start) = decode_start {
            self.record_profile_time::<PROFILE>(CpuProfileStage::Decode, start.elapsed());
        }

        // ---- IF ----
        let fetch_start = Self::profile_start::<PROFILE>();
        let mut next_if_id = self.if_id;
        if load_use_stall {
            // Hold fetch: do not re-issue the bus request.
        } else if halted_by_commit {
            next_if_id.valid = false;
        } else {
            match self.fetch_mode {
                InstructionMode::Normal => {
                    let raw = self.bus.read_u32_be(self.fetch_pc);
                    next_if_id = IfId {
                        valid: true,
                        pc: self.fetch_pc,
                        mode: InstructionMode::Normal,
                        raw: RawInstr::Normal(raw),
                    };
                    self.fetch_pc = self.fetch_pc.wrapping_add(INSN_SIZE);
                }
                InstructionMode::Short => {
                    let hi = self.bus.read_u8(self.fetch_pc) as u16;
                    let lo = self.bus.read_u8(self.fetch_pc.wrapping_add(1)) as u16;
                    next_if_id = IfId {
                        valid: true,
                        pc: self.fetch_pc,
                        mode: InstructionMode::Short,
                        raw: RawInstr::Short((hi << 8) | lo),
                    };
                    self.fetch_pc = self.fetch_pc.wrapping_add(SHORT_INSN_SIZE);
                }
            }
        }

        // ---- Branch redirect: flush freshly fetched/decoded work ----
        if let Some((target, mode)) = redirect {
            self.fetch_pc = target;
            self.fetch_mode = mode;
            // The IF fetch above already consumed the fall-through path;
            // discard it plus the next ID entry when they are fall-through.
            next_if_id.valid = false;
            next_id_ex = invalid_id_ex();
            self.flush_events += 1;
            // Mode-switch commits architecturally at WB via branch_new_mode;
            // set fetch mode now so the next fetch uses the new width.
        }
        if let Some(start) = fetch_start {
            self.record_profile_time::<PROFILE>(CpuProfileStage::Fetch, start.elapsed());
        }

        let other_start = Self::profile_start::<PROFILE>();
        self.if_id = next_if_id;
        self.id_ex = next_id_ex;
        self.ex_mem = next_ex_mem;
        self.mem_wb = next_mem_wb;

        if halted_by_commit {
            // Stop fetching; let in-flight instructions drain, then idle.
            self.if_id.valid = false;
        }

        // IRQ sampled only at instruction-commit boundaries.
        if wb.valid {
            self.accept_pending_irq();
        }
        self.sync_register_block();
        if let Some(start) = other_start {
            self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
        }
    }

    pub fn step_once(&mut self) -> bool {
        if !self.running {
            self.tick();
            if !self.running {
                return false;
            }
        }
        let start = self.instructions_executed;
        // Advance until one more instruction commits.
        for _ in 0..1024 {
            if self.instructions_executed != start {
                break;
            }
            if !self.running && self.pipeline_empty() {
                break;
            }
            self.tick();
            if !self.running && self.pipeline_empty() {
                break;
            }
        }
        true
    }

    pub fn run(&mut self, max_cycles: usize) {
        if self.profile.is_some() {
            self.run_inner::<true>(max_cycles);
        } else {
            self.run_inner::<false>(max_cycles);
        }
    }

    fn run_inner<const PROFILE: bool>(&mut self, max_cycles: usize) {
        let start_cycles = self.cycles;
        let end_cycles = start_cycles.saturating_add(max_cycles as u128);
        while self.cycles < end_cycles {
            let empty = self.pipeline_empty();
            self.tick_with_profile::<PROFILE>();
            if !self.running && empty && self.pipeline_empty() && !self.bus.has_pending_dma() {
                let remaining = end_cycles - self.cycles;
                if remaining != 0 {
                    let remaining = remaining.min(u128::from(u64::MAX)) as u64;
                    let skip = self
                        .bus
                        .cycles_until_next_event()
                        .map_or(remaining, |until_irq| remaining.min(until_irq));
                    if skip != 0 {
                        self.bus.advance_system_cycles(skip);
                        self.cycles = self.cycles.saturating_add(u128::from(skip));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod hazard_tests {
    use super::*;

    #[test]
    fn hazard_sources_match_decoder_for_normal_and_all_short_encodings() {
        let cpu = PipelineCpu::new(Bus::new());
        let check = |raw| {
            let instruction = IfId {
                valid: true,
                pc: 0,
                mode: InstructionMode::Normal,
                raw,
            };
            let decoded = cpu.decode_id(&instruction);
            let sources = cpu.id_sources(&instruction);
            for register in 0..32 {
                let mut load = invalid_id_ex();
                load.valid = true;
                load.is_load = true;
                load.dest = Some(register);
                assert_eq!(
                    PipelineCpu::load_use_hazard(&load, &sources),
                    register != REG_ZERO
                        && decoded.srcs[..decoded.n_srcs as usize].contains(&register),
                    "{raw:?}, R{register}"
                );
            }
        };
        // Exercise every opcode and GPR field, including R0 and R31.
        for opcode in 0u32..64 {
            for register in 0u32..32 {
                check(RawInstr::Normal(
                    (opcode << 26)
                        | (register << 21)
                        | (((register + 1) % 32) << 16)
                        | (((register + 2) % 32) << 11),
                ));
            }
        }
        for raw in 0..=u16::MAX {
            check(RawInstr::Short(raw));
        }
    }
}
