use cpt32::bus::Bus;
use cpt32::cpu_select::{AnyCpu, CpuKind, resolve_cpu_kind, resolve_jit_block_size};
use cpt32::devices::cpu::BUS_ERROR_VECTOR;
use cpt32::devices::ram::{connect_ram, connect_ram_with_size};
use std::sync::atomic::{AtomicU64, Ordering};

fn encode_r(op: u8, rd: u8, rs1: u8, rs2: u8) -> [u8; 4] {
    let raw =
        ((op as u32) << 26) | ((rd as u32) << 21) | ((rs1 as u32) << 16) | ((rs2 as u32) << 11);
    raw.to_be_bytes()
}

fn encode_i(op: u8, rd: u8, rs1: u8, imm: i16) -> [u8; 4] {
    let raw =
        ((op as u32) << 26) | ((rd as u32) << 21) | ((rs1 as u32) << 16) | (imm as u16 as u32);
    raw.to_be_bytes()
}

#[test]
fn unmapped_store_enters_bus_error_handler_for_each_cpu_kind() {
    for kind in [
        CpuKind::Scalar,
        CpuKind::Cached,
        CpuKind::CachedRelaxed,
        CpuKind::Pipelined,
    ] {
        let mut bus = Bus::new();
        connect_ram_with_size(&mut bus, 1024 * 1024);
        let mut cpu = AnyCpu::new(kind, bus);
        cpu.write_mem_u32_be(BUS_ERROR_VECTOR, 0x100);
        let mut program = Vec::new();
        program.extend_from_slice(&encode_i(0x3D, 1, 0, 0x10)); // LDIH R1, 0x10
        program.extend_from_slice(&encode_i(0x02, 0, 1, 0)); // ST R0, [R1]
        cpu.load_program(0, &program);
        cpu.load_program(0x100, &encode_r(0x3F, 0, 0, 0)); // HALT in exception handler

        cpu.run(128);

        assert!(
            !cpu.is_running(),
            "{kind:?} pc={:08X} epc={:08X} cause={}",
            cpu.pc(),
            cpu.epc(),
            cpu.irq_cause()
        );
        assert_eq!(cpu.epc(), 4, "{kind:?}");
        assert_eq!(cpu.irq_cause(), 0x8002_0000, "{kind:?}");
        assert_eq!(cpu.pc(), 0x104, "{kind:?}");
    }
}

#[test]
fn unmapped_load_enters_bus_error_handler_for_each_cpu_kind() {
    for kind in [
        CpuKind::Scalar,
        CpuKind::Cached,
        CpuKind::CachedRelaxed,
        CpuKind::Pipelined,
    ] {
        let mut bus = Bus::new();
        connect_ram_with_size(&mut bus, 1024 * 1024);
        let mut cpu = AnyCpu::new(kind, bus);
        cpu.write_mem_u32_be(BUS_ERROR_VECTOR, 0x100);
        let mut program = Vec::new();
        program.extend_from_slice(&encode_i(0x3D, 1, 0, 0x10)); // LDIH R1, 0x10
        program.extend_from_slice(&encode_i(0x01, 2, 1, 0)); // LD R2, [R1]
        cpu.load_program(0, &program);
        cpu.load_program(0x100, &encode_r(0x3F, 0, 0, 0));

        cpu.run(128);

        assert!(!cpu.is_running(), "{kind:?}");
        assert_eq!(cpu.epc(), 4, "{kind:?}");
        assert_eq!(cpu.irq_cause(), 0x8002_0000, "{kind:?}");
        assert_eq!(cpu.pc(), 0x104, "{kind:?}");
    }
}

#[test]
fn unmapped_store_without_bus_error_handler_stops_cpu_without_host_panic() {
    let mut bus = Bus::new();
    connect_ram_with_size(&mut bus, 1024 * 1024);
    let mut cpu = AnyCpu::new(CpuKind::CachedRelaxed, bus);
    let mut program = Vec::new();
    program.extend_from_slice(&encode_i(0x3D, 1, 0, 0x10)); // LDIH R1, 0x10
    program.extend_from_slice(&encode_i(0x02, 0, 1, 0)); // ST R0, [R1]
    cpu.load_program(0, &program);

    cpu.run(64);

    assert!(!cpu.is_running());
    assert_eq!(cpu.epc(), 4);
    assert_eq!(cpu.irq_cause(), 0x8002_0000);
    assert_eq!(cpu.pc(), BUS_ERROR_VECTOR);
}

#[test]
fn debugger_memory_reads_do_not_raise_cpu_bus_error_for_any_cpu_kind() {
    for kind in [
        CpuKind::Scalar,
        CpuKind::Cached,
        CpuKind::CachedRelaxed,
        CpuKind::Pipelined,
    ] {
        let mut bus = Bus::new();
        connect_ram_with_size(&mut bus, 1024 * 1024);
        let mut cpu = AnyCpu::new(kind, bus);
        cpu.load_program(0, &encode_r(0x3F, 0, 0, 0));

        assert_eq!(cpu.read_debug_mem_u8(0x0010_0000), None, "{kind:?}");
        assert_eq!(cpu.read_mem_u32_be(0x0010_0000), 0, "{kind:?}");

        cpu.run(64);

        assert!(!cpu.is_running(), "{kind:?}");
        assert_eq!(cpu.epc(), 0, "{kind:?}");
        assert_eq!(cpu.irq_cause(), 0, "{kind:?}");
    }
}
#[test]
fn cpu_modes_parse_and_keep_pipeline_as_default() {
    assert_eq!(CpuKind::default(), CpuKind::Pipelined);
    assert_eq!("pipeline".parse::<CpuKind>().unwrap(), CpuKind::Pipelined);
    assert_eq!("scalar".parse::<CpuKind>().unwrap(), CpuKind::Scalar);
    assert_eq!("cached".parse::<CpuKind>().unwrap(), CpuKind::Cached);
    assert_eq!(
        "cached-relaxed".parse::<CpuKind>().unwrap(),
        CpuKind::CachedRelaxed
    );
    if cfg!(feature = "jit") {
        assert_eq!("jit".parse::<CpuKind>().unwrap(), CpuKind::Jit);
        assert_eq!(run_guest(CpuKind::Jit), run_guest(CpuKind::Cached));
    } else {
        assert!(
            "jit"
                .parse::<CpuKind>()
                .unwrap_err()
                .contains("--features jit")
        );
    }
    assert!("fast".parse::<CpuKind>().is_err());
}

fn run_guest(kind: CpuKind) -> [u32; 32] {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = AnyCpu::new(kind, bus);
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 10));
    image.extend_from_slice(&encode_i(0x04, 2, 0, 20));
    image.extend_from_slice(&encode_r(0x03, 3, 1, 2));
    image.extend_from_slice(&encode_r(0x3E, 0, 0, 0)); // CPUID
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    cpu.load_program(0, &image);
    cpu.run(128);
    assert!(!cpu.is_running());
    let mut regs = [0u32; 32];
    for (i, r) in regs.iter_mut().enumerate() {
        *r = cpu.read_reg(i);
    }
    regs
}

#[test]
fn cached_cpu_observes_modified_code_after_cache_population() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = AnyCpu::new(CpuKind::Cached, bus);
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 1));
    image.extend_from_slice(&encode_i(0x04, 2, 0, 1));
    image.extend_from_slice(&encode_i(0x09, 0, 0, -8));
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0));
    cpu.load_program(0, &image);

    assert!(cpu.step_once()); // populate address 0
    assert!(cpu.step_once()); // populate address 4
    assert!(cpu.step_once()); // populate address 8, branch back to address 4
    cpu.write_mem_u32_be(4, u32::from_be_bytes(encode_i(0x04, 2, 0, 9)));
    assert!(cpu.step_once());
    assert_eq!(cpu.read_reg(2), 9);
    assert!(cpu.step_once()); // cached branch at address 8
    assert!(cpu.step_once()); // cached updated instruction at address 4
    assert_eq!(cpu.read_reg(2), 9);
    let (hits, misses) = cpu.instruction_cache_stats().unwrap();
    assert!(hits > 0);
    assert!(misses > 0);
}
#[test]
fn cached_cpu_invalidates_an_instruction_crossing_a_generation_page() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = AnyCpu::new(CpuKind::Cached, bus);
    let pc = 4094;
    cpu.load_program(pc, &encode_i(0x04, 2, 0, 1));
    cpu.set_pc(pc);
    assert!(cpu.step_once());
    assert_eq!(cpu.read_reg(2), 1);

    cpu.write_mem_u8(pc + 3, 9);
    cpu.set_pc(pc);
    assert!(cpu.step_once());
    assert_eq!(cpu.read_reg(2), 9);
}

#[test]
fn cached_interpreter_commits_simple_instruction_in_one_cycle() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = AnyCpu::new(CpuKind::Cached, bus);
    cpu.load_program(0, &encode_i(0x04, 1, 0, 7));
    cpu.run(1);
    assert_eq!(cpu.instructions_executed(), 1);
    assert_eq!(cpu.cycles(), 1);
    assert_eq!(cpu.read_reg(1), 7);
}
#[test]
fn cached_interpreter_executes_short_mode_and_returns_to_normal() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = AnyCpu::new(CpuKind::Cached, bus);
    let mut image = vec![
        0x91, 0x05, // S.LDI R1, 5
        0x92, 0x07, // S.LDI R2, 7
        0x13, 0x12, // S.ADD R3, R1, R2
        0xF0, 0x00, // S.RET
    ];
    image.extend_from_slice(&encode_r(0x03, 4, 3, 2));
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0));
    cpu.load_program(0, &image);
    cpu.set_instruction_mode(cpt32::cpu::InstructionMode::Short);

    cpu.run(5);
    assert_eq!(cpu.read_reg(3), 12);
    assert_eq!(cpu.read_reg(4), 19);
    assert_eq!(cpu.instruction_mode(), cpt32::cpu::InstructionMode::Normal);
    assert_eq!(cpu.pc(), 12);
    assert_eq!(cpu.instructions_executed(), 5);
    cpu.run(1);
    assert!(!cpu.is_running());
    assert_eq!(cpu.instructions_executed(), 6);
}

#[test]
fn cached_interpreter_preserves_multiply_and_divide_latency() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = AnyCpu::new(CpuKind::Cached, bus);
    let mut image = Vec::new();
    image.extend_from_slice(&encode_i(0x04, 1, 0, 2));
    image.extend_from_slice(&encode_i(0x04, 2, 0, 3));
    image.extend_from_slice(&encode_r(0x18, 3, 1, 2));
    image.extend_from_slice(&encode_r(0x19, 4, 3, 2));
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0));
    cpu.load_program(0, &image);

    cpu.run(2);
    assert_eq!(cpu.instructions_executed(), 2);
    assert_eq!(cpu.cycles(), 2);
    cpu.run(3);
    assert_eq!(cpu.instructions_executed(), 3);
    assert_eq!(cpu.cycles(), 5);
    cpu.run(16);
    assert_eq!(cpu.instructions_executed(), 4);
    assert_eq!(cpu.cycles(), 21);
    assert_eq!(cpu.read_reg(3), 6);
    assert_eq!(cpu.read_reg(4), 2);
    cpu.run(1);
    assert!(!cpu.is_running());
    assert_eq!(cpu.instructions_executed(), 5);
    assert_eq!(cpu.cycles(), 22);
}
#[test]
fn any_cpu_runs_the_same_program_on_scalar_cached_and_pipeline_modes() {
    let scalar = run_guest(CpuKind::Scalar);
    let cached = run_guest(CpuKind::Cached);
    let pipelined = run_guest(CpuKind::Pipelined);
    assert_eq!(scalar[3], 30);
    assert_eq!(cached[3], 30);
    assert_eq!(pipelined[3], 30);
    assert_eq!(scalar[2], 0x33F);
    assert_eq!(cached[2], scalar[2]);
    assert_eq!(pipelined[2], 0x33F | 0x80);
}

static CONFIG_SEQ: AtomicU64 = AtomicU64::new(0);

fn scratch_config(contents: &str) -> std::path::PathBuf {
    let id = CONFIG_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "cpt32-cpu-kind-test-{}-{}.yaml",
        std::process::id(),
        id
    ));
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn resolve_prefers_cli_then_config_then_default() {
    let scalar_cfg = scratch_config("cpu:\n  microarchitecture: scalar\n");
    let pipeline_cfg = scratch_config("cpu:\n  microarchitecture: pipeline\n");
    let cached_cfg = scratch_config("cpu:\n  microarchitecture: cached\n");
    let empty_cfg = scratch_config("pcg: {}\n");
    let missing = std::env::temp_dir().join(format!(
        "cpt32-cpu-kind-missing-{}-{}.yaml",
        std::process::id(),
        CONFIG_SEQ.fetch_add(1, Ordering::SeqCst)
    ));

    // No config file at all -> pipeline default.
    assert_eq!(
        resolve_cpu_kind(None, &missing).unwrap(),
        CpuKind::Pipelined
    );
    // Config without a cpu section -> pipeline default.
    assert_eq!(
        resolve_cpu_kind(None, &empty_cfg).unwrap(),
        CpuKind::Pipelined
    );
    // Config selects scalar.
    assert_eq!(
        resolve_cpu_kind(None, &scalar_cfg).unwrap(),
        CpuKind::Scalar
    );
    // Config selects pipeline explicitly.
    assert_eq!(
        resolve_cpu_kind(None, &pipeline_cfg).unwrap(),
        CpuKind::Pipelined
    );
    // Config selects cached interpretation explicitly.
    assert_eq!(
        resolve_cpu_kind(None, &cached_cfg).unwrap(),
        CpuKind::Cached
    );
    // CLI wins over config either way.
    assert_eq!(
        resolve_cpu_kind(Some(CpuKind::Pipelined), &scalar_cfg).unwrap(),
        CpuKind::Pipelined
    );
    assert_eq!(
        resolve_cpu_kind(Some(CpuKind::Scalar), &pipeline_cfg).unwrap(),
        CpuKind::Scalar
    );
    assert_eq!(
        resolve_cpu_kind(Some(CpuKind::Cached), &scalar_cfg).unwrap(),
        CpuKind::Cached
    );

    std::fs::remove_file(&scalar_cfg).ok();
    std::fs::remove_file(&cached_cfg).ok();
    std::fs::remove_file(&pipeline_cfg).ok();
    std::fs::remove_file(&empty_cfg).ok();
}

#[test]
fn resolve_rejects_unknown_microarchitecture() {
    let bad_cfg = scratch_config("cpu:\n  microarchitecture: superscalar\n");
    assert!(resolve_cpu_kind(None, &bad_cfg).is_err());
    std::fs::remove_file(&bad_cfg).ok();
}

#[test]
fn jit_block_size_defaults_and_validates_configured_values() {
    let missing = std::env::temp_dir().join(format!(
        "cpt32-jit-size-missing-{}-{}.yaml",
        std::process::id(),
        CONFIG_SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    assert_eq!(resolve_jit_block_size(&missing).unwrap(), 16);
    for (yaml, expected) in [
        ("cpu:\n  jit_block_size: 8\n", 8),
        ("cpu:\n  jit_block_size: 32\n", 32),
        ("cpu:\n  jit_block_size: 64\n", 64),
        ("cpu:\n  jit_block_size: 256\n", 256),
    ] {
        let path = scratch_config(yaml);
        assert_eq!(resolve_jit_block_size(&path).unwrap(), expected);
        std::fs::remove_file(path).ok();
    }
    for yaml in [
        "cpu:\n  jit_block_size: 1\n",
        "cpu:\n  jit_block_size: 257\n",
        "cpu:\n  jit_block_size: nope\n",
    ] {
        let path = scratch_config(yaml);
        assert!(resolve_jit_block_size(&path).is_err());
        std::fs::remove_file(path).ok();
    }
}

#[test]
fn cpu_stage_profile_is_opt_in_and_preserves_all_microarchitectures() {
    for kind in [CpuKind::Scalar, CpuKind::Cached, CpuKind::Pipelined] {
        let mut profiled = AnyCpu::new(kind, {
            let mut bus = Bus::new();
            connect_ram(&mut bus);
            bus
        });
        let mut reference = AnyCpu::new(kind, {
            let mut bus = Bus::new();
            connect_ram(&mut bus);
            bus
        });
        let program = [0u8; 256];
        profiled.load_program(0, &program);
        reference.load_program(0, &program);

        assert!(profiled.take_profile().is_none());
        profiled.enable_profiling(true);
        profiled.run(40);
        reference.run(40);

        let profile = profiled.take_profile().unwrap();
        assert_eq!(profile.ticks, 40, "{kind:?}");
        assert!(!profile.total().is_zero(), "{kind:?}");
        assert!(
            profile.stages[1..]
                .iter()
                .any(|duration| !duration.is_zero())
        );
        assert_eq!(profiled.cycles(), reference.cycles(), "{kind:?}");
        assert_eq!(
            profiled.instructions_executed(),
            reference.instructions_executed(),
            "{kind:?}"
        );
        for register in 0..32 {
            assert_eq!(
                profiled.read_reg(register),
                reference.read_reg(register),
                "{kind:?} R{register}"
            );
        }
        let next_profile = profiled.take_profile().unwrap();
        assert_eq!(next_profile.ticks, 0, "{kind:?}");
        assert!(next_profile.total().is_zero(), "{kind:?}");
    }
}

#[test]
fn cpu_reset_clears_cycle_and_instruction_totals_for_all_modes() {
    for kind in [CpuKind::Scalar, CpuKind::Cached, CpuKind::Pipelined] {
        let mut bus = Bus::new();
        connect_ram(&mut bus);
        let mut cpu = AnyCpu::new(kind, bus);
        cpu.load_program(0, &[0; 256]);
        cpu.run(40);

        assert!(cpu.cycles() > 0, "{kind:?}");
        assert!(cpu.running_cycles() > 0, "{kind:?}");
        assert!(cpu.instructions_executed() > 0, "{kind:?}");
        cpu.reset(0);
        assert_eq!(cpu.cycles(), 0, "{kind:?}");
        assert_eq!(cpu.running_cycles(), 0, "{kind:?}");
        assert_eq!(cpu.instructions_executed(), 0, "{kind:?}");
    }
}
