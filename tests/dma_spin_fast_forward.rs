//! Spin-loop fast-forward during an active DMA transfer must be invisible.
//! A C-like loop polls DMAC STATUS (with stack spills) while memory copies,
//! BitBlts, colour-keyed and ROP transfers run and a periodic timer IRQ
//! interrupts the loop. Every CPU kind runs in lockstep with and without the
//! fast-forward, and registers, counters, DMAC/BMC state and memory must
//! match at uneven run-budget boundaries. Loops that read DMA progress
//! (REMAIN) or the destination memory must still behave exactly.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::pec::pec::connect_pec_input;
use cpt32::devices::pec::rng::connect_rng;

const DMAC: u32 = 0x8005_0000;
const BMC: u32 = 0x8006_0000;
const PEC: u32 = 0x8004_0000;
const IRQC: u32 = 0xFFFF_0000;
const HANDLER: u32 = 0x800;
const STACK: u32 = 0x0000_8000;
const SRC: u32 = 0x0010_0000;
const SRC_BYTES: u32 = 0x1_0000;
const RAM_DST: u32 = 0x0030_0000;
const VRAM: u32 = 0x1000_0000;
/// Word the hazard loop polls inside the destination of job `hazard`.
const WATCH_DST: u32 = RAM_DST + 0x8000;
const WATCH: u32 = WATCH_DST + 0x2000;

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

/// `[base + offset] = value` through R1.
fn store(words: &mut Vec<u32>, value: u32, base: u32, offset: i16) {
    load_constant(words, 1, value);
    words.push(i(0x02, 1, base, offset));
}

fn branch_back(words: &mut Vec<u32>, op: u32, a: u32, b: u32, target: usize) {
    let offset = (target as i32 - words.len() as i32 - 1) * 4;
    words.push(i(op, a, b, offset as i16));
}

/// Wait for channel 0 like compiled C: STATUS is loaded, spilled with word,
/// halfword and byte stores, reloaded and tested every iteration.
fn poll_done(words: &mut Vec<u32>) {
    let head = words.len();
    words.push(i(0x01, 1, 10, 0x110)); // LD R1, [CH0_STATUS]
    words.push(i(0x02, 1, 29, -4)); // ST [SP-4], R1
    words.push(i(0x16, 1, 29, -8)); // STH [SP-8], R1
    words.push(i(0x15, 1, 29, -12)); // STB [SP-12], R1
    words.push(i(0x01, 3, 29, -4)); // LD R3, [SP-4]
    words.push(r(0x0c, 3, 3, 2)); // AND R3, R3, R2 (BUSY)
    branch_back(words, 0x08, 0, 3, head); // BNE R3, R0
    store(words, 6, 10, 0x110); // clear DONE/ERROR
}

struct Job {
    src: u32,
    dst: u32,
    count: u32,
    control: u32,
    rows: u32,
    src_stride: u32,
    dst_stride: u32,
    key: u32,
}

fn job(src: u32, dst: u32, count: u32, control: u32) -> Job {
    Job {
        src,
        dst,
        count,
        control,
        rows: 0,
        src_stride: 0,
        dst_stride: 0,
        key: 0,
    }
}

fn start(words: &mut Vec<u32>, job: &Job) {
    store(words, job.rows, 10, 0x200);
    store(words, job.src_stride, 10, 0x204);
    store(words, job.dst_stride, 10, 0x208);
    store(words, job.key, 10, 0x20c);
    store(words, job.src, 10, 0x100);
    store(words, job.dst, 10, 0x104);
    store(words, job.count, 10, 0x108);
    store(words, job.control | (1 << 8), 10, 0x10c);
}

const WIDTH8: u32 = 0;
const WIDTH16: u32 = 1;
const WIDTH32: u32 = 2;
const SRC_FIXED: u32 = 1 << 2;
const MODE_2D: u32 = 1 << 6;
const KEY_EN: u32 = 1 << 7;
const ROP_XOR: u32 = 3 << 10;

fn jobs() -> Vec<Job> {
    vec![
        job(SRC, VRAM, 0x8000, WIDTH32),
        Job {
            rows: 50,
            src_stride: 256,
            dst_stride: 640,
            ..job(SRC + 2, VRAM + 0x1_0000, 200, WIDTH16 | MODE_2D)
        },
        Job {
            rows: 40,
            src_stride: 120,
            dst_stride: 320,
            key: 0xFFFF_FF00, // key 0: only the low WIDTH bits count
            ..job(
                SRC + 0x4000,
                VRAM + 0x2_0000,
                120,
                WIDTH8 | MODE_2D | KEY_EN,
            )
        },
        job(SRC, RAM_DST, 0x4000, WIDTH32 | ROP_XOR),
        Job {
            rows: 30,
            src_stride: 0,
            dst_stride: (-256i32) as u32,
            ..job(SRC + 8, VRAM + 0x3_8000, 64, WIDTH32 | SRC_FIXED | MODE_2D)
        },
        Job {
            rows: 16,
            src_stride: 64,
            dst_stride: 64,
            key: 0x0000_1234,
            ..job(
                SRC + 0x100,
                VRAM + 0x4_0000,
                48,
                WIDTH16 | MODE_2D | KEY_EN | ROP_XOR,
            )
        },
    ]
}

/// Timer IRQ every 5000 cycles (counted in R20) during a list of transfers,
/// then a loop that waits on REMAIN and one that polls destination memory.
fn program() -> Vec<u8> {
    let mut words = Vec::new();
    load_constant(&mut words, 10, DMAC);
    load_constant(&mut words, 21, PEC);
    load_constant(&mut words, 22, IRQC);
    load_constant(&mut words, 29, STACK);
    load_constant(&mut words, 30, HANDLER); // every IRQ vector is JR R30
    words.push(i(0x04, 2, 0, 1));
    words.push(i(0x04, 24, 0, 1 << 4)); // IRQC source 4 (PeC timer)
    words.push(i(0x16, 24, 22, 0x44)); // IRQC enable
    store(&mut words, 4999, 21, 0x410); // timer 0 compare
    store(&mut words, 4999, 21, 0x418); // timer 0 reload
    store(&mut words, 1 | 4 | 8, 21, 0x400); // periodic, enable, IRQ
    store(&mut words, 1, 21, 0x184); // timer channel 0 enable
    store(&mut words, 1, 10, 0x04); // DMAC enable
    store(&mut words, 1, 10, 0x08); // channel 0 enable
    for job in jobs() {
        start(&mut words, &job);
        poll_done(&mut words);
    }

    // Wait until REMAIN < 0x1000: the value read changes with every beat
    // while the comparison result repeats, so it must not be skipped over.
    start(&mut words, &job(SRC, VRAM + 0x5_0000, 0x4000, WIDTH32));
    load_constant(&mut words, 4, 0x1000);
    let head = words.len();
    words.push(i(0x01, 3, 10, 0x114)); // LD R3, [CH0_REMAIN]
    words.push(r(0x17, 3, 3, 4)); // SLTU R3, R3, R4
    branch_back(&mut words, 0x07, 3, 0, head); // BEQ R3, R0
    words.push(i(0x01, 5, 10, 0x114)); // R5 = REMAIN just after the exit
    poll_done(&mut words);

    // Wait until the transfer reaches WATCH in its destination.
    start(&mut words, &job(SRC, WATCH_DST, 0x4000, WIDTH32));
    let head = words.len();
    load_constant(&mut words, 6, WATCH);
    words.push(i(0x01, 3, 6, 0)); // LD R3, [WATCH]
    branch_back(&mut words, 0x07, 3, 0, head); // BEQ R3, R0
    words.push(i(0x01, 7, 10, 0x114)); // R7 = REMAIN when it was seen
    poll_done(&mut words);

    store(&mut words, 0, 21, 0x400); // stop the timer
    words.push(r(0x3f, 0, 0, 0)); // HALT
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

fn handler() -> Vec<u8> {
    [
        i(0x04, 20, 20, 1),
        i(0x04, 23, 0, 1),
        i(0x02, 23, 21, 0x18c), // timer ACK
        i(0x04, 23, 0, 1 << 4),
        i(0x16, 23, 22, 0x40), // IRQC pending W1C
        0x8000_0000,           // IRET
    ]
    .iter()
    .flat_map(|word| word.to_be_bytes())
    .collect()
}

/// Source bytes with runs of zeros (transparent for key 0) and of 0x34/0x12.
fn source() -> Vec<u8> {
    (0..SRC_BYTES)
        .map(|index| {
            let hash = index.wrapping_mul(2_654_435_761) >> 13;
            match hash % 7 {
                0 | 1 => 0,
                2 => 0x12,
                3 => 0x34,
                _ => hash as u8 | 1,
            }
        })
        .collect()
}

fn machine(kind: CpuKind, fast_forward: bool) -> AnyCpu {
    let mut bus = Bus::new();
    let _input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    connect_devices_with_vdp(&mut bus);
    let mut cpu = AnyCpu::new(kind, bus);
    cpu.set_spin_fast_forward(fast_forward);
    cpu.load_program(0, &program());
    cpu.load_program(HANDLER, &handler());
    cpu.load_program(SRC, &source());
    // The XOR destination and the watched word start as a known pattern.
    let pattern: Vec<u8> = (0..0x4000u32).map(|index| (index * 7) as u8).collect();
    cpu.load_program(RAM_DST, &pattern);
    cpu.load_program(WATCH, &[0; 4]);
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
    assert_eq!(a.pipeline_stats(), b.pipeline_stats(), "{context}");
    assert_eq!(
        a.instruction_cache_stats(),
        b.instruction_cache_stats(),
        "{context}"
    );
    for address in [
        DMAC + 0x10,
        DMAC + 0x110,
        DMAC + 0x114,
        DMAC + 0x210,
        BMC + 0x08,
        BMC + 0x0C,
        BMC + 0x10,
        IRQC + 0x40,
        PEC + 0x408,
        VRAM,
        VRAM + 0x7FFC,
        VRAM + 0x1_0000 + 640 * 25,
        VRAM + 0x2_0000 + 320 * 20,
        VRAM + 0x3_8000 - 256 * 10,
        RAM_DST + 0x1000,
        WATCH_DST + 0x3FFC,
        WATCH,
        STACK - 12,
        STACK - 8,
        STACK - 4,
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
fn dma_transfers_polled_with_timer_irqs_match_without_fast_forward() {
    for kind in kinds() {
        let mut reference = machine(kind, false);
        let mut fast = machine(kind, true);
        let budgets = [1, 13, 977, 50_000, 3, 200_000, 4999, 5000, 5001];
        let mut step = 0;
        while reference.is_running() && step < 20_000 {
            let budget = budgets[step % budgets.len()];
            reference.run(budget);
            fast.run(budget);
            same(&mut reference, &mut fast, step);
            step += 1;
        }
        assert!(!reference.is_running(), "{kind:?} did not finish");
        assert!(reference.read_reg(20) > 10, "{kind:?} timer IRQs");
        // The REMAIN loop exits just below the threshold; the memory loop
        // sees WATCH before the transfer ends.
        let remain = reference.read_reg(5);
        assert!(remain < 0x1000 && remain > 0xF00, "{kind:?} R5 {remain:#x}");
        let seen = reference.read_reg(7);
        assert!(seen > 0 && seen < 0x4000 - 0x2000, "{kind:?} R7 {seen:#x}");
        for (base, len) in [(VRAM, 0x6_0000), (RAM_DST, 0xC000), (STACK - 16, 16)] {
            for address in (base..base + len).step_by(4) {
                assert_eq!(
                    reference.read_debug_mem_u32_be(address),
                    fast.read_debug_mem_u32_be(address),
                    "{kind:?} {address:#x}"
                );
            }
        }
        assert_eq!(reference.spin_skipped_cycles(), 0);
        // Most transfer time is skipped on every CPU kind (the Pipeline keeps
        // the Main RAM destination: its L1 snoops DMA writes).
        assert!(
            fast.spin_skipped_cycles() > reference.running_cycles() / 2,
            "{kind:?} skipped {} of {}",
            fast.spin_skipped_cycles(),
            reference.running_cycles()
        );
    }
}
