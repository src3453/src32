//! Native blocks with a conservative per-cycle path and a bus-free idle path.
//! Generated code only accesses the explicit [u32; 32] register array.
use super::*;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use std::any::Any;
use std::cell::Cell;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

const MAX_BLOCKS: usize = 1024;
/// Dispatch visits before a block's batch tier is compiled. Compiling briefly
/// warm startup code cost more than it saved on real guests.
const HOT_THRESHOLD: u32 = 128;
/// Hot-block dispatches that could not batch (DMA, budget or window edges)
/// before the conservative per-cycle tier is compiled. That tier barely beats
/// the interpreter but cost most of the compile time when built eagerly.
const STRICT_THRESHOLD: u32 = 32;

type NativeBlock = unsafe extern "C" fn(*mut Runtime, u64);
type NativeBatch = unsafe extern "C" fn(*mut u32, u32, *mut BatchContext) -> u32;

const BATCH_CONTINUE: u32 = 0;
const BATCH_STOP_AFTER: u32 = 1;
const BATCH_STOP_BEFORE: u32 = 2;

#[derive(Clone, Copy)]
struct NativeFunctions {
    strict: Option<NativeBlock>,
    batch: Option<NativeBatch>,
}

#[derive(Clone, Copy)]
struct BlockInstruction {
    pc: u32,
    instruction: Instruction,
}

/// Dispatch keys are guest PCs; SipHash dominated block dispatch.
#[derive(Default)]
struct PcHasher(u64);

impl Hasher for PcHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(byte)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_u32(&mut self, value: u32) {
        self.0 = u64::from(value).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }

    fn finish(&self) -> u64 {
        // Fold the well-mixed high half into the bucket-selecting low bits.
        self.0 ^ (self.0 >> 32)
    }
}

type PcMap<V> = HashMap<u32, V, BuildHasherDefault<PcHasher>>;

/// Proof that every instruction of a block was a valid Cached hit: no cache
/// slot was written since, and every generation unit it covers is unchanged.
#[derive(Clone, Copy)]
struct BatchStamp {
    fills: u64,
    units: [(u64, u32); MAX_STAMP_UNITS],
    unit_count: usize,
}

const MAX_STAMP_UNITS: usize = 2;

type Lookup = (
    NativeFunctions,
    *const BlockInstruction,
    usize,
    *const Cell<Option<BatchStamp>>,
);

struct Entry {
    instructions: Box<[BlockInstruction]>,
    // Boxed so the pointer handed out by lookup stays put if the map grows.
    stamp: Box<Cell<Option<BatchStamp>>>,
    visits: u32,
    strict_requests: u32,
    function: Option<NativeFunctions>,
    failed: bool,
    invalid: bool,
}

pub(super) struct JitEngine {
    module: Option<JITModule>,
    entries: PcMap<Entry>,
    definitions: usize,
    max_instructions: usize,
    /// Visits before the batch tier compiles (default HOT_THRESHOLD).
    hot_threshold: u32,
    /// Declined batch dispatches before the strict tier compiles (default
    /// STRICT_THRESHOLD); 0 compiles it together with the batch tier.
    strict_threshold: u32,
    stats: JitStats,
}

impl JitEngine {
    pub(super) fn new(max_instructions: usize) -> Self {
        let module = match Self::create_module() {
            Ok(module) => Some(module),
            Err(error) => {
                tracing::warn!("CPU JIT unavailable; using Cached execution: {error}");
                None
            }
        };
        Self {
            stats: JitStats {
                compile_failures: u64::from(module.is_none()),
                ..JitStats::default()
            },
            module,
            entries: PcMap::default(),
            definitions: 0,
            hot_threshold: HOT_THRESHOLD,
            strict_threshold: STRICT_THRESHOLD,
            max_instructions: max_instructions
                .clamp(2, crate::cpu_select::MAX_JIT_BLOCK_SIZE as usize),
        }
    }

    fn create_module() -> Result<JITModule, String> {
        // The IR verifier dominated compile time; keep it for debug builds.
        let verifier = if cfg!(debug_assertions) {
            "true"
        } else {
            "false"
        };
        let mut flags = settings::builder();
        flags
            .set("enable_verifier", verifier)
            .map_err(|e| e.to_string())?;
        // Cranelift 0.117's PIC JIT allocates GOT and PLT separately, then
        // unwraps their distance as i32. Windows can place them >2 GiB apart.
        // Non-PIC code uses full-width helper addresses and needs no GOT/PLT.
        // with_flags forces is_pic=true even if passed false: use with_isa.
        flags.set("is_pic", "false").map_err(|e| e.to_string())?;
        flags
            .set("use_colocated_libcalls", "false")
            .map_err(|e| e.to_string())?;
        let isa = cranelift_native::builder()
            .map_err(|e| e.to_string())?
            .finish(settings::Flags::new(flags))
            .map_err(|e| e.to_string())?;
        let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        builder.symbol("src32_prepare", prepare as *const u8);
        builder.symbol("src32_finish", finish as *const u8);
        builder.symbol("src32_batch_memory", batch_memory as *const u8);
        Ok(JITModule::new(builder))
    }

    // Called only outside generated code. No function pointer escapes the CPU.
    pub(super) fn reset(&mut self) {
        self.entries.clear();
        if let Some(module) = self.module.take() {
            // SAFETY: all native calls have returned, and dispatch was cleared.
            unsafe { module.free_memory() };
        }
        self.module = match Self::create_module() {
            Ok(module) => Some(module),
            Err(error) => {
                tracing::warn!("CPU JIT cache recreation failed; using Cached: {error}");
                None
            }
        };
        self.definitions = 0;
        self.stats.native_code_bytes = 0;
        self.stats.cache_resets += 1;
        if self.module.is_none() {
            self.stats.compile_failures += 1;
        }
    }

    pub(super) fn stats(&self) -> JitStats {
        JitStats {
            cached_blocks: self
                .entries
                .values()
                .filter(|e| e.function.is_some() && !e.invalid)
                .count(),
            ..self.stats
        }
    }

    pub(super) fn set_tier_thresholds(&mut self, hot: u32, strict: u32) {
        self.hot_threshold = hot.max(1);
        self.strict_threshold = strict;
    }

    pub(super) fn record_fallback_cycle(&mut self) {
        self.stats.fallback_cycles += 1;
    }

    fn lookup(&mut self, bus: &Bus, pc: u32) -> Option<Lookup> {
        self.module.as_ref()?;
        // Hot path: an established block whose slow path would only count the
        // visit (no compile, prune or reset is due).
        if self.entries.len() < MAX_BLOCKS && self.definitions < MAX_BLOCKS {
            if let Some(entry) = self.entries.get_mut(&pc) {
                if entry.visits > self.hot_threshold
                    && !entry.invalid
                    && !entry.failed
                    && entry.instructions.len() >= 2
                {
                    if let Some(function) = entry.function {
                        entry.visits = entry.visits.saturating_add(1);
                        return Some((
                            function,
                            entry.instructions.as_ptr(),
                            entry.instructions.len(),
                            &*entry.stamp,
                        ));
                    }
                }
            }
        }
        if self.entries.get(&pc).is_some_and(|entry| entry.invalid) {
            self.entries.remove(&pc);
        }
        // Bound both live dispatch entries and total executable allocations,
        // including invalidated definitions retained in the module.
        if self.entries.len() >= MAX_BLOCKS {
            // A long cold code path must not repeatedly throw away hot native
            // blocks just because its interpreter-only entries filled the map.
            self.entries
                .retain(|_, entry| entry.function.is_some() || entry.failed);
        }
        if self.entries.len() >= MAX_BLOCKS || self.definitions >= MAX_BLOCKS {
            self.reset();
            self.module.as_ref()?;
        }
        let entry = self.entries.entry(pc).or_insert_with(|| Entry {
            instructions: preview(bus, pc, self.max_instructions).into_boxed_slice(),
            stamp: Box::new(Cell::new(None)),
            strict_requests: 0,
            visits: 0,
            function: None,
            failed: false,
            invalid: false,
        });
        // A one-instruction native block cannot amortize dispatch/ABI costs.
        if entry.instructions.len() < 2 || entry.failed {
            return None;
        }
        entry.visits = entry.visits.saturating_add(1);
        let mut eager_strict = false;
        if entry.function.is_none() && entry.visits == self.hot_threshold {
            let start = Instant::now();
            let serial = self.definitions;
            let module = self.module.as_mut().unwrap();
            let result = match compile_native(module, Tier::Batch, serial, &entry.instructions) {
                Ok((Native::Batch(batch), bytes)) => Ok((
                    NativeFunctions {
                        strict: None,
                        batch: Some(batch),
                    },
                    bytes,
                )),
                Ok(_) => unreachable!("batch tier compiles a batch function"),
                Err(error) => {
                    tracing::warn!(
                        "CPU JIT batch at {pc:#010x} failed; using conservative native code: {error}"
                    );
                    self.stats.compile_failures += 1;
                    compile_native(module, Tier::Strict, serial, &entry.instructions).map(
                        |(native, bytes)| {
                            let Native::Strict(strict) = native else {
                                unreachable!("strict tier compiles a strict function")
                            };
                            (
                                NativeFunctions {
                                    strict: Some(strict),
                                    batch: None,
                                },
                                bytes,
                            )
                        },
                    )
                }
            };
            self.stats.compile_time += start.elapsed();
            self.definitions += 1;
            match result {
                Ok((functions, bytes)) => {
                    entry.function = Some(functions);
                    self.stats.compilations += 1;
                    self.stats.batch_compilations += u64::from(functions.batch.is_some());
                    self.stats.native_code_bytes += bytes;
                    eager_strict = self.strict_threshold == 0 && functions.strict.is_none();
                }
                Err(error) => {
                    entry.failed = true;
                    self.stats.compile_failures += 1;
                    tracing::warn!("CPU JIT block at {pc:#010x} failed; using Cached: {error}");
                }
            }
        }
        if eager_strict {
            self.compile_strict(pc);
        }
        let entry = self.entries.get(&pc)?;
        Some((
            entry.function?,
            entry.instructions.as_ptr(),
            entry.instructions.len(),
            &*entry.stamp,
        ))
    }
}

impl JitEngine {
    /// Count a hot block dispatch that could not batch, and compile the
    /// conservative tier once that keeps happening. Never resets the module,
    /// so pointers from the preceding lookup stay valid.
    fn request_strict(&mut self, pc: u32) -> Option<NativeBlock> {
        self.module.as_ref()?;
        let entry = self.entries.get_mut(&pc)?;
        let functions = entry.function.as_mut()?;
        if let Some(strict) = functions.strict {
            return Some(strict);
        }
        entry.strict_requests = entry.strict_requests.saturating_add(1);
        if entry.strict_requests < self.strict_threshold {
            return None;
        }
        self.compile_strict(pc)
    }

    /// Compile the conservative tier for an entry that has native functions.
    fn compile_strict(&mut self, pc: u32) -> Option<NativeBlock> {
        let module = self.module.as_mut()?;
        let entry = self.entries.get_mut(&pc)?;
        let functions = entry.function.as_mut()?;
        if entry.failed || self.definitions >= MAX_BLOCKS {
            return None;
        }
        let start = Instant::now();
        let result = compile_native(module, Tier::Strict, self.definitions, &entry.instructions);
        self.stats.compile_time += start.elapsed();
        self.definitions += 1;
        match result {
            Ok((Native::Strict(strict), bytes)) => {
                functions.strict = Some(strict);
                self.stats.compilations += 1;
                self.stats.native_code_bytes += bytes;
                Some(strict)
            }
            Ok(_) => unreachable!("strict tier compiles a strict function"),
            Err(error) => {
                // Do not retry on every later dispatch.
                entry.failed = true;
                self.stats.compile_failures += 1;
                tracing::warn!("CPU JIT block at {pc:#010x} failed; using Cached: {error}");
                None
            }
        }
    }
}

impl Drop for JitEngine {
    fn drop(&mut self) {
        self.entries.clear();
        if let Some(module) = self.module.take() {
            // SAFETY: a CPU cannot be dropped while its exclusive run call is active.
            unsafe { module.free_memory() };
        }
    }
}

fn supported(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Nop
            | Instruction::Add { .. }
            | Instruction::Addi { .. }
            | Instruction::Sub { .. }
            | Instruction::Slt { .. }
            | Instruction::Sltu { .. }
            | Instruction::And { .. }
            | Instruction::Or { .. }
            | Instruction::Xor { .. }
            | Instruction::Sll { .. }
            | Instruction::Srl { .. }
            | Instruction::Sla { .. }
            | Instruction::Sra { .. }
            | Instruction::Ldil { .. }
            | Instruction::Ldih { .. }
            | Instruction::Beq { .. }
            | Instruction::Bne { .. }
            | Instruction::Jmp { .. }
            | Instruction::Jal { .. }
            | Instruction::Jr { .. }
            | Instruction::Jalr { .. }
            | Instruction::Cpuid
            | Instruction::Mul { .. }
            | Instruction::Mulh { .. }
            | Instruction::Div { .. }
            | Instruction::Divu { .. }
            | Instruction::Mod { .. }
            | Instruction::Ld { .. }
            | Instruction::Ldb { .. }
            | Instruction::Ldh { .. }
            | Instruction::St { .. }
            | Instruction::Stb { .. }
            | Instruction::Sth { .. }
    )
}

/// Base register, offset, width and direction of a supported memory access.
fn memory_access(instruction: Instruction) -> Option<(u8, i16, u32, bool)> {
    match instruction {
        Instruction::Ld { base, offset, .. } => Some((base, offset, 4, false)),
        Instruction::Ldh { base, offset, .. } => Some((base, offset, 2, false)),
        Instruction::Ldb { base, offset, .. } => Some((base, offset, 1, false)),
        Instruction::St { base, offset, .. } => Some((base, offset, 4, true)),
        Instruction::Sth { base, offset, .. } => Some((base, offset, 2, true)),
        Instruction::Stb { base, offset, .. } => Some((base, offset, 1, true)),
        _ => None,
    }
}

/// Cached-mode cycles from issue to commit (MUL/MULH 3, DIV/DIVU/MOD 16).
fn latency(instruction: Instruction) -> u64 {
    let pending = PendingInstruction::Normal(instruction);
    if Cpu::is_mul(pending) {
        3
    } else if Cpu::is_div(pending) {
        16
    } else {
        1
    }
}

/// Multi-cycle instructions stall in Cached's FSM with device progress between
/// issue and commit; the conservative path hands them to the interpreter.
/// Memory accesses use the interpreter's bus path there as well.
fn interpreter_only_in_strict(instruction: Instruction) -> bool {
    latency(instruction) != 1 || memory_access(instruction).is_some()
}

fn ends_block(instruction: Instruction) -> bool {
    matches!(
        instruction,
        Instruction::Beq { .. }
            | Instruction::Bne { .. }
            | Instruction::Jmp { .. }
            | Instruction::Jal { .. }
            | Instruction::Jr { .. }
            | Instruction::Jalr { .. }
    )
}

fn preview(bus: &Bus, mut pc: u32, max_instructions: usize) -> Vec<BlockInstruction> {
    let mut instructions = Vec::new();
    for _ in 0..max_instructions {
        // No speculative MMIO reads. Both APIs must opt in for the entire word.
        let raw = (0..4).try_fold(0u32, |word, offset| {
            let address = pc.wrapping_add(offset);
            bus.instruction_cache_generation(address)?;
            Some((word << 8) | u32::from(bus.peek_instruction_u8(address)?))
        });
        let Some(raw) = raw else { break };
        let instruction = Cpu::decode(raw);
        if !supported(instruction) {
            break;
        }
        instructions.push(BlockInstruction { pc, instruction });
        if ends_block(instruction) {
            break;
        }
        pc = pc.wrapping_add(4);
    }
    instructions
}

/// State shared by a native batch and its memory helper.
struct BatchContext {
    cpu: *mut Cpu,
    instructions: *const BlockInstruction,
    /// Instructions executed when the helper stopped the batch early.
    executed: Option<u32>,
    /// Batch cycles already handed to the Bus before a memory access.
    deferred: u64,
    /// Code units of a stamped block; `None` makes every store stop the batch.
    stamp: Option<BatchStamp>,
    panic: Option<Box<dyn Any + Send>>,
}

struct Runtime {
    cpu: *mut Cpu,
    instructions: *const BlockInstruction,
    entry_pc: u32,
    panic: Option<Box<dyn Any + Send>>,
}

impl Cpu {
    pub(super) fn run_jit_block(&mut self, budget: u128) -> bool {
        let start_cycles = self.cycles;
        // Consecutive batches stay in native registers: between them only
        // plain-memory accesses happen, which cannot touch the CPU register
        // block or change IRQC output without stopping the batch. The block
        // is synchronized once when the chain ends.
        let mut chained = false;
        loop {
            if !self.running
                || self.state != CpuState::Fetch
                || self.instr_mode != InstructionMode::Normal
            {
                break;
            }
            let remaining = budget - (self.cycles - start_cycles);
            if remaining == 0 {
                break;
            }
            let Some((functions, instructions, length, stamp)) =
                self.jit.as_mut().unwrap().lookup(&self.bus, self.pc)
            else {
                break;
            };
            // SAFETY: lookup metadata remains allocated until the next
            // lookup/reset. Neither the preflight nor native batch can mutate
            // the JIT cache.
            let items = unsafe { std::slice::from_raw_parts(instructions, length) };
            if functions.batch.is_some_and(|batch| {
                // SAFETY: same lifetime as `items`; the stamp is a boxed Cell.
                self.run_native_idle_batch(batch, items, unsafe { &*stamp }, remaining, chained)
            }) {
                chained = true;
                continue;
            }
            if chained {
                // Usually the deferred window ran out; let the next dispatch
                // start a fresh one instead of counting a strict-tier decline.
                break;
            }
            let strict = match functions.strict {
                Some(strict) => Some(strict),
                None => self.jit.as_mut().unwrap().request_strict(self.pc),
            };
            let Some(strict) = strict else {
                break;
            };
            self.run_strict_block(strict, instructions, remaining);
            return self.cycles != start_cycles;
        }
        if chained {
            self.sync_register_block();
        }
        self.cycles != start_cycles
    }

    fn run_strict_block(
        &mut self,
        strict: NativeBlock,
        instructions: *const BlockInstruction,
        budget: u128,
    ) {
        let mut runtime = Runtime {
            cpu: self,
            instructions,
            entry_pc: self.pc,
            panic: None,
        };
        // SAFETY: the module and instruction array remain owned by this CPU for
        // the whole call. Helpers never evict/reset them, and all borrows from
        // lookup have ended. Native code uses only registers returned by prepare.
        unsafe { strict(&mut runtime, budget.min(u128::from(u64::MAX)) as u64) };
        if let Some(panic) = runtime.panic {
            resume_unwind(panic);
        }
    }

    fn run_native_idle_batch(
        &mut self,
        function: NativeBatch,
        items: &[BlockInstruction],
        stamp: &Cell<Option<BatchStamp>>,
        budget: u128,
        chained: bool,
    ) -> bool {
        if !self.relaxed_run || budget < 2 {
            return false;
        }
        // Longest prefix whose Cached cycle cost fits the bus-free window.
        let prefix = |idle: u64| {
            let cycle_limit = budget.min(u128::from(idle)) as u64;
            let mut cycles = 0;
            let mut limit = 0;
            for item in items {
                let next = cycles + latency(item.instruction);
                if next > cycle_limit {
                    break;
                }
                cycles = next;
                limit += 1;
            }
            (cycles, limit)
        };
        let (mut cycles, mut limit) = prefix(self.bus.native_idle_budget());
        if (limit == 0 || cycles < 2) && !chained && self.bus.has_deferred_cycles() {
            // Timing-insensitive accesses no longer end the deferred window;
            // catch devices up at this instruction boundary, as the relaxed
            // interpreter does at its window limit, and open a new window.
            // IRQ inputs are sampled below, after the flush.
            self.bus.flush_relaxed_devices();
            (cycles, limit) = prefix(self.bus.native_idle_budget());
        }
        if limit == 0 || cycles < 2 {
            return false;
        }
        if !chained {
            self.sync_from_register_block();
            let output = self.irq_controller.borrow().irq_output();
            self.irqc_line = output.valid;
            self.irqc_number = output.number;
            self.refresh_irq_input();
        }
        if self.pc != items[0].pc
            || self.instr_mode != InstructionMode::Normal
            || (self.irq_enable && self.irq_pending)
        {
            return false;
        }
        // Only cache HITS are eligible: cold fetches have bus/IRQ side effects.
        // No device progresses, no CPU store executes, and exclusive run owns
        // the bus, so these generations cannot change during the native call.
        let fills = self.instruction_cache_fills;
        let stamped = stamp.get().is_some_and(|stamp| {
            stamp.fills == fills
                && stamp.units[..stamp.unit_count]
                    .iter()
                    .all(|&(generation, address)| {
                        self.bus.instruction_cache_generation(address) == Some(generation)
                    })
        });
        let count = if stamped {
            limit
        } else {
            self.validate_native_batch(items, stamp, limit)
        };
        if count != limit {
            return false;
        }
        let registers = self.reg.as_mut_ptr();
        let mut context = BatchContext {
            cpu: self,
            instructions: items.as_ptr(),
            executed: None,
            deferred: 0,
            stamp: stamp.get(),
            panic: None,
        };
        // SAFETY: generated code touches only this live register array and
        // calls the memory helper, which reaches the CPU through `context`;
        // arithmetic cannot trap. count is a validated prefix that cannot
        // cross a branch or event boundary.
        let next_pc = unsafe { function(registers, count as u32, &mut context) };
        if let Some(panic) = context.panic {
            resume_unwind(panic);
        }
        let executed = context.executed.map_or(count, |executed| executed as usize);
        let cycles: u64 = items[..executed]
            .iter()
            .map(|item| latency(item.instruction))
            .sum();
        if executed == 0 {
            return false;
        }
        self.bus.defer_native_idle_cycles(cycles - context.deferred);
        self.pc = next_pc;
        self.cycles += u128::from(cycles);
        self.running_cycles += u128::from(cycles);
        self.instructions_executed += executed as u128;
        self.instruction_cache_hits += executed as u128;
        let stats = &mut self.jit.as_mut().unwrap().stats;
        stats.native_instructions += executed as u128;
        stats.batched_instructions += executed as u64;
        stats.native_batches += 1;
        true
    }
}

impl Cpu {
    /// Count the leading instructions that are current Cached hits, checking
    /// the whole block so a fully valid block can be stamped for later visits.
    fn validate_native_batch(
        &self,
        items: &[BlockInstruction],
        stamp: &Cell<Option<BatchStamp>>,
        limit: usize,
    ) -> usize {
        let cache = self.instruction_cache.as_ref().unwrap();
        // Blocks normally sit in one generation unit (a RAM page); reuse its
        // token. Nothing can write memory during this validation.
        let mut span: Option<(u64, u32, u32)> = None;
        let mut units = [(0, 0); MAX_STAMP_UNITS];
        let mut unit_count = 0;
        let mut stampable = true;
        let mut generation = |address: u32| {
            if let Some((generation, first, last)) = span {
                if (first..=last).contains(&address) {
                    return Some(generation);
                }
            }
            span = self.bus.instruction_cache_generation_span(address);
            if let Some((generation, first, _)) = span {
                if unit_count < MAX_STAMP_UNITS {
                    units[unit_count] = (generation, first);
                    unit_count += 1;
                } else {
                    stampable = false;
                }
            }
            span.map(|(generation, _, _)| generation)
        };
        let hits = items
            .iter()
            .take_while(|item| {
                let index = Self::instruction_cache_index(item.pc, cache.len());
                cache[index].is_some_and(|entry| {
                    entry.pc == item.pc
                        && entry.mode == InstructionMode::Normal
                        && entry.instruction == PendingInstruction::Normal(item.instruction)
                        && generation(item.pc) == Some(entry.start_generation)
                        && generation(item.pc.wrapping_add(3)) == Some(entry.end_generation)
                })
            })
            .count();
        stamp.set((hits == items.len() && stampable).then_some(BatchStamp {
            fills: self.instruction_cache_fills,
            units,
            unit_count,
        }));
        hits.min(limit)
    }
}

// Neither Rust panics nor Rust object layouts cross the native ABI. A panic is
// transported to the enclosing Rust run call after generated code has returned.
unsafe extern "C" fn prepare(runtime: *mut Runtime, index: u32, previous_pc: u32) -> *mut u32 {
    // SAFETY: only generated blocks call this with their live stack Runtime and
    // a statically bounded instruction index. Copy metadata before mutating CPU.
    let runtime = unsafe { &mut *runtime };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let expected = unsafe { *runtime.instructions.add(index as usize) };
        let cpu = unsafe { &mut *runtime.cpu };
        // Fuse the previous commit with the next cycle boundary so a native
        // instruction normally needs only one Rust ABI call, not two.
        if index != 0 {
            commit_native(cpu, previous_pc);
        }
        if !cpu.running
            || cpu.state != CpuState::Fetch
            || cpu.pc != expected.pc
            || cpu.instr_mode != InstructionMode::Normal
        {
            return std::ptr::null_mut();
        }
        if !cpu.begin_cached_cycle::<false>() {
            return std::ptr::null_mut();
        }
        if cpu.pc != expected.pc
            || cpu.instr_mode != InstructionMode::Normal
            || cpu.state != CpuState::Fetch
        {
            cpu.execute_cached_cycle::<false>();
            cpu.jit.as_mut().unwrap().record_fallback_cycle();
            return std::ptr::null_mut();
        }
        // This retains Cached's cold-fetch bus effects and validates both page
        // generations AFTER device/DMA progress, on every instruction. Comparing
        // the decoded identity also covers instruction changes mid-block.
        let actual = cpu.fetch_cached_instruction();
        if actual == PendingInstruction::Normal(expected.instruction)
            && interpreter_only_in_strict(expected.instruction)
        {
            // Issue exactly like Cached; the stall cycles and commit continue
            // in the interpreter, and native execution resumes at a new entry.
            cpu.issue_cached_instruction::<false>(actual);
            cpu.sync_register_block();
            return std::ptr::null_mut();
        }
        if actual != PendingInstruction::Normal(expected.instruction) {
            let jit = cpu.jit.as_mut().unwrap();
            jit.entries.get_mut(&runtime.entry_pc).unwrap().invalid = true;
            jit.stats.invalidations += 1;
            jit.record_fallback_cycle();
            cpu.issue_cached_instruction::<false>(actual);
            cpu.sync_register_block();
            return std::ptr::null_mut();
        }
        cpu.reg.as_mut_ptr()
    }));
    match result {
        Ok(registers) => registers,
        Err(panic) => {
            runtime.panic = Some(panic);
            std::ptr::null_mut()
        }
    }
}

/// Perform one batched load/store exactly as the relaxed interpreter would:
/// device time up to and including this cycle is deferred first, so the Bus
/// flushes it at the access. Only plain memory is accessed here.
unsafe extern "C" fn batch_memory(context: *mut BatchContext, index: u32, cycles: u32) -> u32 {
    // SAFETY: only generated batches call this with their live context.
    let context = unsafe { &mut *context };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let item = unsafe { *context.instructions.add(index as usize) };
        let cpu = unsafe { &mut *context.cpu };
        let (base, offset, width, store) =
            memory_access(item.instruction).expect("batched memory instruction");
        let address = Cpu::add_signed(cpu.read_reg(usize::from(base)), offset);
        if !cpu.bus.is_timing_insensitive_range(address, width) {
            context.executed = Some(index);
            return BATCH_STOP_BEFORE;
        }
        let cycles = u64::from(cycles);
        cpu.bus.defer_native_idle_cycles(cycles - context.deferred);
        context.deferred = cycles;
        cpu.pc = item.pc;
        cpu.defer_register_sync = true;
        cpu.execute_normal(item.instruction);
        cpu.defer_register_sync = false;
        let faulted = cpu.bus.take_bus_error();
        debug_assert!(!faulted, "plain memory cannot fault");
        // The flush may have changed IRQC output (e.g. host input); the
        // interpreter would sample it at the next cycle, so return to Rust.
        let output = cpu.irq_controller.borrow().irq_output();
        let irq_changed = output.valid != cpu.irqc_line || output.number != cpu.irqc_number;
        let code_changed = store
            && !context.stamp.is_some_and(|stamp| {
                stamp.units[..stamp.unit_count]
                    .iter()
                    .all(|&(generation, address)| {
                        cpu.bus.instruction_cache_generation(address) == Some(generation)
                    })
            });
        if faulted || irq_changed || code_changed {
            context.executed = Some(index + 1);
            BATCH_STOP_AFTER
        } else {
            BATCH_CONTINUE
        }
    }));
    match result {
        Ok(status) => status,
        Err(panic) => {
            context.panic = Some(panic);
            context.executed = Some(index);
            BATCH_STOP_BEFORE
        }
    }
}

unsafe extern "C" fn finish(runtime: *mut Runtime, next_pc: u32) -> u8 {
    // SAFETY: the generated caller passed the same live Runtime as prepare.
    let runtime = unsafe { &mut *runtime };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let cpu = unsafe { &mut *runtime.cpu };
        commit_native(cpu, next_pc);
    }));
    match result {
        Ok(()) => 1,
        Err(panic) => {
            runtime.panic = Some(panic);
            0
        }
    }
}

fn commit_native(cpu: &mut Cpu, next_pc: u32) {
    cpu.pc = next_pc;
    cpu.instructions_executed += 1;
    cpu.jit.as_mut().unwrap().stats.native_instructions += 1;
    cpu.accept_pending_irq();
    cpu.sync_register_block();
}

fn load(builder: &mut FunctionBuilder, registers: Value, reg: u8) -> Value {
    if reg == 0 {
        builder.ins().iconst(types::I32, 0)
    } else {
        builder
            .ins()
            .load(types::I32, MemFlags::new(), registers, i32::from(reg) * 4)
    }
}

fn store(builder: &mut FunctionBuilder, registers: Value, reg: u8, value: Value) {
    if reg != 0 {
        builder
            .ins()
            .store(MemFlags::new(), value, registers, i32::from(reg) * 4);
    }
}

fn emit(builder: &mut FunctionBuilder, registers: Value, item: BlockInstruction) -> Value {
    let next = item.pc.wrapping_add(4);
    let mut next_pc = builder.ins().iconst(types::I32, i64::from(next));
    match item.instruction {
        Instruction::Nop => {}
        Instruction::Add { rd, rs1, rs2 }
        | Instruction::Sub { rd, rs1, rs2 }
        | Instruction::And { rd, rs1, rs2 }
        | Instruction::Or { rd, rs1, rs2 }
        | Instruction::Xor { rd, rs1, rs2 }
        | Instruction::Slt { rd, rs1, rs2 }
        | Instruction::Sltu { rd, rs1, rs2 }
        | Instruction::Sll { rd, rs1, rs2 }
        | Instruction::Sla { rd, rs1, rs2 }
        | Instruction::Srl { rd, rs1, rs2 }
        | Instruction::Sra { rd, rs1, rs2 } => {
            let a = load(builder, registers, rs1);
            let b = load(builder, registers, rs2);
            let value = match item.instruction {
                Instruction::Add { .. } => builder.ins().iadd(a, b),
                Instruction::Sub { .. } => builder.ins().isub(a, b),
                Instruction::And { .. } => builder.ins().band(a, b),
                Instruction::Or { .. } => builder.ins().bor(a, b),
                Instruction::Xor { .. } => builder.ins().bxor(a, b),
                Instruction::Slt { .. } | Instruction::Sltu { .. } => {
                    let cc = if matches!(item.instruction, Instruction::Slt { .. }) {
                        IntCC::SignedLessThan
                    } else {
                        IntCC::UnsignedLessThan
                    };
                    let bit = builder.ins().icmp(cc, a, b);
                    builder.ins().uextend(types::I32, bit)
                }
                _ => {
                    let shift = builder.ins().band_imm(b, 31);
                    match item.instruction {
                        Instruction::Sll { .. } | Instruction::Sla { .. } => {
                            builder.ins().ishl(a, shift)
                        }
                        Instruction::Srl { .. } => builder.ins().ushr(a, shift),
                        _ => builder.ins().sshr(a, shift),
                    }
                }
            };
            store(builder, registers, rd, value);
        }
        Instruction::Mul { rd, rs1, rs2 }
        | Instruction::Mulh { rd, rs1, rs2 }
        | Instruction::Div { rd, rs1, rs2 }
        | Instruction::Divu { rd, rs1, rs2 }
        | Instruction::Mod { rd, rs1, rs2 } => {
            let a = load(builder, registers, rs1);
            let b = load(builder, registers, rs2);
            let value = match item.instruction {
                Instruction::Mul { .. } => builder.ins().imul(a, b),
                Instruction::Mulh { .. } => builder.ins().smulhi(a, b),
                _ => emit_division(builder, item.instruction, a, b),
            };
            store(builder, registers, rd, value);
        }
        Instruction::Addi { rd, rs1, imm } => {
            let value = load(builder, registers, rs1);
            let value = builder.ins().iadd_imm(value, i64::from(imm));
            store(builder, registers, rd, value);
        }
        Instruction::Ldil { rd, imm } | Instruction::Ldih { rd, imm } => {
            let current = load(builder, registers, rd);
            let high = matches!(item.instruction, Instruction::Ldih { .. });
            let masked = builder.ins().band_imm(
                current,
                if high {
                    0xFFFF
                } else {
                    0xFFFF_0000u32 as i32 as i64
                },
            );
            let bits = if high {
                u32::from(imm) << 16
            } else {
                u32::from(imm)
            };
            let value = builder.ins().bor_imm(masked, i64::from(bits));
            store(builder, registers, rd, value);
        }
        Instruction::Beq { rs1, rs2, offset } | Instruction::Bne { rs1, rs2, offset } => {
            let a = load(builder, registers, rs1);
            let b = load(builder, registers, rs2);
            let cc = if matches!(item.instruction, Instruction::Beq { .. }) {
                IntCC::Equal
            } else {
                IntCC::NotEqual
            };
            let condition = builder.ins().icmp(cc, a, b);
            let target = builder
                .ins()
                .iconst(types::I32, i64::from(Cpu::branch_target(next, offset)));
            next_pc = builder.ins().select(condition, target, next_pc);
        }
        Instruction::Jmp { offset } | Instruction::Jal { offset } => {
            if matches!(item.instruction, Instruction::Jal { .. }) {
                store(builder, registers, REG_LR as u8, next_pc);
            }
            next_pc = builder
                .ins()
                .iconst(types::I32, i64::from(Cpu::branch_target(next, offset)));
        }
        Instruction::Jr { rd } | Instruction::Jalr { rd } => {
            let target = load(builder, registers, rd);
            if matches!(item.instruction, Instruction::Jalr { .. }) {
                store(builder, registers, REG_LR as u8, next_pc);
            }
            next_pc = target;
        }
        Instruction::Cpuid => {
            let id = builder.ins().iconst(types::I32, i64::from(CPU_ID));
            let features = builder.ins().iconst(types::I32, i64::from(CPU_FEATURES));
            store(builder, registers, REG_CPUID as u8, id);
            store(builder, registers, REG_FEATURES as u8, features);
        }
        _ => unreachable!("only supported instructions reach the emitter"),
    }
    next_pc
}

/// Non-trapping SRC32 division: x/0 = 0 and x%0 = 0; DIV MIN/-1 = MIN and
/// MOD MIN/-1 = 0. The divisor is replaced with 1 in both special cases.
fn emit_division(
    builder: &mut FunctionBuilder,
    instruction: Instruction,
    a: Value,
    b: Value,
) -> Value {
    let zero = builder.ins().iconst(types::I32, 0);
    let one = builder.ins().iconst(types::I32, 1);
    let by_zero = builder.ins().icmp_imm(IntCC::Equal, b, 0);
    if matches!(instruction, Instruction::Divu { .. }) {
        let divisor = builder.ins().select(by_zero, one, b);
        let quotient = builder.ins().udiv(a, divisor);
        return builder.ins().select(by_zero, zero, quotient);
    }
    let min = builder.ins().icmp_imm(IntCC::Equal, a, i64::from(i32::MIN));
    let minus_one = builder.ins().icmp_imm(IntCC::Equal, b, -1);
    let overflow = builder.ins().band(min, minus_one);
    let special = builder.ins().bor(by_zero, overflow);
    let divisor = builder.ins().select(special, one, b);
    if matches!(instruction, Instruction::Mod { .. }) {
        // MIN % 1 and x % 1 are both 0.
        builder.ins().srem(a, divisor)
    } else {
        // MIN / 1 is MIN; only division by zero needs the explicit 0.
        let quotient = builder.ins().sdiv(a, divisor);
        builder.ins().select(by_zero, zero, quotient)
    }
}

fn compile(
    module: &mut JITModule,
    serial: usize,
    instructions: &[BlockInstruction],
) -> Result<(cranelift_module::FuncId, usize), String> {
    let pointer = module.target_config().pointer_type();
    let mut context = module.make_context();
    context.func.signature.params = vec![AbiParam::new(pointer), AbiParam::new(types::I64)];
    let mut prepare_signature = module.make_signature();
    prepare_signature.params = vec![
        AbiParam::new(pointer),
        AbiParam::new(types::I32),
        AbiParam::new(types::I32),
    ];
    prepare_signature.returns.push(AbiParam::new(pointer));
    let prepare_id = module
        .declare_function("src32_prepare", Linkage::Import, &prepare_signature)
        .map_err(|e| e.to_string())?;
    let mut finish_signature = module.make_signature();
    finish_signature.params = vec![AbiParam::new(pointer), AbiParam::new(types::I32)];
    finish_signature.returns.push(AbiParam::new(types::I8));
    let finish_id = module
        .declare_function("src32_finish", Linkage::Import, &finish_signature)
        .map_err(|e| e.to_string())?;
    let function_id = module
        .declare_function(
            &format!("src32_block_{serial}"),
            Linkage::Local,
            &context.func.signature,
        )
        .map_err(|e| e.to_string())?;
    let prepare_ref = module.declare_func_in_func(prepare_id, &mut context.func);
    let finish_ref = module.declare_func_in_func(finish_id, &mut context.func);
    let mut frontend = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut context.func, &mut frontend);
    let entry = builder.create_block();
    let exit = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    let runtime = builder.block_params(entry)[0];
    let budget = builder.block_params(entry)[1];
    let mut previous_pc = builder.ins().iconst(types::I32, 0);
    for (index, item) in instructions.iter().enumerate() {
        let ready = builder.create_block();
        let execute = builder.create_block();
        let exhausted = builder.create_block();
        let within_budget =
            builder
                .ins()
                .icmp_imm(IntCC::UnsignedGreaterThan, budget, index as i64);
        builder
            .ins()
            .brif(within_budget, ready, &[], exhausted, &[]);
        builder.switch_to_block(exhausted);
        if index != 0 {
            builder.ins().call(finish_ref, &[runtime, previous_pc]);
        }
        builder.ins().jump(exit, &[]);
        builder.switch_to_block(ready);
        let index = builder.ins().iconst(types::I32, index as i64);
        let call = builder
            .ins()
            .call(prepare_ref, &[runtime, index, previous_pc]);
        let registers = builder.inst_results(call)[0];
        let valid = builder.ins().icmp_imm(IntCC::NotEqual, registers, 0);
        builder.ins().brif(valid, execute, &[], exit, &[]);
        builder.switch_to_block(execute);
        previous_pc = if memory_access(item.instruction).is_some() {
            // Unreachable: prepare executes it in the interpreter and exits.
            builder
                .ins()
                .iconst(types::I32, i64::from(item.pc.wrapping_add(4)))
        } else {
            emit(&mut builder, registers, *item)
        };
    }
    builder.ins().call(finish_ref, &[runtime, previous_pc]);
    builder.ins().jump(exit, &[]);
    builder.switch_to_block(exit);
    builder.ins().return_(&[]);
    builder.seal_all_blocks();
    builder.finalize();
    module
        .define_function(function_id, &mut context)
        .map_err(|e| e.to_string())?;
    let bytes = context.compiled_code().unwrap().code_buffer().len();
    Ok((function_id, bytes))
}

#[derive(Clone, Copy)]
enum Tier {
    Strict,
    Batch,
}

enum Native {
    Strict(NativeBlock),
    Batch(NativeBatch),
}

/// Define one tier and make it executable.
fn compile_native(
    module: &mut JITModule,
    tier: Tier,
    serial: usize,
    instructions: &[BlockInstruction],
) -> Result<(Native, usize), String> {
    let (id, bytes) = match tier {
        Tier::Strict => compile(module, serial, instructions)?,
        Tier::Batch => compile_batch(module, serial, instructions)?,
    };
    module.finalize_definitions().map_err(|e| e.to_string())?;
    let code = module.get_finalized_function(id);
    // SAFETY: each tier's declared host signature exactly matches NativeBlock
    // or NativeBatch. Ownership stays in JitEngine; callers can use these
    // pointers only until module reset.
    let native = unsafe {
        match tier {
            Tier::Strict => Native::Strict(std::mem::transmute::<*const u8, NativeBlock>(code)),
            Tier::Batch => Native::Batch(std::mem::transmute::<*const u8, NativeBatch>(code)),
        }
    };
    Ok((native, bytes))
}

/// Batch function: ALU/branch/multiply code inline, plain-memory accesses via
/// the batch helper. Observation and validation happen in Rust
/// before entry. Each early exit returns the PC of the executed prefix.
fn compile_batch(
    module: &mut JITModule,
    serial: usize,
    instructions: &[BlockInstruction],
) -> Result<(cranelift_module::FuncId, usize), String> {
    let mut context = module.make_context();
    let pointer = module.target_config().pointer_type();
    context.func.signature.params = vec![
        AbiParam::new(pointer),
        AbiParam::new(types::I32),
        AbiParam::new(pointer),
    ];
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I32));
    let mut memory_signature = module.make_signature();
    memory_signature.params = vec![
        AbiParam::new(pointer),
        AbiParam::new(types::I32),
        AbiParam::new(types::I32),
    ];
    memory_signature.returns.push(AbiParam::new(types::I32));
    let memory_id = module
        .declare_function("src32_batch_memory", Linkage::Import, &memory_signature)
        .map_err(|e| e.to_string())?;
    let id = module
        .declare_function(
            &format!("src32_batch_{serial}"),
            Linkage::Local,
            &context.func.signature,
        )
        .map_err(|e| e.to_string())?;
    let memory_ref = module.declare_func_in_func(memory_id, &mut context.func);
    let mut frontend = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut context.func, &mut frontend);
    let entry = builder.create_block();
    let exit = builder.create_block();
    builder.append_block_param(exit, types::I32);
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    let registers = builder.block_params(entry)[0];
    let count = builder.block_params(entry)[1];
    let batch_context = builder.block_params(entry)[2];
    let mut cycles = 0;
    for (index, item) in instructions.iter().enumerate() {
        cycles += latency(item.instruction);
        let next_pc = if memory_access(item.instruction).is_some() {
            let index_value = builder.ins().iconst(types::I32, index as i64);
            let cycles_value = builder.ins().iconst(types::I32, cycles as i64);
            let call = builder
                .ins()
                .call(memory_ref, &[batch_context, index_value, cycles_value]);
            let status = builder.inst_results(call)[0];
            let resume = builder.create_block();
            let stopped = builder.create_block();
            builder.ins().brif(status, stopped, &[], resume, &[]);
            builder.switch_to_block(stopped);
            let after = builder
                .ins()
                .iconst(types::I32, i64::from(item.pc.wrapping_add(4)));
            let before = builder.ins().iconst(types::I32, i64::from(item.pc));
            let is_after =
                builder
                    .ins()
                    .icmp_imm(IntCC::Equal, status, i64::from(BATCH_STOP_AFTER));
            let stop_pc = builder.ins().select(is_after, after, before);
            builder.ins().jump(exit, &[stop_pc]);
            builder.switch_to_block(resume);
            builder
                .ins()
                .iconst(types::I32, i64::from(item.pc.wrapping_add(4)))
        } else {
            emit(&mut builder, registers, *item)
        };
        if index + 1 == instructions.len() {
            builder.ins().jump(exit, &[next_pc]);
        } else {
            let next = builder.create_block();
            let more =
                builder
                    .ins()
                    .icmp_imm(IntCC::UnsignedGreaterThan, count, (index + 1) as i64);
            builder.ins().brif(more, next, &[], exit, &[next_pc]);
            builder.switch_to_block(next);
        }
    }
    builder.switch_to_block(exit);
    let pc = builder.block_params(exit)[0];
    builder.ins().return_(&[pc]);
    builder.seal_all_blocks();
    builder.finalize();
    module
        .define_function(id, &mut context)
        .map_err(|e| e.to_string())?;
    let bytes = context.compiled_code().unwrap().code_buffer().len();
    Ok((id, bytes))
}
