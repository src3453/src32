// CPU micro-architecture selection (Extension p rollout).
//
// The emulator boots on the pipelined CPU by default; the scalar CPU remains
// available as the behavioral reference. Selection order: `--cpu` CLI flag,
// then the `cpu.microarchitecture` key in config.yaml, then the default.

use std::path::Path;
use std::str::FromStr;

use crate::bus::{Bus, BusRegion};
use crate::cpu::{Cpu, CpuProfile, DecodedInstruction, InstructionMode};
use crate::cpu_pipeline::PipelineCpu;

/// Which micro-architecture backs the emulator. Case-sensitive extension
/// naming applies: this selects implementations, not ISA extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CpuKind {
    Scalar,
    Cached,
    CachedRelaxed,
    Jit,
    #[default]
    Pipelined,
}

impl CpuKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CpuKind::Scalar => "scalar",
            CpuKind::Cached => "cached",
            CpuKind::CachedRelaxed => "cached-relaxed",
            CpuKind::Jit => "jit",
            CpuKind::Pipelined => "pipeline",
        }
    }

    pub fn is_pipeline(self) -> bool {
        matches!(self, CpuKind::Pipelined)
    }
}

impl FromStr for CpuKind {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.to_ascii_lowercase().as_str() {
            "scalar" => Ok(CpuKind::Scalar),
            "cached" => Ok(CpuKind::Cached),
            "cached-relaxed" => Ok(CpuKind::CachedRelaxed),
            "jit" if cfg!(feature = "jit") => Ok(CpuKind::Jit),
            "jit" => Err("CPU JIT requires a build with --features jit".into()),
            "pipeline" | "pipelined" => Ok(CpuKind::Pipelined),
            _ => Err(format!(
                "unknown CPU micro-architecture '{text}' (expected 'scalar', 'cached', 'cached-relaxed', 'jit' (feature required), or 'pipeline')"
            )),
        }
    }
}

/// Resolve the effective micro-architecture: CLI flag first, then the
/// `cpu.microarchitecture` key in `config_path`, then the pipeline default.
/// A missing config file (or a config without the key) means default.
pub fn resolve_cpu_kind(cli: Option<CpuKind>, config_path: &Path) -> Result<CpuKind, String> {
    if let Some(kind) = cli {
        return Ok(kind);
    }
    let contents = match std::fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CpuKind::default());
        }
        Err(error) => {
            return Err(format!("cannot read {}: {error}", config_path.display()));
        }
    };
    let root: yaml_serde::Value = yaml_serde::from_str(&contents)
        .map_err(|error| format!("invalid YAML in {}: {error}", config_path.display()))?;
    let Some(cpu) = root.get("cpu") else {
        return Ok(CpuKind::default());
    };
    let table = cpu
        .as_mapping()
        .ok_or("`cpu` must be a mapping".to_string())?;
    let Some(mode) = table.get("microarchitecture") else {
        return Ok(CpuKind::default());
    };
    let text = mode
        .as_str()
        .ok_or("`cpu.microarchitecture` must be a string".to_string())?;
    text.parse::<CpuKind>().map_err(|error| {
        format!(
            "invalid `cpu.microarchitecture` in {}: {error}",
            config_path.display()
        )
    })
}

/// Largest configurable JIT block. A batch still stops at branches, at the
/// relaxed device window and at the run budget.
pub const MAX_JIT_BLOCK_SIZE: u64 = 256;

/// Resolve the maximum instructions previewed into each JIT block. Presets are
/// Small=16 (default), Medium=32, Big=64; custom values may use 2..=256.
pub fn resolve_jit_block_size(config_path: &Path) -> Result<usize, String> {
    let contents = match std::fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(16),
        Err(error) => return Err(format!("cannot read {}: {error}", config_path.display())),
    };
    let root: yaml_serde::Value = yaml_serde::from_str(&contents)
        .map_err(|error| format!("invalid YAML in {}: {error}", config_path.display()))?;
    let Some(cpu) = root.get("cpu") else {
        return Ok(16);
    };
    let table = cpu
        .as_mapping()
        .ok_or("`cpu` must be a mapping".to_string())?;
    let Some(value) = table.get("jit_block_size") else {
        return Ok(16);
    };
    let size = value
        .as_u64()
        .ok_or("`cpu.jit_block_size` must be an integer".to_string())?;
    if !(2..=MAX_JIT_BLOCK_SIZE).contains(&size) {
        return Err(format!(
            "`cpu.jit_block_size` must be between 2 and {MAX_JIT_BLOCK_SIZE} (got {size})"
        ));
    }
    Ok(size as usize)
}

/// Parse `--cpu KIND` / `--cpu=KIND` out of a free-form argument list.
/// Returns the remaining (non-cpu) arguments plus the selected kind, if any.
pub fn split_cpu_arg(args: &[String]) -> Result<(Vec<String>, Option<CpuKind>), String> {
    let mut rest = Vec::with_capacity(args.len());
    let mut selected: Option<CpuKind> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--cpu" {
            if selected.is_some() {
                return Err("Duplicate --cpu".into());
            }
            let value = iter.next().ok_or(
                "--cpu requires 'scalar', 'cached', 'cached-relaxed', 'jit' (feature required), or 'pipeline'",
            )?;
            selected = Some(value.parse::<CpuKind>()?);
        } else if let Some(value) = arg.strip_prefix("--cpu=") {
            if selected.is_some() {
                return Err("Duplicate --cpu".into());
            }
            selected = Some(value.parse::<CpuKind>()?);
        } else {
            rest.push(arg.clone());
        }
    }
    Ok((rest, selected))
}

/// Shared harness over scalar, cached, native JIT, and pipeline execution.
/// ISA state is preserved; timing, CPUID features, and instruction-fetch bus
/// traffic depend on the selected microarchitecture.
pub enum AnyCpu {
    Scalar(Cpu),
    Cached(Cpu),
    Jit(Cpu),
    Pipelined(PipelineCpu),
}

impl AnyCpu {
    /// Construct a CPU using the optional `cpu.jit_block_size` setting in a YAML config.
    pub fn new_configured(kind: CpuKind, bus: Bus, config_path: &Path) -> Result<Self, String> {
        if kind == CpuKind::Jit {
            let block_size = resolve_jit_block_size(config_path)?;
            Ok(Self::new_with_jit_block_size(kind, bus, block_size))
        } else {
            Ok(Self::new(kind, bus))
        }
    }

    /// Test hook for JIT tiering: visits before the batch tier compiles and
    /// declined batch dispatches before the strict tier compiles (0 compiles
    /// both together). No effect on other CPU kinds.
    #[doc(hidden)]
    #[cfg(feature = "jit")]
    pub fn set_jit_tier_thresholds(&mut self, hot: u32, strict: u32) {
        if let Self::Jit(cpu) = self {
            cpu.set_jit_tier_thresholds(hot, strict);
        }
    }

    /// Test hook: disable Bus optimizations based on timing-insensitive devices.
    #[doc(hidden)]
    pub fn set_timing_insensitive_fast_paths(&mut self, enabled: bool) {
        match self {
            Self::Pipelined(cpu) => cpu.set_timing_insensitive_fast_paths(enabled),
            Self::Scalar(cpu) | Self::Cached(cpu) | Self::Jit(cpu) => {
                cpu.set_timing_insensitive_fast_paths(enabled)
            }
        }
    }

    /// Cycles skipped by exact spin-loop fast-forward.
    #[doc(hidden)]
    pub fn spin_skipped_cycles(&self) -> u128 {
        match self {
            Self::Pipelined(cpu) => cpu.spin_skipped_cycles(),
            Self::Scalar(cpu) | Self::Cached(cpu) | Self::Jit(cpu) => cpu.spin_skipped_cycles(),
        }
    }

    /// Test hook: enable or disable exact spin-loop fast-forward (default on).
    #[doc(hidden)]
    pub fn set_spin_fast_forward(&mut self, enabled: bool) {
        match self {
            Self::Pipelined(cpu) => cpu.set_spin_fast_forward(enabled),
            Self::Scalar(cpu) | Self::Cached(cpu) | Self::Jit(cpu) => {
                cpu.set_spin_fast_forward(enabled)
            }
        }
    }

    pub fn jit_stats(&self) -> Option<crate::cpu::JitStats> {
        match self {
            Self::Jit(cpu) => cpu.jit_stats(),
            _ => None,
        }
    }

    pub fn new(kind: CpuKind, bus: Bus) -> Self {
        Self::new_with_jit_block_size(kind, bus, 16)
    }

    pub fn new_with_jit_block_size(kind: CpuKind, bus: Bus, _jit_block_size: usize) -> Self {
        match kind {
            CpuKind::Scalar => AnyCpu::Scalar(Cpu::new(bus)),
            CpuKind::Cached => AnyCpu::Cached(Cpu::new_cached(bus)),
            CpuKind::CachedRelaxed => {
                let mut cpu = Cpu::new_cached(bus);
                cpu.relaxed_devices = true;
                AnyCpu::Cached(cpu)
            }
            CpuKind::Jit => {
                #[cfg(feature = "jit")]
                {
                    AnyCpu::Jit(Cpu::new_jit(bus, _jit_block_size))
                }
                #[cfg(not(feature = "jit"))]
                {
                    panic!("CPU JIT requires a build with --features jit")
                }
            }
            CpuKind::Pipelined => AnyCpu::Pipelined(PipelineCpu::new(bus)),
        }
    }

    pub fn kind(&self) -> CpuKind {
        match self {
            AnyCpu::Scalar(_) => CpuKind::Scalar,
            AnyCpu::Cached(cpu) => {
                if cpu.relaxed_devices {
                    CpuKind::CachedRelaxed
                } else {
                    CpuKind::Cached
                }
            }
            AnyCpu::Jit(_) => CpuKind::Jit,
            AnyCpu::Pipelined(_) => CpuKind::Pipelined,
        }
    }

    /// (stall_cycles, flush_events) for the pipeline; `None` for interpreters.
    pub fn pipeline_stats(&self) -> Option<(u128, u128)> {
        match self {
            AnyCpu::Scalar(_) | AnyCpu::Cached(_) | AnyCpu::Jit(_) => None,
            AnyCpu::Pipelined(cpu) => Some((cpu.stall_cycles(), cpu.flush_events())),
        }
    }

    /// (cache_hits, cache_misses) for cached interpretation; `None` otherwise.
    pub fn instruction_cache_stats(&self) -> Option<(u128, u128)> {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.instruction_cache_stats()
            }
            AnyCpu::Pipelined(_) => None,
        }
    }

    pub fn enable_profiling(&mut self, enabled: bool) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.enable_profiling(enabled)
            }
            AnyCpu::Pipelined(cpu) => cpu.enable_profiling(enabled),
        }
    }

    /// Per-PC instruction/cycle histogram; only the pipeline CPU records it.
    pub fn enable_pc_profile(&mut self, enabled: bool) {
        if let AnyCpu::Pipelined(cpu) = self {
            cpu.enable_pc_profile(enabled);
        }
    }

    pub fn take_pc_profile(&mut self) -> Vec<(bool, u32, u64, u64)> {
        match self {
            AnyCpu::Pipelined(cpu) => cpu.take_pc_profile(),
            _ => Vec::new(),
        }
    }

    pub fn take_profile(&mut self) -> Option<CpuProfile> {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.take_profile(),
            AnyCpu::Pipelined(cpu) => cpu.take_profile(),
        }
    }

    pub fn reset(&mut self, pc: u32) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.reset(pc),
            AnyCpu::Pipelined(cpu) => cpu.reset(pc),
        }
    }

    pub fn load_program(&mut self, base: u32, image: &[u8]) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.load_program(base, image)
            }
            AnyCpu::Pipelined(cpu) => cpu.load_program(base, image),
        }
    }

    pub fn pc(&self) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.pc(),
            AnyCpu::Pipelined(cpu) => cpu.pc(),
        }
    }

    pub fn set_pc(&mut self, pc: u32) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.set_pc(pc),
            AnyCpu::Pipelined(cpu) => cpu.set_pc(pc),
        }
    }

    pub fn instruction_mode(&self) -> InstructionMode {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.instruction_mode(),
            AnyCpu::Pipelined(cpu) => cpu.instruction_mode(),
        }
    }

    pub fn set_instruction_mode(&mut self, mode: InstructionMode) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.set_instruction_mode(mode)
            }
            AnyCpu::Pipelined(cpu) => cpu.set_instruction_mode(mode),
        }
    }

    pub fn is_running(&self) -> bool {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.is_running(),
            AnyCpu::Pipelined(cpu) => cpu.is_running(),
        }
    }

    pub fn cycles(&self) -> u128 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.cycles(),
            AnyCpu::Pipelined(cpu) => cpu.cycles(),
        }
    }

    pub fn running_cycles(&self) -> u128 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.running_cycles(),
            AnyCpu::Pipelined(cpu) => cpu.running_cycles(),
        }
    }

    pub fn instructions_executed(&self) -> u128 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.instructions_executed()
            }
            AnyCpu::Pipelined(cpu) => cpu.instructions_executed(),
        }
    }

    pub fn mmu_snapshot(&self) -> crate::cpu_privilege::MmuSnapshot {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.mmu_snapshot(),
            AnyCpu::Pipelined(cpu) => cpu.mmu_snapshot(),
        }
    }

    pub fn mmu_translate(&mut self, va: u32) -> crate::cpu_privilege::DebugTranslation {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.mmu_translate(va),
            AnyCpu::Pipelined(cpu) => cpu.mmu_translate(va),
        }
    }

    pub fn system_snapshot(&self) -> crate::cpu_privilege::SystemSnapshot {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.system_snapshot(),
            AnyCpu::Pipelined(cpu) => cpu.system_snapshot(),
        }
    }

    pub fn epc(&self) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.epc(),
            AnyCpu::Pipelined(cpu) => cpu.epc(),
        }
    }

    pub fn irq_cause(&self) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.irq_cause(),
            AnyCpu::Pipelined(cpu) => cpu.irq_cause(),
        }
    }

    /// STATUS as defined by Extension P (bit 2 = user mode).
    pub fn status(&self) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.status(),
            AnyCpu::Pipelined(cpu) => cpu.status(),
        }
    }

    pub fn irq_enabled(&self) -> bool {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.irq_enabled(),
            AnyCpu::Pipelined(cpu) => cpu.irq_enabled(),
        }
    }

    pub fn set_irq_input(&mut self, level: bool, number: u8) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.set_irq_input(level, number)
            }
            AnyCpu::Pipelined(cpu) => cpu.set_irq_input(level, number),
        }
    }

    pub fn set_irq_source(&mut self, number: u8, level: bool) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.set_irq_source(number, level)
            }
            AnyCpu::Pipelined(cpu) => cpu.set_irq_source(number, level),
        }
    }

    pub fn read_mem_u8(&mut self, addr: u32) -> u8 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.read_mem_u8(addr),
            AnyCpu::Pipelined(cpu) => cpu.read_mem_u8(addr),
        }
    }

    pub fn read_debug_mem_u8(&mut self, addr: u32) -> Option<u8> {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.read_debug_mem_u8(addr)
            }
            AnyCpu::Pipelined(cpu) => cpu.read_debug_mem_u8(addr),
        }
    }

    pub fn read_debug_mem_u16_be(&mut self, addr: u32) -> Option<u16> {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.read_debug_mem_u16_be(addr)
            }
            AnyCpu::Pipelined(cpu) => cpu.read_debug_mem_u16_be(addr),
        }
    }

    pub fn read_debug_mem_u32_be(&mut self, addr: u32) -> Option<u32> {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.read_debug_mem_u32_be(addr)
            }
            AnyCpu::Pipelined(cpu) => cpu.read_debug_mem_u32_be(addr),
        }
    }

    pub fn read_mem_u32_be(&mut self, addr: u32) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.read_mem_u32_be(addr)
            }
            AnyCpu::Pipelined(cpu) => cpu.read_mem_u32_be(addr),
        }
    }

    pub fn write_mem_u8(&mut self, addr: u32, value: u8) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.write_mem_u8(addr, value)
            }
            AnyCpu::Pipelined(cpu) => cpu.write_mem_u8(addr, value),
        }
    }

    pub fn write_mem_u32_be(&mut self, addr: u32, value: u32) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.write_mem_u32_be(addr, value)
            }
            AnyCpu::Pipelined(cpu) => cpu.write_mem_u32_be(addr, value),
        }
    }

    pub fn read_u32(&mut self, addr: u32) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.read_u32(addr),
            AnyCpu::Pipelined(cpu) => cpu.read_u32(addr),
        }
    }

    pub fn read_u16_be(&mut self, addr: u32) -> u16 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.read_u16_be(addr),
            AnyCpu::Pipelined(cpu) => cpu.read_u16_be(addr),
        }
    }

    pub fn read_reg(&self, reg: usize) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.read_reg(reg),
            AnyCpu::Pipelined(cpu) => cpu.read_reg(reg),
        }
    }

    pub fn write_reg(&mut self, reg: usize, value: u32) -> Result<String, String> {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.write_reg(reg, value)
            }
            AnyCpu::Pipelined(cpu) => cpu.write_reg(reg, value),
        }
    }

    pub fn bus_memory_map(&self) -> Box<dyn Iterator<Item = BusRegion> + '_> {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                Box::new(cpu.bus_memory_map())
            }
            AnyCpu::Pipelined(cpu) => Box::new(cpu.bus_memory_map()),
        }
    }

    pub fn mix_expansion_audio(&mut self, rate: u32, left: &mut [i16], right: &mut [i16]) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.mix_expansion_audio(rate, left, right)
            }
            AnyCpu::Pipelined(cpu) => cpu.mix_expansion_audio(rate, left, right),
        }
    }

    pub fn expansion_card_debug_info(
        &self,
    ) -> Vec<crate::devices::expansion::ExpansionCardDebugInfo> {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.expansion_card_debug_info()
            }
            AnyCpu::Pipelined(cpu) => cpu.expansion_card_debug_info(),
        }
    }

    pub fn reset_expansion_card_debug_stats(&mut self) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.reset_expansion_card_debug_stats()
            }
            AnyCpu::Pipelined(cpu) => cpu.reset_expansion_card_debug_stats(),
        }
    }

    pub fn decode_at(&mut self, addr: u32, mode: InstructionMode) -> DecodedInstruction {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => {
                cpu.decode_at(addr, mode)
            }
            AnyCpu::Pipelined(cpu) => cpu.decode_at(addr, mode),
        }
    }

    pub fn return_state_text(&mut self) -> String {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.return_state_text(),
            AnyCpu::Pipelined(cpu) => cpu.return_state_text(),
        }
    }

    pub fn tick(&mut self) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.tick(),
            AnyCpu::Pipelined(cpu) => cpu.tick(),
        }
    }

    pub fn step_once(&mut self) -> bool {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.step_once(),
            AnyCpu::Pipelined(cpu) => cpu.step_once(),
        }
    }

    pub fn run(&mut self, max_cycles: usize) {
        match self {
            AnyCpu::Scalar(cpu) | AnyCpu::Cached(cpu) | AnyCpu::Jit(cpu) => cpu.run(max_cycles),
            AnyCpu::Pipelined(cpu) => cpu.run(max_cycles),
        }
    }
}
