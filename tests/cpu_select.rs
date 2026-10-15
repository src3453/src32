use cpt32::bus::Bus;
use cpt32::cpu_select::{AnyCpu, CpuKind, resolve_cpu_kind};
use cpt32::devices::ram::connect_ram;
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
fn pipeline_is_the_default_kind() {
    assert_eq!(CpuKind::default(), CpuKind::Pipelined);
    assert_eq!("pipeline".parse::<CpuKind>().unwrap(), CpuKind::Pipelined);
    assert_eq!("scalar".parse::<CpuKind>().unwrap(), CpuKind::Scalar);
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
fn any_cpu_runs_the_same_program_on_both_kinds() {
    let scalar = run_guest(CpuKind::Scalar);
    let pipelined = run_guest(CpuKind::Pipelined);
    assert_eq!(scalar[3], 30);
    assert_eq!(pipelined[3], 30);
    assert_eq!(scalar[2], 0x3F);
    assert_eq!(pipelined[2], 0x3F | 0x80);
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
    // CLI wins over config either way.
    assert_eq!(
        resolve_cpu_kind(Some(CpuKind::Pipelined), &scalar_cfg).unwrap(),
        CpuKind::Pipelined
    );
    assert_eq!(
        resolve_cpu_kind(Some(CpuKind::Scalar), &pipeline_cfg).unwrap(),
        CpuKind::Scalar
    );

    std::fs::remove_file(&scalar_cfg).ok();
    std::fs::remove_file(&pipeline_cfg).ok();
    std::fs::remove_file(&empty_cfg).ok();
}

#[test]
fn resolve_rejects_unknown_microarchitecture() {
    let bad_cfg = scratch_config("cpu:\n  microarchitecture: superscalar\n");
    assert!(resolve_cpu_kind(None, &bad_cfg).is_err());
    std::fs::remove_file(&bad_cfg).ok();
}
