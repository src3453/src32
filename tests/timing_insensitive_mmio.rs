//! Optimizations for timing-insensitive MMIO devices (VDP/VRAM, VPU, SGU, RNG,
//! UART, plain memory) must be invisible: the same guest runs in lockstep
//! with them disabled, for every CPU kind.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::pec::idc::idc::connect_idc;
use cpt32::devices::pec::pec::connect_pec_input;
use cpt32::devices::vdp::vdp::{VDP_REG_BASE, VDP_VRAM_BASE};

const PEC: u32 = 0x8004_0000;
const IRQC: u32 = 0xFFFF_0000;
const BMC: u32 = 0x8006_0000;
const HANDLER: u32 = 0x800;
const PCM_RAM: u32 = 0x1800_0000;

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

/// VRAM/PCM RAM word and byte traffic, VDP register writes and reads, plus a
/// periodic timer IRQ (counted in R20) and a sensitive timer-counter read.
fn program() -> Vec<u8> {
    let mut words = Vec::new();
    load_constant(&mut words, 10, VDP_VRAM_BASE);
    load_constant(&mut words, 11, VDP_REG_BASE);
    load_constant(&mut words, 12, PCM_RAM);
    load_constant(&mut words, 13, 0x3FFC);
    load_constant(&mut words, 21, PEC);
    load_constant(&mut words, 22, IRQC);
    load_constant(&mut words, 30, HANDLER);
    load_constant(&mut words, 9, 6000);
    words.push(i(0x04, 24, 0, 1 << 4));
    words.push(i(0x16, 24, 22, 0x44)); // IRQC enable source 4
    store(&mut words, 777, 21, 0x410); // timer compare
    store(&mut words, 777, 21, 0x418); // timer reload
    store(&mut words, 1 | 4 | 8, 21, 0x400); // periodic, enable, IRQ
    store(&mut words, 1, 21, 0x184);
    let head = words.len() as i16;
    words.extend([
        r(0x0c, 5, 1, 13),
        r(0x03, 6, 5, 10),
        i(0x02, 1, 6, 0), // ST VRAM
        i(0x16, 4, 6, 6), // STH VRAM
        r(0x03, 7, 5, 12),
        i(0x02, 4, 7, 0),  // ST PCM RAM
        i(0x01, 8, 7, 0),  // LD PCM RAM
        i(0x15, 1, 11, 3), // STB VDP border color
        i(0x13, 3, 11, 3), // LDB VDP border color
        i(0x01, 2, 6, 0),  // LD VRAM
        r(0x0e, 4, 4, 2),
        r(0x03, 4, 4, 8),
        i(0x01, 15, 21, 0x408), // LD timer count (timing-sensitive)
        r(0x03, 4, 4, 15),
        i(0x04, 1, 1, 4),
        i(0x04, 9, 9, -1),
    ]);
    words.push(i(0x08, 0, 9, (head - words.len() as i16 - 1) * 4));
    store(&mut words, 0, 21, 0x400);
    words.push(r(0x3f, 0, 0, 0));
    bytes(&words)
}

fn handler() -> Vec<u8> {
    bytes(&[
        i(0x04, 20, 20, 1),
        i(0x04, 23, 0, 1),
        i(0x02, 23, 21, 0x18c),
        i(0x04, 23, 0, 1 << 4),
        i(0x16, 23, 22, 0x40),
        0x8000_0000,
    ])
}

fn machine(kind: CpuKind, fast: bool) -> AnyCpu {
    let mut bus = Bus::new();
    let _input = connect_pec_input(&mut bus);
    connect_devices_with_vdp(&mut bus);
    connect_idc(&mut bus, std::array::from_fn(|_| None));
    let mut cpu = AnyCpu::new(kind, bus);
    cpu.set_timing_insensitive_fast_paths(fast);
    cpu.load_program(0, &program());
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
    assert_eq!(a.epc(), b.epc(), "{context}");
    assert_eq!(a.irq_cause(), b.irq_cause(), "{context}");
    assert_eq!(a.pipeline_stats(), b.pipeline_stats(), "{context}");
    assert_eq!(
        a.instruction_cache_stats(),
        b.instruction_cache_stats(),
        "{context}"
    );
    for address in [
        BMC + 0x08,
        BMC + 0x0c,
        BMC + 0x10,
        IRQC + 0x40,
        PEC + 0x188,
        PEC + 0x408,
        VDP_REG_BASE,
    ] {
        assert_eq!(
            a.read_debug_mem_u32_be(address),
            b.read_debug_mem_u32_be(address),
            "{address:#x} {context}"
        );
    }
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
fn timing_insensitive_fast_paths_match_sensitive_execution() {
    for kind in kinds() {
        let mut reference = machine(kind, false);
        let mut fast = machine(kind, true);
        let budgets = [1, 13, 977, 50_000, 3, 200_000, 776, 777, 778];
        let mut step = 0;
        while reference.is_running() && step < 20_000 {
            let budget = budgets[step % budgets.len()];
            reference.run(budget);
            fast.run(budget);
            same(&mut reference, &mut fast, step);
            step += 1;
        }
        assert!(!reference.is_running(), "{kind:?} did not finish");
        assert!(
            reference.read_reg(20) > 20,
            "timer IRQs: {}",
            reference.read_reg(20)
        );
        for (base, words) in [(VDP_VRAM_BASE, 0x1000), (PCM_RAM, 0x1000)] {
            for word in 0..words {
                let address = base + word * 4;
                assert_eq!(
                    reference.read_debug_mem_u32_be(address),
                    fast.read_debug_mem_u32_be(address),
                    "{kind:?} {address:#x}"
                );
            }
        }
    }
}
