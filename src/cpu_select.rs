// CPU micro-architecture selection (Extension p rollout).
//
// The emulator boots on the pipelined CPU by default; the scalar CPU remains
// available as the behavioral reference. Selection order: `--cpu` CLI flag,
// then the `cpu.microarchitecture` key in config.yaml, then the default.

use std::path::Path;
use std::str::FromStr;

use crate::bus::{Bus, BusRegion};
use crate::cpu::{Cpu, DecodedInstruction, InstructionMode};
use crate::cpu_pipeline::PipelineCpu;

/// Which micro-architecture backs the emulator. Case-sensitive extension
/// naming applies: this selects implementations, not ISA extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CpuKind {
    Scalar,
    #[default]
    Pipelined,
}

impl CpuKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CpuKind::Scalar => "scalar",
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
            "pipeline" | "pipelined" => Ok(CpuKind::Pipelined),
            _ => Err(format!(
                "unknown CPU micro-architecture '{text}' (expected 'scalar' or 'pipeline')"
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
            let value = iter.next().ok_or("--cpu requires 'scalar' or 'pipeline'")?;
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

/// Either CPU micro-architecture behind one harness API. All guest-facing
/// behavior (registers, PC, memory, IRQ, HALT) matches; only the reported
/// CPUID feature mask and the timing/counters differ.
pub enum AnyCpu {
    Scalar(Cpu),
    Pipelined(PipelineCpu),
}

impl AnyCpu {
    pub fn new(kind: CpuKind, bus: Bus) -> Self {
        match kind {
            CpuKind::Scalar => AnyCpu::Scalar(Cpu::new(bus)),
            CpuKind::Pipelined => AnyCpu::Pipelined(PipelineCpu::new(bus)),
        }
    }

    pub fn kind(&self) -> CpuKind {
        match self {
            AnyCpu::Scalar(_) => CpuKind::Scalar,
            AnyCpu::Pipelined(_) => CpuKind::Pipelined,
        }
    }

    /// (stall_cycles, flush_events) for the pipeline; `None` for scalar.
    pub fn pipeline_stats(&self) -> Option<(u128, u128)> {
        match self {
            AnyCpu::Scalar(_) => None,
            AnyCpu::Pipelined(cpu) => Some((cpu.stall_cycles(), cpu.flush_events())),
        }
    }

    pub fn reset(&mut self, pc: u32) {
        match self {
            AnyCpu::Scalar(cpu) => cpu.reset(pc),
            AnyCpu::Pipelined(cpu) => cpu.reset(pc),
        }
    }

    pub fn load_program(&mut self, base: u32, image: &[u8]) {
        match self {
            AnyCpu::Scalar(cpu) => cpu.load_program(base, image),
            AnyCpu::Pipelined(cpu) => cpu.load_program(base, image),
        }
    }

    pub fn pc(&self) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) => cpu.pc(),
            AnyCpu::Pipelined(cpu) => cpu.pc(),
        }
    }

    pub fn set_pc(&mut self, pc: u32) {
        match self {
            AnyCpu::Scalar(cpu) => cpu.set_pc(pc),
            AnyCpu::Pipelined(cpu) => cpu.set_pc(pc),
        }
    }

    pub fn instruction_mode(&self) -> InstructionMode {
        match self {
            AnyCpu::Scalar(cpu) => cpu.instruction_mode(),
            AnyCpu::Pipelined(cpu) => cpu.instruction_mode(),
        }
    }

    pub fn set_instruction_mode(&mut self, mode: InstructionMode) {
        match self {
            AnyCpu::Scalar(cpu) => cpu.set_instruction_mode(mode),
            AnyCpu::Pipelined(cpu) => cpu.set_instruction_mode(mode),
        }
    }

    pub fn is_running(&self) -> bool {
        match self {
            AnyCpu::Scalar(cpu) => cpu.is_running(),
            AnyCpu::Pipelined(cpu) => cpu.is_running(),
        }
    }

    pub fn cycles(&self) -> u128 {
        match self {
            AnyCpu::Scalar(cpu) => cpu.cycles(),
            AnyCpu::Pipelined(cpu) => cpu.cycles(),
        }
    }

    pub fn running_cycles(&self) -> u128 {
        match self {
            AnyCpu::Scalar(cpu) => cpu.running_cycles(),
            AnyCpu::Pipelined(cpu) => cpu.running_cycles(),
        }
    }

    pub fn instructions_executed(&self) -> u128 {
        match self {
            AnyCpu::Scalar(cpu) => cpu.instructions_executed(),
            AnyCpu::Pipelined(cpu) => cpu.instructions_executed(),
        }
    }

    pub fn epc(&self) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) => cpu.epc(),
            AnyCpu::Pipelined(cpu) => cpu.epc(),
        }
    }

    pub fn irq_cause(&self) -> u8 {
        match self {
            AnyCpu::Scalar(cpu) => cpu.irq_cause(),
            AnyCpu::Pipelined(cpu) => cpu.irq_cause(),
        }
    }

    pub fn irq_enabled(&self) -> bool {
        match self {
            AnyCpu::Scalar(cpu) => cpu.irq_enabled(),
            AnyCpu::Pipelined(cpu) => cpu.irq_enabled(),
        }
    }

    pub fn set_irq_input(&mut self, level: bool, number: u8) {
        match self {
            AnyCpu::Scalar(cpu) => cpu.set_irq_input(level, number),
            AnyCpu::Pipelined(cpu) => cpu.set_irq_input(level, number),
        }
    }

    pub fn set_irq_source(&mut self, number: u8, level: bool) {
        match self {
            AnyCpu::Scalar(cpu) => cpu.set_irq_source(number, level),
            AnyCpu::Pipelined(cpu) => cpu.set_irq_source(number, level),
        }
    }

    pub fn read_mem_u8(&mut self, addr: u32) -> u8 {
        match self {
            AnyCpu::Scalar(cpu) => cpu.read_mem_u8(addr),
            AnyCpu::Pipelined(cpu) => cpu.read_mem_u8(addr),
        }
    }

    pub fn read_debug_mem_u8(&mut self, addr: u32) -> Option<u8> {
        match self {
            AnyCpu::Scalar(cpu) => cpu.read_debug_mem_u8(addr),
            AnyCpu::Pipelined(cpu) => cpu.read_debug_mem_u8(addr),
        }
    }

    pub fn read_mem_u32_be(&mut self, addr: u32) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) => cpu.read_mem_u32_be(addr),
            AnyCpu::Pipelined(cpu) => cpu.read_mem_u32_be(addr),
        }
    }

    pub fn write_mem_u8(&mut self, addr: u32, value: u8) {
        match self {
            AnyCpu::Scalar(cpu) => cpu.write_mem_u8(addr, value),
            AnyCpu::Pipelined(cpu) => cpu.write_mem_u8(addr, value),
        }
    }

    pub fn write_mem_u32_be(&mut self, addr: u32, value: u32) {
        match self {
            AnyCpu::Scalar(cpu) => cpu.write_mem_u32_be(addr, value),
            AnyCpu::Pipelined(cpu) => cpu.write_mem_u32_be(addr, value),
        }
    }

    pub fn read_u32(&mut self, addr: u32) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) => cpu.read_u32(addr),
            AnyCpu::Pipelined(cpu) => cpu.read_u32(addr),
        }
    }

    pub fn read_u16_be(&mut self, addr: u32) -> u16 {
        match self {
            AnyCpu::Scalar(cpu) => cpu.read_u16_be(addr),
            AnyCpu::Pipelined(cpu) => cpu.read_u16_be(addr),
        }
    }

    pub fn read_reg(&self, reg: usize) -> u32 {
        match self {
            AnyCpu::Scalar(cpu) => cpu.read_reg(reg),
            AnyCpu::Pipelined(cpu) => cpu.read_reg(reg),
        }
    }

    pub fn write_reg(&mut self, reg: usize, value: u32) -> Result<String, String> {
        match self {
            AnyCpu::Scalar(cpu) => cpu.write_reg(reg, value),
            AnyCpu::Pipelined(cpu) => cpu.write_reg(reg, value),
        }
    }

    pub fn bus_memory_map(&self) -> Box<dyn Iterator<Item = BusRegion> + '_> {
        match self {
            AnyCpu::Scalar(cpu) => Box::new(cpu.bus_memory_map()),
            AnyCpu::Pipelined(cpu) => Box::new(cpu.bus_memory_map()),
        }
    }

    pub fn decode_at(&mut self, addr: u32, mode: InstructionMode) -> DecodedInstruction {
        match self {
            AnyCpu::Scalar(cpu) => cpu.decode_at(addr, mode),
            AnyCpu::Pipelined(cpu) => cpu.decode_at(addr, mode),
        }
    }

    pub fn return_state_text(&mut self) -> String {
        match self {
            AnyCpu::Scalar(cpu) => cpu.return_state_text(),
            AnyCpu::Pipelined(cpu) => cpu.return_state_text(),
        }
    }

    pub fn tick(&mut self) {
        match self {
            AnyCpu::Scalar(cpu) => cpu.tick(),
            AnyCpu::Pipelined(cpu) => cpu.tick(),
        }
    }

    pub fn step_once(&mut self) -> bool {
        match self {
            AnyCpu::Scalar(cpu) => cpu.step_once(),
            AnyCpu::Pipelined(cpu) => cpu.step_once(),
        }
    }

    pub fn run(&mut self, max_cycles: usize) {
        match self {
            AnyCpu::Scalar(cpu) => cpu.run(max_cycles),
            AnyCpu::Pipelined(cpu) => cpu.run(max_cycles),
        }
    }
}
