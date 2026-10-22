// CPU: Central Processing Unit implementation
// This module defines the SRC32 CPU struct, instruction set, and execution logic for the CPT32 emulator.

// Features: SRC32-ALMSIP

use crate::bus::{Bus, BusAccessSource, BusRegion};
use crate::cpu_privilege::{
    Access, EXT_P, EXT_V, Exception, SystemState, format_sr, format_syscall,
};
use crate::devices::cpu::CpuRegisterBlock;
use crate::devices::irqc::irqc::IrqController;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

#[cfg(feature = "jit")]
#[path = "cpu_jit.rs"]
mod jit;

/// JIT compilation and execution counters; compile time is included in run time.
#[derive(Debug, Clone, Copy, Default)]
pub struct JitStats {
    /// Second-tier pure native variants (included in compile_time/code bytes).
    pub batch_compilations: u64,
    pub native_batches: u64,
    pub batched_instructions: u64,
    pub compilations: u64,
    pub compile_time: Duration,
    pub compile_failures: u64,
    pub native_instructions: u128,
    /// Interpreter ticks, excluding HALT cycles advanced in bulk.
    pub fallback_cycles: u128,
    pub invalidations: u64,
    pub cache_resets: u64,
    pub cached_blocks: usize,
    /// Live native instruction bytes; excludes allocator alignment/metadata.
    pub native_code_bytes: usize,
}

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
    Syscall { imm: u16 },
    Mfsr { rd: u8, sr: u16 },
    Mtsr { rs: u8, sr: u16 },
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
const CPU_FEATURES: u32 = EXT_BASE | EXT_A | EXT_L | EXT_M | EXT_S | EXT_I | EXT_P | EXT_V; // CPUID features bitfield
const CPU_ID: u32 = 0x5352_4332; // "SRC2" style tag
pub const IRQ_VECTOR_BASE: u32 = 0xFFFF_0100;

const INSN_SIZE: u32 = 4;
const SHORT_INSN_SIZE: u32 = 2;

/// Current phase of the non-pipelined CPU. One call to `tick` advances one state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuState {
    Fetch,
    Decode,
    Execute,
    MemRead,
    MemWrite,
    Mul,
    Div,
    Writeback,
    Halt,
}

pub const CPU_PROFILE_STAGE_NAMES: [&str; 7] = [
    "Device / IRQ",
    "Instruction fetch",
    "Decode / issue",
    "Execute",
    "Memory access",
    "Commit / writeback",
    "Stall / bookkeeping",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum CpuProfileStage {
    DeviceIrq,
    Fetch,
    Decode,
    Execute,
    Memory,
    Commit,
    Other,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuProfile {
    pub ticks: u128,
    pub stages: [Duration; CPU_PROFILE_STAGE_NAMES.len()],
}

impl CpuProfile {
    pub fn total(self) -> Duration {
        self.stages.into_iter().sum()
    }

    pub fn accumulate(&mut self, other: Self) {
        self.ticks += other.ticks;
        for (total, duration) in self.stages.iter_mut().zip(other.stages) {
            *total += duration;
        }
    }

    pub fn divided(self, count: u32) -> Self {
        Self {
            ticks: self.ticks / u128::from(count),
            stages: self.stages.map(|duration| duration / count),
        }
    }

    pub(crate) fn record(&mut self, stage: CpuProfileStage, duration: Duration) {
        self.stages[stage as usize] += duration;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingInstruction {
    RawNormal(u32),
    RawShort(u16),
    Normal(Instruction),
    Short(ShortInstruction),
}

const INSTRUCTION_CACHE_SIZE: usize = 1024;

#[derive(Clone, Copy)]
struct CachedInstruction {
    pc: u32,
    mode: InstructionMode,
    start_generation: u64,
    end_generation: u64,
    instruction: PendingInstruction,
}

/// Architectural CPU state at an instruction boundary that determines the
/// next loop iteration (with constant memory and device reads).
#[derive(Clone, Copy, PartialEq, Eq)]
struct SpinState {
    reg: [u32; 32],
    pc: u32,
    instr_mode: InstructionMode,
    sys: SystemState,
    irq_pending: bool,
    irq_pending_number: u8,
    irq_line: bool,
    external_irq_line: bool,
    external_irq_number: u8,
    irqc_line: bool,
    irqc_number: u8,
}

/// One observed iteration of a candidate polling loop.
#[derive(Clone, Copy)]
struct SpinProbe {
    state: SpinState,
    cycles: u128,
    instructions: u128,
    cache_hits: u128,
    effectful_accesses: u64,
    fills: u64,
    /// Last CPU cycle of a quiet device window measured at the loop head,
    /// once a previous iteration already matched.
    quiet_until: Option<u128>,
}

/// Exact fast-forward of loops that spin on unchanging inputs: if one whole
/// iteration ran inside a quiet device window, made no effectful bus access
/// and no decode-cache fill, and returned to an identical state, every later
/// iteration repeats it until the window ends.
/// Loop heads tracked at once: an iteration may take several branches (a
/// call, a return and the loop's back-jump), each recreating its own head.
const SPIN_PROBES: usize = 8;

struct SpinDetector {
    enabled: bool,
    last_pc: u32,
    probes: [Option<SpinProbe>; SPIN_PROBES],
    next_probe: usize,
    skipped_cycles: u128,
}

impl SpinDetector {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_pc: 0,
            probes: [None; SPIN_PROBES],
            next_probe: 0,
            skipped_cycles: 0,
        }
    }

    /// Slot holding the probe for `head`, or the round-robin victim.
    fn slot(&mut self, head: u32) -> usize {
        if let Some(slot) = self
            .probes
            .iter()
            .position(|probe| probe.is_some_and(|probe| probe.state.pc == head))
        {
            return slot;
        }
        let slot = self.next_probe;
        self.next_probe = (slot + 1) % SPIN_PROBES;
        self.probes[slot] = None;
        slot
    }
}

pub struct Cpu {
    reg: [u32; 32],
    pc: u32,
    running: bool,
    bus: Bus,
    cycles: u128,
    running_cycles: u128,
    instructions_executed: u128,
    instr_mode: InstructionMode,
    sys: SystemState,
    irq_pending: bool,
    irq_pending_number: u8,
    irq_line: bool,
    external_irq_line: bool,
    external_irq_number: u8,
    irqc_line: bool,
    irqc_number: u8,
    register_block: Rc<RefCell<CpuRegisterBlock>>,
    irq_controller: Rc<RefCell<IrqController>>,
    state: CpuState,
    instruction: Option<PendingInstruction>,
    cycles_left: u32,
    instruction_cache: Option<Box<[Option<CachedInstruction>]>>,
    instruction_cache_hits: u128,
    instruction_cache_misses: u128,
    /// Bumped on every instruction-cache slot write; JIT batch stamps and the
    /// spin detector use it to prove cached hits are still present.
    instruction_cache_fills: u64,
    spin: SpinDetector,
    defer_register_sync: bool,
    profile: Option<CpuProfile>,
    pub(crate) relaxed_devices: bool,
    relaxed_run: bool,
    #[cfg(feature = "jit")]
    jit: Option<jit::JitEngine>,
}

impl Cpu {
    pub fn new(bus: Bus) -> Self {
        Self::new_with_instruction_cache(bus, false)
    }

    pub(crate) fn new_cached(bus: Bus) -> Self {
        Self::new_with_instruction_cache(bus, true)
    }

    #[cfg(feature = "jit")]
    pub(crate) fn new_jit(bus: Bus, max_block_instructions: usize) -> Self {
        let mut cpu = Self::new_cached(bus);
        // Native blocks use the same bounded idle-device batching as the
        // experimental relaxed Cached engine. Pure native batches fit entirely
        // inside bus-free windows; other native execution validates each cycle.
        cpu.relaxed_devices = true;
        cpu.jit = Some(jit::JitEngine::new(max_block_instructions));
        cpu
    }

    /// Test hook: visits before the batch tier compiles, and declined batch
    /// dispatches before the strict tier compiles (0 = with the batch tier).
    #[cfg(feature = "jit")]
    pub(crate) fn set_jit_tier_thresholds(&mut self, hot: u32, strict: u32) {
        if let Some(jit) = &mut self.jit {
            jit.set_tier_thresholds(hot, strict);
        }
    }

    pub fn jit_stats(&self) -> Option<JitStats> {
        #[cfg(feature = "jit")]
        if let Some(jit) = &self.jit {
            return Some(jit.stats());
        }
        None
    }

    fn new_with_instruction_cache(mut bus: Bus, cached: bool) -> Self {
        if cached {
            bus.enable_instruction_cache_tracking();
        }
        let register_block = crate::devices::cpu::connect_cpu_registers(&mut bus);
        let irq_controller = crate::devices::irqc::irqc::connect_irqc(&mut bus);
        bus.attach_irq_controller(Rc::clone(&irq_controller));
        let mut cpu = Self {
            reg: [0; 32],
            pc: 0,
            running: true,
            bus,
            cycles: 0,
            running_cycles: 0,
            instructions_executed: 0,
            instr_mode: InstructionMode::Normal,
            sys: SystemState::default(),
            irq_pending: false,
            irq_pending_number: 0,
            irq_line: false,
            external_irq_line: false,
            external_irq_number: 0,
            irqc_line: false,
            irqc_number: 0,
            register_block,
            irq_controller,
            state: CpuState::Fetch,
            instruction: None,
            cycles_left: 0,
            instruction_cache: cached
                .then(|| vec![None; INSTRUCTION_CACHE_SIZE].into_boxed_slice()),
            instruction_cache_hits: 0,
            instruction_cache_misses: 0,
            instruction_cache_fills: 0,
            spin: SpinDetector::new(true),
            defer_register_sync: false,
            profile: None,
            relaxed_devices: false,
            relaxed_run: false,
            #[cfg(feature = "jit")]
            jit: None,
        };
        cpu.sync_register_block();
        cpu
    }

    pub fn reset(&mut self, pc: u32) {
        self.spin = SpinDetector::new(self.spin.enabled);
        self.bus.flush_relaxed_devices();
        self.relaxed_run = false;
        self.reg = [0; 32];
        self.pc = pc;
        self.running = true;
        self.cycles = 0;
        self.running_cycles = 0;
        self.instructions_executed = 0;
        self.instr_mode = InstructionMode::Normal;
        self.sys = SystemState::default();
        self.irq_pending = false;
        self.irq_pending_number = 0;
        self.irq_line = false;
        self.external_irq_line = false;
        self.external_irq_number = 0;
        self.irqc_line = false;
        self.irqc_number = 0;
        self.state = CpuState::Fetch;
        self.instruction = None;
        self.cycles_left = 0;
        self.defer_register_sync = false;
        #[cfg(feature = "jit")]
        if let Some(jit) = &mut self.jit {
            jit.reset();
        }
        self.bus.reset_expansion_cards();
        self.irq_controller.borrow_mut().reset();
        self.sync_register_block();
    }

    fn sync_register_block(&mut self) {
        let mut block = self.register_block.borrow_mut();
        block.regs = self.reg;
        block.regs[0] = 0;
        block.pc = self.pc;
        block.epc = self.sys.epc;
        block.cause = self.sys.cause;
        block.status = self.sys.status();
        block.instr_mode = match self.instr_mode {
            InstructionMode::Normal => 0,
            InstructionMode::Short => 1,
        };
    }

    fn trace_cpu_name(&self) -> &'static str {
        #[cfg(feature = "jit")]
        if self.jit.is_some() {
            return "jit";
        }
        if self.instruction_cache.is_some() {
            "cached"
        } else {
            "scalar"
        }
    }

    fn sync_from_register_block(&mut self) {
        let block = self.register_block.borrow();
        let irq_enable = block.status & 1 != 0;
        if self.sys.ie != irq_enable && crate::devices::pec::pec::pec_trace_enabled() {
            tracing::info!(
                target: "pec_debug",
                "[PEC-DBG] CPU status resync cpu={} old_irq_enabled={} block_irq_enabled={} block_pc=0x{:08X} block_epc=0x{:08X} cause={}",
                self.trace_cpu_name(),
                self.sys.ie,
                irq_enable,
                block.pc,
                block.epc,
                block.cause
            );
        }
        self.reg = block.regs;
        self.reg[REG_ZERO] = 0;
        self.pc = block.pc;
        self.sys.epc = block.epc;
        self.sys.cause = block.cause;
        self.sys.set_status(block.status);
        self.instr_mode = if block.instr_mode & 1 != 0 {
            InstructionMode::Short
        } else {
            InstructionMode::Normal
        };
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
        self.forget_spin();
        self.pc = pc;
        self.sync_register_block();
    }

    pub fn instruction_mode(&self) -> InstructionMode {
        self.instr_mode
    }

    pub fn set_instruction_mode(&mut self, mode: InstructionMode) {
        self.forget_spin();
        self.instr_mode = mode;
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

    pub fn state(&self) -> CpuState {
        self.state
    }

    fn is_load(insn: PendingInstruction) -> bool {
        matches!(
            insn,
            PendingInstruction::Normal(
                Instruction::Ld { .. } | Instruction::Ldb { .. } | Instruction::Ldh { .. }
            ) | PendingInstruction::Short(ShortInstruction::Ld { .. })
        )
    }
    fn is_store(insn: PendingInstruction) -> bool {
        matches!(
            insn,
            PendingInstruction::Normal(
                Instruction::St { .. } | Instruction::Stb { .. } | Instruction::Sth { .. }
            ) | PendingInstruction::Short(ShortInstruction::St { .. })
        )
    }
    fn is_mul(insn: PendingInstruction) -> bool {
        matches!(
            insn,
            PendingInstruction::Normal(Instruction::Mul { .. } | Instruction::Mulh { .. })
        )
    }
    fn is_div(insn: PendingInstruction) -> bool {
        matches!(
            insn,
            PendingInstruction::Normal(
                Instruction::Div { .. } | Instruction::Divu { .. } | Instruction::Mod { .. }
            )
        )
    }

    /// Advance one emulated clock using the selected interpreter path.
    pub fn tick(&mut self) {
        if self.profile.is_some() {
            self.tick_with_profile::<true>();
        } else {
            self.tick_with_profile::<false>();
        }
    }

    #[inline(always)]
    fn tick_with_profile<const PROFILE: bool>(&mut self) {
        if self.instruction_cache.is_some() {
            self.tick_cached::<PROFILE>();
        } else {
            self.tick_scalar::<PROFILE>();
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

    fn tick_scalar<const PROFILE: bool>(&mut self) {
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

        if self.state == CpuState::Fetch {
            let sync_start = Self::profile_start::<PROFILE>();
            self.sync_from_register_block();
            if let Some(start) = sync_start {
                self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
            }
        }
        let idle_start = if !self.running {
            Self::profile_start::<PROFILE>()
        } else {
            None
        };
        if !self.running {
            self.state = CpuState::Halt;
            if !self.accept_pending_irq() {
                // HALT still consumes clock cycles so run() can poll IRQ inputs.
                self.cycles += 1;
                if let Some(start) = idle_start {
                    self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
                }
                return;
            }
        }
        self.cycles += 1;
        self.running_cycles += 1;
        let phase = self.state;
        let phase_start = Self::profile_start::<PROFILE>();
        match phase {
            CpuState::Fetch if !self.fetch_allowed() => {}
            CpuState::Fetch => {
                let raw = match self.instr_mode {
                    InstructionMode::Normal => self.fetch_u32(),
                    InstructionMode::Short => u32::from(self.fetch_u16_at(self.pc)),
                };
                self.instruction = Some(match self.instr_mode {
                    InstructionMode::Normal => PendingInstruction::RawNormal(raw),
                    InstructionMode::Short => PendingInstruction::RawShort(raw as u16),
                });
                self.state = CpuState::Decode;
            }
            CpuState::Decode => {
                self.instruction = self.instruction.map(|instruction| match instruction {
                    PendingInstruction::RawNormal(raw) => {
                        PendingInstruction::Normal(Self::decode(raw))
                    }
                    PendingInstruction::RawShort(raw) => {
                        PendingInstruction::Short(Self::decode_short(raw))
                    }
                    other => other,
                });
                if let Some(instruction) = self.instruction {
                    self.state = if Self::is_load(instruction) {
                        CpuState::MemRead
                    } else if Self::is_store(instruction) {
                        CpuState::MemWrite
                    } else if Self::is_mul(instruction) {
                        self.cycles_left = 3;
                        CpuState::Mul
                    } else if Self::is_div(instruction) {
                        self.cycles_left = 16;
                        CpuState::Div
                    } else {
                        CpuState::Execute
                    };
                }
            }
            CpuState::Execute | CpuState::MemRead | CpuState::MemWrite => {
                self.commit_pending();
                if self.running {
                    self.state = CpuState::Writeback;
                } else {
                    self.state = CpuState::Halt;
                }
            }
            CpuState::Mul | CpuState::Div => {
                self.cycles_left -= 1;
                if self.cycles_left == 0 {
                    self.commit_pending();
                    self.state = if self.running {
                        CpuState::Writeback
                    } else {
                        CpuState::Halt
                    };
                }
            }
            CpuState::Writeback => {
                self.instruction = None;
                self.accept_pending_irq();
                self.state = CpuState::Fetch;
            }
            CpuState::Halt => {}
        }
        if let Some(start) = phase_start {
            let stage = match phase {
                CpuState::Fetch => CpuProfileStage::Fetch,
                CpuState::Decode => CpuProfileStage::Decode,
                CpuState::Execute => CpuProfileStage::Execute,
                CpuState::MemRead | CpuState::MemWrite => CpuProfileStage::Memory,
                CpuState::Mul | CpuState::Div if self.cycles_left == 0 => CpuProfileStage::Execute,
                CpuState::Mul | CpuState::Div | CpuState::Halt => CpuProfileStage::Other,
                CpuState::Writeback => CpuProfileStage::Commit,
            };
            self.record_profile_time::<PROFILE>(stage, start.elapsed());
        }
        let sync_start = Self::profile_start::<PROFILE>();
        self.sync_register_block();
        if let Some(start) = sync_start {
            self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
        }
    }

    fn tick_cached<const PROFILE: bool>(&mut self) {
        if self.begin_cached_cycle::<PROFILE>() {
            self.execute_cached_cycle::<PROFILE>();
        }
    }

    // Shared by the interpreter and native blocks. Strict ordering is devices/DMA,
    // IRQ inputs, register-block sync, then execution. Cached-relaxed and JIT
    // run() batch idle device progress; direct ticks stay strict.
    fn begin_cached_cycle<const PROFILE: bool>(&mut self) -> bool {
        let device_start = Self::profile_start::<PROFILE>();
        if self.relaxed_run {
            self.bus.tick_relaxed_devices();
        } else {
            self.bus.tick_devices();
        }
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

        if self.state == CpuState::Fetch {
            let sync_start = Self::profile_start::<PROFILE>();
            self.sync_from_register_block();
            if let Some(start) = sync_start {
                self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
            }
        }
        if !self.running {
            self.state = CpuState::Halt;
            let idle_start = Self::profile_start::<PROFILE>();
            if !self.accept_pending_irq() {
                self.cycles += 1;
                if let Some(start) = idle_start {
                    self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
                }
                return false;
            }
        }
        self.cycles += 1;
        self.running_cycles += 1;
        true
    }

    fn execute_cached_cycle<const PROFILE: bool>(&mut self) {
        let phase = self.state;
        match phase {
            CpuState::Fetch if !self.fetch_allowed() => {}
            CpuState::Fetch => {
                let fetch_start = Self::profile_start::<PROFILE>();
                let instruction = self.fetch_cached_instruction();
                self.instruction = Some(instruction);
                if let Some(start) = fetch_start {
                    self.record_profile_time::<PROFILE>(CpuProfileStage::Fetch, start.elapsed());
                }
                self.issue_cached_instruction::<PROFILE>(instruction);
            }
            CpuState::Mul | CpuState::Div => {
                let stall_start = Self::profile_start::<PROFILE>();
                self.cycles_left -= 1;
                if self.cycles_left == 0 {
                    let execute_start = Self::profile_start::<PROFILE>();
                    self.commit_cached_instruction();
                    if let Some(start) = execute_start {
                        self.record_profile_time::<PROFILE>(
                            CpuProfileStage::Execute,
                            start.elapsed(),
                        );
                    }
                } else if let Some(start) = stall_start {
                    self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
                }
            }
            CpuState::Halt => {
                let idle_start = Self::profile_start::<PROFILE>();
                if let Some(start) = idle_start {
                    self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
                }
            }
            _ => unreachable!("cached interpreter entered a scalar FSM phase"),
        }
        let sync_start = Self::profile_start::<PROFILE>();
        self.sync_register_block();
        if let Some(start) = sync_start {
            self.record_profile_time::<PROFILE>(CpuProfileStage::Other, start.elapsed());
        }
    }

    fn issue_cached_instruction<const PROFILE: bool>(&mut self, instruction: PendingInstruction) {
        self.instruction = Some(instruction);
        if Self::is_mul(instruction) {
            self.cycles_left = 2;
            self.state = CpuState::Mul;
        } else if Self::is_div(instruction) {
            self.cycles_left = 15;
            self.state = CpuState::Div;
        } else {
            let execute_start = Self::profile_start::<PROFILE>();
            self.commit_cached_instruction();
            if let Some(start) = execute_start {
                self.record_profile_time::<PROFILE>(CpuProfileStage::Execute, start.elapsed());
            }
        }
    }

    fn commit_cached_instruction(&mut self) {
        self.defer_register_sync = true;
        self.commit_pending();
        self.defer_register_sync = false;
        self.instruction = None;
        if self.running {
            self.state = CpuState::Fetch;
            self.accept_pending_irq();
        } else {
            self.state = CpuState::Halt;
        }
    }

    fn accept_pending_irq(&mut self) -> bool {
        if !self.sys.ie || !self.irq_pending {
            return false;
        }

        if crate::devices::pec::pec::pec_trace_enabled() {
            tracing::info!(
                target: "pec_debug",
                "[PEC-DBG] CPU IRQ accept cpu={} cause={} pc=0x{:08X} running_cycles={} instructions={}",
                self.trace_cpu_name(),
                self.irq_pending_number,
                self.pc,
                self.running_cycles,
                self.instructions_executed
            );
        }
        let cause = u32::from(self.irq_pending_number);
        self.sys
            .enter_trap(&mut self.reg, self.pc, cause, self.instr_mode);
        self.irq_pending = false;
        self.instr_mode = InstructionMode::Normal;
        self.pc = IRQ_VECTOR_BASE.wrapping_add(cause * 4);
        self.instruction = None;
        self.running = true;
        self.state = CpuState::Fetch;
        true
    }

    fn commit_pending(&mut self) {
        let instruction_pc = self.pc;
        match self.instruction {
            Some(PendingInstruction::Normal(instruction)) => {
                self.instructions_executed += 1;
                self.execute_normal(instruction);
            }
            Some(PendingInstruction::Short(instruction)) => {
                self.instructions_executed += 1;
                self.execute_short(instruction);
            }
            _ => {}
        }
        if self.bus.take_bus_error() {
            tracing::error!(pc = format_args!("0x{instruction_pc:08X}"), "CPU Bus Error");
            self.enter_exception(Exception::BusError, instruction_pc);
        }
    }

    /// Exception entry (doc/spec_CPU_priv.md section 6). `epc` is the faulting
    /// instruction, or the next one for SYSCALL. A vector word still holding
    /// its own address halts the CPU at that vector.
    fn enter_exception(&mut self, exception: Exception, epc: u32) {
        if let Exception::PageFault { addr, .. } = exception {
            self.sys.vm.vfault = addr;
        }
        self.sys
            .enter_trap(&mut self.reg, epc, exception.cause(), self.instr_mode);
        self.instr_mode = InstructionMode::Normal;
        let vector = exception.vector();
        self.pc = self.bus.read_u32_be(vector);
        self.instruction = None;
        self.state = CpuState::Fetch;
        self.running = self.pc != vector;
        if !self.running {
            self.state = CpuState::Halt;
        }
    }

    /// User-mode protection check; a denied access enters the protection
    /// fault before the bus is touched.
    fn access_allowed(&mut self, addr: u32, len: u32, access: Access, instruction_pc: u32) -> bool {
        match self.sys.check_access(&mut self.bus, addr, len, access) {
            Ok(()) => true,
            Err(exception) => {
                self.enter_exception(exception, instruction_pc);
                false
            }
        }
    }

    /// Fetch protection for the instruction at PC.
    fn fetch_allowed(&mut self) -> bool {
        let len = match self.instr_mode {
            InstructionMode::Normal => INSN_SIZE,
            InstructionMode::Short => SHORT_INSN_SIZE,
        };
        self.access_allowed(self.pc, len, Access::Fetch, self.pc)
    }

    /// Extension V control registers and TLB for the debugger.
    pub fn mmu_snapshot(&self) -> crate::cpu_privilege::MmuSnapshot {
        self.sys.mmu_snapshot()
    }

    /// Debugger translation of a virtual address (no TLB fill, no faults).
    pub fn mmu_translate(&mut self, va: u32) -> crate::cpu_privilege::DebugTranslation {
        let bus = &mut self.bus;
        self.sys.debug_translate(va, |addr| {
            let mut word = 0u32;
            for index in 0..4 {
                word = (word << 8) | u32::from(bus.read_debug_u8(addr.wrapping_add(index))?);
            }
            Some(word)
        })
    }

    pub fn epc(&self) -> u32 {
        self.sys.epc
    }

    pub fn irq_cause(&self) -> u32 {
        self.sys.cause
    }

    pub fn irq_enabled(&self) -> bool {
        self.sys.ie
    }

    /// STATUS as defined by Extension P.
    pub fn status(&self) -> u32 {
        self.sys.status()
    }

    /// Drive the CPU's external IRQ input. A rising edge latches one request.
    /// IRQC owns source arbitration; the CPU only receives the selected number.
    pub fn set_irq_input(&mut self, level: bool, number: u8) {
        self.forget_spin();
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

    /// Drive an external source into IRQC. Sources are edge-latched by the
    /// controller and delivered to the CPU when their enable bit is set.
    pub fn set_irq_source(&mut self, number: u8, level: bool) {
        self.forget_spin();
        self.irq_controller.borrow_mut().set_source(number, level);
    }

    pub fn read_mem_u8(&mut self, addr: u32) -> u8 {
        self.bus.set_access_source(BusAccessSource::Debugger);
        let value = self.bus.read_u8(addr);
        self.bus.set_access_source(BusAccessSource::Cpu);
        value
    }

    pub fn read_debug_mem_u8(&mut self, addr: u32) -> Option<u8> {
        self.bus.set_access_source(BusAccessSource::Debugger);
        let value = self.bus.read_debug_u8(addr);
        self.bus.set_access_source(BusAccessSource::Cpu);
        value
    }

    pub fn read_debug_mem_u16_be(&mut self, addr: u32) -> Option<u16> {
        Some(
            (u16::from(self.read_debug_mem_u8(addr)?) << 8)
                | u16::from(self.read_debug_mem_u8(addr.wrapping_add(1))?),
        )
    }

    pub fn read_debug_mem_u32_be(&mut self, addr: u32) -> Option<u32> {
        Some(
            (u32::from(self.read_debug_mem_u8(addr)?) << 24)
                | (u32::from(self.read_debug_mem_u8(addr.wrapping_add(1))?) << 16)
                | (u32::from(self.read_debug_mem_u8(addr.wrapping_add(2))?) << 8)
                | u32::from(self.read_debug_mem_u8(addr.wrapping_add(3))?),
        )
    }
    pub fn read_mem_u32_be(&mut self, addr: u32) -> u32 {
        self.bus.set_access_source(BusAccessSource::Debugger);
        let value = self.bus.read_u32_be(addr);
        self.bus.set_access_source(BusAccessSource::Cpu);
        value
    }
    /// Return the registered system-bus ranges without reading device memory.
    pub fn bus_memory_map(&self) -> impl Iterator<Item = BusRegion> + '_ {
        self.bus.memory_map()
    }

    pub fn mix_expansion_audio(&mut self, rate: u32, left: &mut [i16], right: &mut [i16]) {
        self.bus.mix_expansion_audio(rate, left, right);
    }

    pub fn expansion_card_debug_info(
        &self,
    ) -> Vec<crate::devices::expansion::ExpansionCardDebugInfo> {
        self.bus.expansion_card_debug_info()
    }

    pub fn reset_expansion_card_debug_stats(&mut self) {
        self.bus.reset_expansion_card_debug_stats();
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

    pub fn read_reg(&self, reg: usize) -> u32 {
        if reg >= self.reg.len() {
            panic!("Invalid register index: {reg}");
        }
        if reg == REG_ZERO { 0 } else { self.reg[reg] }
    }

    /// Host/debugger register write. Instruction writeback uses `set_gpr`.
    pub fn write_reg(&mut self, reg: usize, value: u32) -> Result<String, String> {
        self.forget_spin();
        self.set_gpr(reg, value)
    }

    fn set_gpr(&mut self, reg: usize, value: u32) -> Result<String, String> {
        if reg >= self.reg.len() {
            return Err(format!("Invalid register index: {reg}"));
        }
        if reg == REG_ZERO {
            return Ok("Warning: Writing to R0 has no effect".into());
        }
        self.reg[reg] = value;
        if !self.defer_register_sync {
            self.sync_register_block();
        }
        Ok("".into())
    }

    fn fetch_u32(&mut self) -> u32 {
        self.sys.read_data(&mut self.bus, self.pc, 4)
    }

    fn fetch_u32_at(&mut self, addr: u32) -> u32 {
        self.sys.read_data(&mut self.bus, addr, 4)
    }

    fn fetch_u16_at(&mut self, addr: u32) -> u16 {
        self.sys.read_data(&mut self.bus, addr, 2) as u16
    }

    fn instruction_cache_index(pc: u32, cache_len: usize) -> usize {
        ((pc >> 1) as usize) & (cache_len - 1)
    }

    fn fetch_cached_instruction(&mut self) -> PendingInstruction {
        let pc = self.pc;
        let mode = self.instr_mode;
        let instruction_size = match mode {
            InstructionMode::Normal => INSN_SIZE,
            InstructionMode::Short => SHORT_INSN_SIZE,
        };
        if self.sys.translating() {
            // The cache is keyed by virtual address; translated fetches skip it.
            return self.read_and_decode_instruction(pc, mode);
        }
        let start_generation = self.bus.instruction_cache_generation(pc);
        let end_generation = self
            .bus
            .instruction_cache_generation(pc.wrapping_add(instruction_size - 1));
        if let (Some(start_generation), Some(end_generation)) = (start_generation, end_generation) {
            let cache_len = self.instruction_cache.as_ref().unwrap().len();
            let index = Self::instruction_cache_index(pc, cache_len);
            if let Some(entry) = self.instruction_cache.as_ref().unwrap()[index].filter(|entry| {
                entry.pc == pc
                    && entry.mode == mode
                    && entry.start_generation == start_generation
                    && entry.end_generation == end_generation
            }) {
                self.instruction_cache_hits += 1;
                return entry.instruction;
            }
            self.instruction_cache_misses += 1;
            self.instruction_cache_fills += 1;
            let instruction = self.read_and_decode_instruction(pc, mode);
            self.instruction_cache.as_mut().unwrap()[index] = Some(CachedInstruction {
                pc,
                mode,
                start_generation,
                end_generation,
                instruction,
            });
            instruction
        } else {
            self.read_and_decode_instruction(pc, mode)
        }
    }

    fn read_and_decode_instruction(
        &mut self,
        pc: u32,
        mode: InstructionMode,
    ) -> PendingInstruction {
        match mode {
            InstructionMode::Normal => {
                PendingInstruction::Normal(Self::decode(self.fetch_u32_at(pc)))
            }
            InstructionMode::Short => {
                PendingInstruction::Short(Self::decode_short(self.fetch_u16_at(pc)))
            }
        }
    }

    pub(crate) fn instruction_cache_stats(&self) -> Option<(u128, u128)> {
        self.instruction_cache
            .as_ref()
            .map(|_| (self.instruction_cache_hits, self.instruction_cache_misses))
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
            0x21 => Instruction::Jalr { rd },
            0x22 => Instruction::Jalrs { rd },
            0x23 => Instruction::Syscall { imm: imm_u16 },
            0x24 => Instruction::Mfsr { rd, sr: imm_u16 },
            0x25 => Instruction::Mtsr {
                rs: rs1,
                sr: imm_u16,
            },
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
            Instruction::Jalr { rd } => format!("JALR R{}", rd),
            Instruction::Jalrs { rd } => format!("JALRS R{}", rd),
            Instruction::Cpuid => "CPUID".to_string(),
            Instruction::Halt => "HALT".to_string(),
            Instruction::Iret => "IRET".to_string(),
            Instruction::Syscall { imm } => format_syscall(imm),
            Instruction::Mfsr { rd, sr } => format!("MFSR R{}, {}", rd, format_sr(sr)),
            Instruction::Mtsr { rs, sr } => format!("MTSR {}, R{}", format_sr(sr), rs),
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
                let raw = self.read_debug_mem_u32_be(addr).unwrap_or(0);
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
                let raw = self.read_debug_mem_u16_be(addr).unwrap_or(0);
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
        self.read_mem_u32_be(addr)
    }

    pub fn read_u16_be(&mut self, addr: u32) -> u16 {
        self.read_debug_u16(addr)
    }

    pub fn read_u40(&mut self, addr: u32) -> u32 {
        self.read_mem_u32_be(addr)
    }

    fn read_debug_u16(&mut self, addr: u32) -> u16 {
        self.bus.set_access_source(BusAccessSource::Debugger);
        let hi = self.bus.read_u8(addr) as u16;
        let lo = self.bus.read_u8(addr.wrapping_add(1)) as u16;
        self.bus.set_access_source(BusAccessSource::Cpu);
        (hi << 8) | lo
    }

    fn add_signed(base: u32, offset: i16) -> u32 {
        base.wrapping_add((offset as i32) as u32)
    }

    /// Instructions that raise a privileged-instruction exception in U mode.
    fn is_privileged(insn: Instruction) -> bool {
        matches!(
            insn,
            Instruction::Halt
                | Instruction::Iret
                | Instruction::Mfsr { .. }
                | Instruction::Mtsr { .. }
        )
    }

    /// Base register, offset, width and kind of a Normal-mode load/store.
    fn data_access(insn: Instruction) -> Option<(u8, i16, u32, Access)> {
        match insn {
            Instruction::Ld { base, offset, .. } => Some((base, offset, 4, Access::Read)),
            Instruction::Ldh { base, offset, .. } => Some((base, offset, 2, Access::Read)),
            Instruction::Ldb { base, offset, .. } => Some((base, offset, 1, Access::Read)),
            Instruction::St { base, offset, .. } => Some((base, offset, 4, Access::Write)),
            Instruction::Sth { base, offset, .. } => Some((base, offset, 2, Access::Write)),
            Instruction::Stb { base, offset, .. } => Some((base, offset, 1, Access::Write)),
            _ => None,
        }
    }

    fn branch_target(next_pc: u32, offset: i16) -> u32 {
        next_pc.wrapping_add((offset as i32) as u32)
    }

    fn short_reg_to_gpr(sr: u8) -> usize {
        if sr < 15 { sr as usize } else { REG_LR }
    }

    fn read_short_reg(&self, sr: u8) -> u32 {
        self.read_reg(Self::short_reg_to_gpr(sr))
    }

    fn write_short_reg(&mut self, sr: u8, value: u32) {
        let _ = self.set_gpr(Self::short_reg_to_gpr(sr), value);
    }

    fn execute_normal(&mut self, insn: Instruction) {
        let instruction_pc = self.pc;
        let next_pc = self.pc.wrapping_add(INSN_SIZE);
        self.pc = next_pc;

        if self.sys.user && Self::is_privileged(insn) {
            self.enter_exception(Exception::PrivilegedInstruction, instruction_pc);
            return;
        }
        if let Some((base, offset, len, access)) = Self::data_access(insn) {
            let addr = Self::add_signed(self.read_reg(base as usize), offset);
            if !self.access_allowed(addr, len, access, instruction_pc) {
                return;
            }
        }

        match insn {
            Instruction::Nop => {}
            Instruction::Ld { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.sys.read_data(&mut self.bus, addr, 4);
                let _ = self.set_gpr(rd as usize, value);
            }
            Instruction::St { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.read_reg(rd as usize);
                self.sys.write_data(&mut self.bus, addr, 4, value);
            }
            Instruction::Syscall { imm } => {
                self.enter_exception(Exception::Syscall(imm), next_pc);
            }
            Instruction::Mfsr { rd, sr } => match self.sys.read_sr(sr, &self.reg) {
                Some(value) => {
                    let _ = self.set_gpr(rd as usize, value);
                }
                None => self.enter_exception(Exception::IllegalInstruction, instruction_pc),
            },
            Instruction::Mtsr { rs, sr } => {
                // STATUS may enable IRQs: observe deferred device IRQs first.
                self.bus.flush_relaxed_devices();
                let value = self.read_reg(rs as usize);
                if !self.sys.write_sr(sr, value, &mut self.reg) {
                    self.enter_exception(Exception::IllegalInstruction, instruction_pc);
                }
            }
            Instruction::Ldil { rd, imm } => {
                let current = self.read_reg(rd as usize);
                let value = (current & 0xFFFF_0000) | u32::from(imm);
                let _ = self.set_gpr(rd as usize, value);
            }
            Instruction::Ldih { rd, imm } => {
                let current = self.read_reg(rd as usize);
                let value = (current & 0x0000_FFFF) | (u32::from(imm) << 16);
                let _ = self.set_gpr(rd as usize, value);
            }
            Instruction::Add { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = self.read_reg(rs2 as usize);
                let _ = self.set_gpr(rd as usize, lhs.wrapping_add(rhs));
            }
            Instruction::Addi { rd, rs1, imm } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = (imm as i32) as u32;
                let _ = self.set_gpr(rd as usize, lhs.wrapping_add(rhs));
            }
            Instruction::Sub { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = self.read_reg(rs2 as usize);
                let _ = self.set_gpr(rd as usize, lhs.wrapping_sub(rhs));
            }
            Instruction::Slt { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize) as i32;
                let rhs = self.read_reg(rs2 as usize) as i32;
                let _ = self.set_gpr(rd as usize, u32::from(lhs < rhs));
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
                let _ = self.set_gpr(REG_LR, next_pc);
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
                let _ = self.set_gpr(REG_LR, next_pc);
                self.pc = Self::branch_target(next_pc, offset);
                self.instr_mode = InstructionMode::Short;
            }
            Instruction::Jrs { rd } => {
                self.pc = self.read_reg(rd as usize);
                self.instr_mode = InstructionMode::Short;
            }
            Instruction::Jalr { rd } => {
                let target = self.read_reg(rd as usize);
                let _ = self.set_gpr(REG_LR, next_pc);
                self.pc = target;
            }
            Instruction::Jalrs { rd } => {
                let target = self.read_reg(rd as usize);
                let _ = self.set_gpr(REG_LR, next_pc);
                self.pc = target;
                self.instr_mode = InstructionMode::Short;
            }
            Instruction::Cpuid => {
                let _ = self.set_gpr(REG_CPUID, CPU_ID);
                let _ = self.set_gpr(REG_FEATURES, CPU_FEATURES);
            }
            Instruction::Halt => {
                self.bus.flush_relaxed_devices();
                self.running = false;
            }
            Instruction::Iret => {
                self.bus.flush_relaxed_devices();
                if crate::devices::pec::pec::pec_trace_enabled() {
                    tracing::info!(
                        target: "pec_debug",
                        "[PEC-DBG] CPU IRET cpu={} pc=0x{:08X} epc=0x{:08X} cause={} running_cycles={} instructions={}",
                        self.trace_cpu_name(),
                        self.pc,
                        self.sys.epc,
                        self.sys.cause,
                        self.running_cycles,
                        self.instructions_executed
                    );
                }
                let (pc, mode) = self.sys.leave_trap(&mut self.reg);
                self.pc = pc;
                self.instr_mode = mode;
            }
            Instruction::And { rd, rs1, rs2 } => {
                let _ = self.set_gpr(
                    rd as usize,
                    self.read_reg(rs1 as usize) & self.read_reg(rs2 as usize),
                );
            }
            Instruction::Or { rd, rs1, rs2 } => {
                let _ = self.set_gpr(
                    rd as usize,
                    self.read_reg(rs1 as usize) | self.read_reg(rs2 as usize),
                );
            }
            Instruction::Xor { rd, rs1, rs2 } => {
                let _ = self.set_gpr(
                    rd as usize,
                    self.read_reg(rs1 as usize) ^ self.read_reg(rs2 as usize),
                );
            }
            Instruction::Sll { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                let _ = self.set_gpr(rd as usize, self.read_reg(rs1 as usize).wrapping_shl(sh));
            }
            Instruction::Srl { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                let _ = self.set_gpr(rd as usize, self.read_reg(rs1 as usize).wrapping_shr(sh));
            }
            Instruction::Sla { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                let _ = self.set_gpr(rd as usize, self.read_reg(rs1 as usize).wrapping_shl(sh));
            }
            Instruction::Sra { rd, rs1, rs2 } => {
                let sh = self.read_reg(rs2 as usize) & 0x1F;
                let value = self.read_reg(rs1 as usize) as i32;
                let _ = self.set_gpr(rd as usize, (value >> sh) as u32);
            }
            Instruction::Sltu { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = self.read_reg(rs2 as usize);
                let _ = self.set_gpr(rd as usize, u32::from(lhs < rhs));
            }
            Instruction::Mul { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize) as u64;
                let rhs = self.read_reg(rs2 as usize) as u64;
                let _ = self.set_gpr(rd as usize, lhs.wrapping_mul(rhs) as u32);
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
                let _ = self.set_gpr(rd as usize, result as u32);
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
                let _ = self.set_gpr(rd as usize, result as u32);
            }
            Instruction::Mulh { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize) as i32 as i64;
                let rhs = self.read_reg(rs2 as usize) as i32 as i64;
                let _ = self.set_gpr(rd as usize, ((lhs.wrapping_mul(rhs)) >> 32) as u32);
            }
            Instruction::Divu { rd, rs1, rs2 } => {
                let lhs = self.read_reg(rs1 as usize);
                let rhs = self.read_reg(rs2 as usize);
                let result = if rhs == 0 { 0 } else { lhs / rhs };
                let _ = self.set_gpr(rd as usize, result);
            }
            Instruction::Ldb { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.sys.read_data(&mut self.bus, addr, 1);
                let _ = self.set_gpr(rd as usize, value);
            }
            Instruction::Ldh { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.sys.read_data(&mut self.bus, addr, 2);
                let _ = self.set_gpr(rd as usize, value);
            }
            Instruction::Stb { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.read_reg(rd as usize) as u8;
                self.sys
                    .write_data(&mut self.bus, addr, 1, u32::from(value));
            }
            Instruction::Sth { rd, base, offset } => {
                let addr = Self::add_signed(self.read_reg(base as usize), offset);
                let value = self.read_reg(rd as usize);
                self.sys.write_data(&mut self.bus, addr, 2, value & 0xFFFF);
            }
            Instruction::Unknown(raw) => {
                tracing::error!(
                    pc = format_args!("0x{instruction_pc:08X}"),
                    raw = format_args!("0x{raw:08X}"),
                    "Illegal instruction"
                );
                self.enter_exception(Exception::IllegalInstruction, instruction_pc);
            }
        }
    }

    fn execute_short(&mut self, insn: ShortInstruction) {
        let instruction_pc = self.pc;
        let next_pc = self.pc.wrapping_add(SHORT_INSN_SIZE);
        self.pc = next_pc;

        let data_access = match insn {
            ShortInstruction::Ld { rs1, .. } => Some((self.read_short_reg(rs1), Access::Read)),
            ShortInstruction::St { rd, .. } => Some((self.read_short_reg(rd), Access::Write)),
            _ => None,
        };
        if let Some((addr, access)) = data_access {
            if !self.access_allowed(addr, 4, access, instruction_pc) {
                return;
            }
        }

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
                let value = self.sys.read_data(&mut self.bus, addr, 4);
                self.write_short_reg(rd, value);
            }
            ShortInstruction::St { rd, rs1 } => {
                let addr = self.read_short_reg(rd);
                let value = self.read_short_reg(rs1);
                self.sys.write_data(&mut self.bus, addr, 4, value);
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
                let _ = self.set_gpr(REG_LR, next_pc);
                self.pc = Self::branch_target(next_pc, offset);
            }
            ShortInstruction::Ldi { rd, imm } => {
                self.write_short_reg(rd, u32::from(imm));
            }
            ShortInstruction::Ret => {
                self.instr_mode = InstructionMode::Normal;
            }
            ShortInstruction::Unknown(raw) => {
                tracing::error!(
                    pc = format_args!("0x{instruction_pc:08X}"),
                    raw = format_args!("0x{raw:04X}"),
                    "Illegal short instruction"
                );
                self.enter_exception(Exception::IllegalInstruction, instruction_pc);
            }
        }
    }

    pub fn return_state_text(&mut self) -> String {
        let pc = self.pc;
        let decoded = self.decode_current();
        let mut txt = format!(
            "PC=0x{:08X} MODE={:?} OP={}\n",
            pc, self.instr_mode, decoded.text
        );
        txt.push_str(&format!(
            "EPC=0x{:08X} CAUSE=0x{:08X} STATUS=0x{:08X} (IRQ {}, {} mode)\n",
            self.sys.epc,
            self.sys.cause,
            self.sys.status(),
            if self.sys.ie { "enabled" } else { "disabled" },
            if self.sys.user { "user" } else { "supervisor" },
        ));
        for i in 0..32 {
            txt.push_str(&format!(" R{:<2}=0x{:08X}", i, self.read_reg(i)));
            if i % 4 == 3 {
                txt.push('\n');
            }
        }
        txt
    }

    pub fn step_once(&mut self) -> bool {
        if !self.running {
            self.tick();
            if !self.running {
                return false;
            }
        }
        let start = self.cycles;
        while self.running && (self.cycles == start || self.state != CpuState::Fetch) {
            self.tick();
        }
        // Complete one full instruction, returning when its commit returns the FSM to FETCH.
        if self.cycles == start {
            while self.running && self.state != CpuState::Fetch {
                self.tick();
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

    /// Test hook: disable Bus optimizations based on timing-insensitive devices.
    #[doc(hidden)]
    pub fn set_timing_insensitive_fast_paths(&mut self, enabled: bool) {
        self.bus.set_timing_insensitive_fast_paths(enabled);
    }

    /// Cycles skipped by spin-loop fast-forward since construction or reset.
    #[doc(hidden)]
    pub fn spin_skipped_cycles(&self) -> u128 {
        self.spin.skipped_cycles
    }

    /// Host-side state changes are not CPU execution; an earlier loop-head
    /// snapshot must not be compared across them.
    fn forget_spin(&mut self) {
        self.spin.probes = [None; SPIN_PROBES];
    }

    #[doc(hidden)]
    pub fn set_spin_fast_forward(&mut self, enabled: bool) {
        self.spin = SpinDetector::new(enabled);
    }

    fn spin_state(&self) -> SpinState {
        SpinState {
            reg: self.reg,
            pc: self.pc,
            instr_mode: self.instr_mode,
            sys: self.sys,
            irq_pending: self.irq_pending,
            irq_pending_number: self.irq_pending_number,
            irq_line: self.irq_line,
            external_irq_line: self.external_irq_line,
            external_irq_number: self.external_irq_number,
            irqc_line: self.irqc_line,
            irqc_number: self.irqc_number,
        }
    }

    fn spin_probe(&self, quiet_until: Option<u128>) -> SpinProbe {
        SpinProbe {
            state: self.spin_state(),
            cycles: self.cycles,
            instructions: self.instructions_executed,
            cache_hits: self.instruction_cache_hits,
            effectful_accesses: self.bus.effectful_accesses(),
            fills: self.instruction_cache_fills,
            quiet_until,
        }
    }

    /// Called at instruction boundaries of `run`. Returns true after skipping
    /// whole iterations of a proven spin loop.
    fn skip_spin(&mut self, end_cycles: u128) -> bool {
        let pc = self.pc;
        let backward = pc <= self.spin.last_pc;
        self.spin.last_pc = pc;
        if !backward {
            return false;
        }
        let slot = self.spin.slot(pc);
        let Some(probe) = self.spin.probes[slot] else {
            self.spin.probes[slot] = Some(self.spin_probe(None));
            return false;
        };
        let repeated = self.cycles > probe.cycles
            && probe.effectful_accesses == self.bus.effectful_accesses()
            && probe.fills == self.instruction_cache_fills
            && probe.state == self.spin_state()
            && !(self.sys.ie && self.irq_pending);
        if !repeated {
            self.spin.probes[slot] = Some(self.spin_probe(None));
            return false;
        }
        let Some(quiet_until) = probe.quiet_until else {
            // Matched once: measure the window this next iteration must fit.
            let quiet = u128::from(self.bus.quiet_cycles());
            self.spin.probes[slot] = Some(self.spin_probe(Some(self.cycles + quiet)));
            return false;
        };
        if self.cycles > quiet_until {
            self.spin.probes[slot] = Some(self.spin_probe(None));
            return false;
        }
        let period = self.cycles - probe.cycles;
        let room = u128::from(self.bus.quiet_cycles()).min(end_cycles - self.cycles);
        let iterations = room / period;
        if iterations == 0 {
            self.spin.probes[slot] = Some(self.spin_probe(None));
            return false;
        }
        let skipped = iterations * period;
        self.spin.skipped_cycles += skipped;
        self.bus
            .advance_quiet_cycles(skipped.min(u128::from(u64::MAX)) as u64);
        self.cycles += skipped;
        self.running_cycles += skipped;
        self.instructions_executed +=
            iterations * (self.instructions_executed - probe.instructions);
        self.instruction_cache_hits +=
            iterations * (self.instruction_cache_hits - probe.cache_hits);
        self.sync_register_block();
        self.spin.probes[slot] = Some(self.spin_probe(None));
        true
    }

    fn run_inner<const PROFILE: bool>(&mut self, max_cycles: usize) {
        self.relaxed_run = self.relaxed_devices && !PROFILE;
        let start_cycles = self.cycles;
        let end_cycles = start_cycles.saturating_add(max_cycles as u128);
        while self.cycles < end_cycles {
            if !PROFILE
                && self.spin.enabled
                && self.running
                && self.state == CpuState::Fetch
                && self.skip_spin(end_cycles)
            {
                continue;
            }
            #[cfg(feature = "jit")]
            if !PROFILE
                && self.jit.is_some()
                && self.running
                && self.state == CpuState::Fetch
                && self.instr_mode == InstructionMode::Normal
                && self.run_jit_block(end_cycles - self.cycles)
            {
                continue;
            }
            #[cfg(feature = "jit")]
            if let Some(jit) = &mut self.jit {
                jit.record_fallback_cycle();
            }
            self.tick_with_profile::<PROFILE>();
            if self.relaxed_run && !self.running {
                self.bus.flush_relaxed_devices();
                let output = self.irq_controller.borrow().irq_output();
                self.irqc_line = output.valid;
                self.irqc_number = output.number;
                self.refresh_irq_input();
                if self.accept_pending_irq() {
                    self.sync_register_block();
                }
            }
            if !self.running && !self.bus.has_pending_dma() {
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
        self.bus.flush_relaxed_devices();
        self.relaxed_run = false;
    }
}
