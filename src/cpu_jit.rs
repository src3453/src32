//! Conservative native blocks. Rust owns every cycle boundary; generated code
//! only accesses the explicit [u32; 32] register array between those boundaries.
use super::*;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use std::any::Any;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

const MAX_INSTRUCTIONS: usize = 16;
const MAX_BLOCKS: usize = 1024;
const HOT_THRESHOLD: u32 = 16;

type NativeBlock = unsafe extern "C" fn(*mut Runtime, u64);

#[derive(Clone, Copy)]
struct BlockInstruction {
    pc: u32,
    instruction: Instruction,
}

struct Entry {
    instructions: Box<[BlockInstruction]>,
    visits: u32,
    function: Option<NativeBlock>,
    failed: bool,
    invalid: bool,
}

pub(super) struct JitEngine {
    module: Option<JITModule>,
    entries: HashMap<u32, Entry>,
    definitions: usize,
    stats: JitStats,
}

impl JitEngine {
    pub(super) fn new() -> Self {
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
            entries: HashMap::new(),
            definitions: 0,
        }
    }

    fn create_module() -> Result<JITModule, String> {
        let mut builder = JITBuilder::new(cranelift_module::default_libcall_names())
            .map_err(|error| error.to_string())?;
        builder.symbol("src32_prepare", prepare as *const u8);
        builder.symbol("src32_finish", finish as *const u8);
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

    pub(super) fn record_fallback_cycle(&mut self) {
        self.stats.fallback_cycles += 1;
    }

    fn lookup(&mut self, bus: &Bus, pc: u32) -> Option<(NativeBlock, *const BlockInstruction)> {
        self.module.as_ref()?;
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
            instructions: preview(bus, pc).into_boxed_slice(),
            visits: 0,
            function: None,
            failed: false,
            invalid: false,
        });
        if entry.instructions.is_empty() || entry.failed {
            return None;
        }
        entry.visits = entry.visits.saturating_add(1);
        if entry.function.is_none() && entry.visits >= HOT_THRESHOLD {
            let start = Instant::now();
            let result = compile(
                self.module.as_mut().unwrap(),
                self.definitions,
                &entry.instructions,
            );
            self.stats.compile_time += start.elapsed();
            self.definitions += 1;
            match result {
                Ok((function, bytes)) => {
                    entry.function = Some(function);
                    self.stats.compilations += 1;
                    self.stats.native_code_bytes += bytes;
                }
                Err(error) => {
                    entry.failed = true;
                    self.stats.compile_failures += 1;
                    tracing::warn!("CPU JIT block at {pc:#010x} failed; using Cached: {error}");
                }
            }
        }
        Some((entry.function?, entry.instructions.as_ptr()))
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
    )
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

fn preview(bus: &Bus, mut pc: u32) -> Vec<BlockInstruction> {
    let mut instructions = Vec::new();
    for _ in 0..MAX_INSTRUCTIONS {
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

struct Runtime {
    cpu: *mut Cpu,
    instructions: *const BlockInstruction,
    entry_pc: u32,
    panic: Option<Box<dyn Any + Send>>,
}

impl Cpu {
    pub(super) fn run_jit_block(&mut self, budget: u128) -> bool {
        if !self.running
            || self.state != CpuState::Fetch
            || self.instr_mode != InstructionMode::Normal
        {
            return false;
        }
        let Some((function, instructions)) = self.jit.as_mut().unwrap().lookup(&self.bus, self.pc)
        else {
            return false;
        };
        let start_cycles = self.cycles;
        let mut runtime = Runtime {
            cpu: self,
            instructions,
            entry_pc: self.pc,
            panic: None,
        };
        // SAFETY: the module and instruction array remain owned by this CPU for
        // the whole call. Helpers never evict/reset them, and all borrows from
        // lookup have ended. Native code uses only registers returned by prepare.
        unsafe { function(&mut runtime, budget.min(u128::from(u64::MAX)) as u64) };
        if let Some(panic) = runtime.panic {
            resume_unwind(panic);
        }
        self.cycles != start_cycles
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

fn compile(
    module: &mut JITModule,
    serial: usize,
    instructions: &[BlockInstruction],
) -> Result<(NativeBlock, usize), String> {
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
        previous_pc = emit(&mut builder, registers, *item);
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
    module.finalize_definitions().map_err(|e| e.to_string())?;
    let code = module.get_finalized_function(function_id);
    // SAFETY: the declared host signature exactly matches NativeBlock. Ownership
    // stays in JitEngine; callers can use this pointer only until module reset.
    Ok((
        unsafe { std::mem::transmute::<*const u8, NativeBlock>(code) },
        bytes,
    ))
}
