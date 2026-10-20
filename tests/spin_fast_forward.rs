//! Spin-loop fast-forward must be invisible: a polled IDC/DMA transfer with
//! periodic timer IRQs runs in lockstep with and without it, for every CPU
//! kind, and every observable counter, register, device status and RAM byte
//! must match at uneven run-budget boundaries.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::pec::idc::idc::{Idc, connect_idc};
use cpt32::devices::pec::pec::connect_pec_input;
use cpt32::devices::pec::rng::connect_rng;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const DMAC: u32 = 0x8005_0000;
const IDC: u32 = 0x8004_1000;
const PEC: u32 = 0x8004_0000;
const IRQC: u32 = 0xFFFF_0000;
const BUFFER: u32 = 0x0010_0000;
const HANDLER: u32 = 0x800;
const SECTORS: u32 = 3;

static SEQ: AtomicU32 = AtomicU32::new(0);

fn r(op: u32, rd: u32, a: u32, b: u32) -> u32 {
    (op << 26) | (rd << 21) | (a << 16) | (b << 11)
}

fn i(op: u32, rd: u32, a: u32, imm: i16) -> u32 {
    (op << 26) | (rd << 21) | (a << 16) | u32::from(imm as u16)
}

fn load_constant(words: &mut Vec<u32>, rd: u32, value: u32) {
    words.push(i(0x3d, rd, 0, (value >> 16) as i16));
    words.push(i(0x3c, rd, 0, value as i16));
}

fn store(words: &mut Vec<u32>, value: u32, base: u32, offset: i16) {
    load_constant(words, 1, value);
    words.push(i(0x02, 1, base, offset));
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

/// Periodic PeC timer IRQ4 (counted in R20) during a polled DMA READ, the
/// same register sequence as `sol/cpt32/idc.sol` `idc_read`, then HALT.
fn main_program() -> Vec<u8> {
    let mut words = Vec::new();
    load_constant(&mut words, 10, DMAC);
    load_constant(&mut words, 11, IDC);
    load_constant(&mut words, 21, PEC);
    load_constant(&mut words, 22, IRQC);
    load_constant(&mut words, 30, HANDLER); // every IRQ vector is JR R30
    words.push(i(0x04, 24, 0, 1 << 4)); // IRQC source 4 (PeC timer)
    words.push(i(0x16, 24, 22, 0x44)); // IRQC enable
    store(&mut words, 4999, 21, 0x410); // timer 0 compare
    store(&mut words, 4999, 21, 0x418); // timer 0 reload
    store(&mut words, 1 | 4 | 8, 21, 0x400); // periodic, enable, IRQ
    store(&mut words, 1, 21, 0x184); // timer channel 0 enable
    store(&mut words, 1, 10, 0x04);
    store(&mut words, 1, 10, 0x08);
    store(&mut words, 6, 10, 0x110);
    store(&mut words, 6, 11, 0x08);
    store(&mut words, 0, 11, 0x14);
    store(&mut words, 0, 11, 0x1c);
    store(&mut words, SECTORS, 11, 0x20);
    store(&mut words, 0x8004_1024, 10, 0x100);
    store(&mut words, BUFFER, 10, 0x104);
    store(&mut words, SECTORS * 512, 10, 0x108);
    store(&mut words, 1, 11, 0x18);
    store(&mut words, 0x104, 10, 0x10c);
    words.push(i(0x04, 2, 0, 1));
    // Inline poll of the DMAC channel status.
    let poll = words.len() as i16;
    words.push(i(0x01, 3, 10, 0x110));
    words.push(r(0x0c, 3, 3, 2));
    words.push(i(0x08, 0, 3, (poll - words.len() as i16 - 1) * 4));
    // Call-based poll of the IDC status, like the sol driver: each iteration
    // takes a call, a return and the back-jump.
    let call_loop = words.len() as i16;
    let call_at = words.len();
    words.push(0); // JAL poll_status (patched below)
    words.push(i(0x08, 0, 3, (call_loop - words.len() as i16 - 1) * 4));
    store(&mut words, 0, 21, 0x400); // stop the timer
    words.push(r(0x3f, 0, 0, 0)); // HALT
    let poll_status = words.len() as i16;
    words.push(i(0x01, 3, 11, 0x08));
    words.push(r(0x0c, 3, 3, 2));
    words.push(r(0x0b, 31, 0, 0)); // JR R31
    words[call_at] = i(0x0a, 0, 0, (poll_status - call_at as i16 - 1) * 4);
    bytes(&words)
}

fn handler() -> Vec<u8> {
    bytes(&[
        i(0x04, 20, 20, 1),
        i(0x04, 23, 0, 1),
        i(0x02, 23, 21, 0x18c), // timer ACK
        i(0x04, 23, 0, 1 << 4),
        i(0x16, 23, 22, 0x40), // IRQC pending W1C
        0x8000_0000,           // IRET
    ])
}

fn config(kind: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cpt32-spin-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let image: Vec<u8> = (0..SECTORS * 512)
        .map(|index| (index.wrapping_mul(2_654_435_761) >> 11) as u8)
        .collect();
    std::fs::write(dir.join("disk.img"), image).unwrap();
    // Fast rates keep the test short while leaving long quiet windows.
    std::fs::write(
        dir.join("config.yaml"),
        format!(
            "disk:\n  - slot: 0\n    type: {kind}\n    image: disk.img\n    transfer_rate_bytes_per_second: 600000\n    access_latency_ms: 2\n"
        ),
    )
    .unwrap();
    dir.join("config.yaml")
}

fn machine(kind: CpuKind, config: &Path, fast_forward: bool) -> AnyCpu {
    let mut bus = Bus::new();
    let _input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    connect_devices_with_vdp(&mut bus);
    connect_idc(&mut bus, Idc::load_drives(config).unwrap());
    let mut cpu = AnyCpu::new(kind, bus);
    cpu.set_spin_fast_forward(fast_forward);
    cpu.load_program(0, &main_program());
    cpu.load_program(HANDLER, &handler());
    for vector in 0..16 {
        cpu.write_mem_u32_be(0xFFFF_0100 + vector * 4, r(0x0b, 30, 0, 0));
    }
    cpu.set_pc(0);
    cpu
}

fn same(a: &mut AnyCpu, b: &mut AnyCpu, step: usize) {
    let context = format!("{:?} step {step} pc={:#x}", a.kind(), a.pc());
    for reg in 0..32 {
        assert_eq!(a.read_reg(reg), b.read_reg(reg), "R{reg} {context}");
    }
    assert_eq!(a.pc(), b.pc(), "{context}");
    assert_eq!(a.cycles(), b.cycles(), "{context}");
    assert_eq!(a.running_cycles(), b.running_cycles(), "{context}");
    assert_eq!(
        a.instructions_executed(),
        b.instructions_executed(),
        "{context}"
    );
    assert_eq!(a.is_running(), b.is_running(), "{context}");
    assert_eq!(a.irq_enabled(), b.irq_enabled(), "{context}");
    assert_eq!(a.epc(), b.epc(), "{context}");
    assert_eq!(a.irq_cause(), b.irq_cause(), "{context}");
    assert_eq!(a.pipeline_stats(), b.pipeline_stats(), "{context}");
    assert_eq!(
        a.instruction_cache_stats(),
        b.instruction_cache_stats(),
        "{context}"
    );
    for address in [
        DMAC + 0x110,
        DMAC + 0x114,
        DMAC + 0x10,
        IDC + 0x08,
        IDC + 0x10,
        IRQC + 0x40,
        PEC + 0x188,
        PEC + 0x408,
        BUFFER,
        BUFFER + 508,
        BUFFER + SECTORS * 512 - 4,
    ] {
        assert_eq!(
            a.read_debug_mem_u32_be(address),
            b.read_debug_mem_u32_be(address),
            "{address:#x} {context}"
        );
    }
}

fn lockstep(kind: CpuKind, drive: &str) {
    let config = config(drive);
    let mut reference = machine(kind, &config, false);
    let mut fast = machine(kind, &config, true);
    let budgets = [1, 13, 977, 50_000, 3, 200_000, 4999, 5000, 5001];
    let mut step = 0;
    while (reference.is_running() || step < 40) && step < 20_000 {
        let budget = budgets[step % budgets.len()];
        reference.run(budget);
        fast.run(budget);
        same(&mut reference, &mut fast, step);
        step += 1;
    }
    assert!(!reference.is_running(), "{kind:?} did not finish");
    assert_eq!(reference.read_reg(3), 0);
    assert!(
        reference.read_reg(20) > 10,
        "timer IRQs: {}",
        reference.read_reg(20)
    );
    for word in 0..SECTORS * 128 {
        let address = BUFFER + word * 4;
        assert_eq!(
            reference.read_debug_mem_u32_be(address),
            fast.read_debug_mem_u32_be(address)
        );
    }
    assert_eq!(reference.spin_skipped_cycles(), 0);
    assert!(
        fast.spin_skipped_cycles() > 100_000,
        "{kind:?} skipped {}",
        fast.spin_skipped_cycles()
    );
    std::fs::remove_dir_all(config.parent().unwrap()).ok();
}

fn kinds() -> Vec<CpuKind> {
    let mut kinds = vec![
        CpuKind::Pipelined,
        CpuKind::Scalar,
        CpuKind::Cached,
        CpuKind::CachedRelaxed,
    ];
    if cfg!(feature = "jit") {
        kinds.push(CpuKind::Jit);
    }
    kinds
}

#[test]
fn polled_fdd_read_with_timer_irqs_matches_without_fast_forward() {
    for kind in kinds() {
        lockstep(kind, "fdd");
    }
}

#[test]
fn polled_hdd_read_with_timer_irqs_matches_without_fast_forward() {
    for kind in kinds() {
        lockstep(kind, "hdd");
    }
}

#[test]
fn host_register_and_pc_rewrites_between_runs_are_not_spin_iterations() {
    // JALR R31 at 128: each host rewrite of R31/PC recreates the same loop
    // head without any CPU iteration, so nothing may be skipped.
    for kind in kinds() {
        let config = config("hdd");
        let mut reference = machine(kind, &config, false);
        let mut fast = machine(kind, &config, true);
        for cpu in [&mut reference, &mut fast] {
            cpu.write_mem_u32_be(128, r(0x21, 31, 0, 0));
        }
        for step in 0..40 {
            for cpu in [&mut reference, &mut fast] {
                cpu.write_reg(31, 0).unwrap();
                cpu.set_pc(128);
                cpu.run(1);
            }
            same(&mut reference, &mut fast, step);
        }
        assert_eq!(fast.spin_skipped_cycles(), 0, "{kind:?}");
        std::fs::remove_dir_all(config.parent().unwrap()).ok();
    }
}
