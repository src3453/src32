#![cfg(feature = "jit")]

use cpt32::bus::Bus;
use cpt32::cpu::InstructionMode;
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::ram::connect_ram;

fn r(op: u32, rd: u32, a: u32, b: u32) -> u32 {
    (op << 26) | (rd << 21) | (a << 16) | (b << 11)
}

fn i(op: u32, rd: u32, a: u32, imm: i16) -> u32 {
    (op << 26) | (rd << 21) | (a << 16) | u32::from(imm as u16)
}

fn boot(kind: CpuKind, base: u32, words: &[u32]) -> AnyCpu {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = AnyCpu::new(kind, bus);
    let image: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    cpu.load_program(base, &image);
    cpu.set_pc(base);
    cpu
}

fn same(a: &AnyCpu, b: &AnyCpu) {
    for reg in 0..32 {
        assert_eq!(a.read_reg(reg), b.read_reg(reg), "R{reg}, PC={:#x}", a.pc());
    }
    assert_eq!(a.pc(), b.pc());
    assert_eq!(a.instruction_mode(), b.instruction_mode());
    assert_eq!(a.cycles(), b.cycles());
    assert_eq!(a.running_cycles(), b.running_cycles());
    assert_eq!(a.instructions_executed(), b.instructions_executed());
    assert_eq!(a.is_running(), b.is_running());
    assert_eq!(a.epc(), b.epc());
    assert_eq!(a.irq_cause(), b.irq_cause());
    assert_eq!(a.irq_enabled(), b.irq_enabled());
    assert_eq!(a.instruction_cache_stats(), b.instruction_cache_stats());
}

fn native_ran(cpu: &AnyCpu) {
    let stats = cpu.jit_stats().unwrap();
    assert_eq!(stats.compile_failures, 0, "{stats:?}");
    assert!(
        stats.compilations > 0 && stats.native_instructions > 0,
        "{stats:?}"
    );
}

#[test]
fn native_alu_matches_cached_at_every_cycle_with_randomized_operands() {
    let ops = [
        0x03, 0x05, 0x06, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x17,
    ];
    let mut seed = 0x1357_9bdfu32;
    let mut words = vec![0];
    for index in 0..64 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        words.push(r(
            ops[index % ops.len()],
            seed & 31,
            (seed >> 5) & 31,
            (seed >> 10) & 31,
        ));
        words.push(i(0x04, (seed >> 15) & 31, (seed >> 20) & 31, seed as i16));
        words.push(i(0x3c, (seed >> 25) & 31, 0, seed as i16));
        words.push(i(0x3d, (seed >> 25) & 31, 0, (seed >> 16) as i16));
    }
    words.push(r(0x3e, 0, 0, 0)); // native CPUID preserves the Cached feature bits
    words.push(i(0x09, 0, 0, -((words.len() as i16 + 1) * 4)));
    let mut cached = boot(CpuKind::Cached, 0, &words);
    let mut jit = boot(CpuKind::Jit, 0, &words);
    for reg in 1..32 {
        let value = [0, 1, u32::MAX, 0x8000_0000, 31, 32, 255][reg % 7];
        cached.write_reg(reg, value).unwrap();
        jit.write_reg(reg, value).unwrap();
    }
    // Heat every block, then check each native instruction boundary. run(1)
    // exercises native budget exits rather than the debugger's interpreter step.
    for budget in [20_000, 1, 2, 7, 16, 23, 1, 64] {
        cached.run(budget);
        jit.run(budget);
        same(&cached, &jit);
    }
    for _ in 0..3000 {
        cached.run(1);
        jit.run(1);
        same(&cached, &jit);
    }
    native_ran(&jit);
}

#[test]
fn native_branch_call_and_return_match_cached() {
    let words = [
        i(0x04, 1, 1, 1),
        i(0x07, 0, 1, 8), // not taken
        i(0x08, 0, 1, 4), // taken
        i(0x04, 2, 2, 1), // skipped
        i(0x0a, 0, 0, 8), // call 28
        i(0x04, 3, 3, 1),
        i(0x09, 0, 0, -28),
        i(0x04, 4, 4, 1),
        r(0x0b, 31, 0, 0),
    ];
    let mut cached = boot(CpuKind::Cached, 0, &words);
    let mut jit = boot(CpuKind::Jit, 0, &words);
    for _ in 0..200 {
        cached.run(31);
        jit.run(31);
        same(&cached, &jit);
    }
    assert_eq!(jit.read_reg(2), 0);
    native_ran(&jit);
    // JALR R31 must read the old target before writing its return address.
    for cpu in [&mut cached, &mut jit] {
        cpu.write_mem_u32_be(128, r(0x21, 31, 0, 0));
        cpu.write_reg(31, 0).unwrap();
        cpu.set_pc(128);
    }
    for _ in 0..20 {
        for cpu in [&mut cached, &mut jit] {
            cpu.write_reg(31, 0).unwrap();
            cpu.set_pc(128);
            cpu.run(1);
        }
        same(&cached, &jit);
        assert_eq!(jit.read_reg(31), 132);
        assert_eq!(jit.pc(), 0);
    }
}

#[test]
fn native_blocks_observe_code_replacement_and_cross_page_changes() {
    for base in [0, 4094] {
        let words = [i(0x04, 1, 1, 1), i(0x04, 2, 2, 1), i(0x09, 0, 0, -12)];
        let mut cached = boot(CpuKind::Cached, base, &words);
        let mut jit = boot(CpuKind::Jit, base, &words);
        cached.run(300);
        jit.run(300);
        native_ran(&jit);
        for cpu in [&mut cached, &mut jit] {
            cpu.write_mem_u32_be(base + 4, i(0x04, 2, 2, 9));
            cpu.set_pc(base);
        }
        cached.run(300);
        jit.run(300);
        same(&cached, &jit);
        assert!(jit.jit_stats().unwrap().invalidations > 0);
        // load_program must also invalidate executable code through RAM writes.
        for cpu in [&mut cached, &mut jit] {
            cpu.load_program(base, &i(0x04, 1, 1, 13).to_be_bytes());
            cpu.set_pc(base);
        }
        cached.run(300);
        jit.run(300);
        same(&cached, &jit);
        for cpu in [&mut cached, &mut jit] {
            cpu.reset(base);
            cpu.run(300);
        }
        same(&cached, &jit);
        assert!(jit.jit_stats().unwrap().cache_resets > 0);
    }
}

#[test]
fn fallback_memory_arithmetic_short_mode_and_halt_preserve_cycles() {
    let words = [
        i(0x04, 1, 0, 2),
        i(0x04, 2, 0, 3),
        r(0x18, 3, 1, 2),
        r(0x19, 4, 3, 2),
        i(0x02, 4, 0, 1024),
        i(0x01, 5, 0, 1024),
        i(0x1e, 0, 0, 0), // Short mode at 28, LR=28
        0x2601_f000,      // S.ADDI R6,1; S.RET changes mode at next PC
        r(0x3e, 0, 0, 0),
        r(0x3f, 0, 0, 0),
    ];
    let mut cached = boot(CpuKind::Cached, 0, &words);
    let mut jit = boot(CpuKind::Jit, 0, &words);
    for _ in 0..50 {
        cached.run(1);
        jit.run(1);
        same(&cached, &jit);
    }
    assert_eq!(jit.read_reg(2), 0x3f);
    assert_eq!(jit.read_reg(5), 2);
    assert_eq!(jit.read_reg(6), 1);
    assert_eq!(jit.instruction_mode(), InstructionMode::Normal);
    assert!(!jit.is_running());
    assert_eq!(cached.read_mem_u32_be(1024), jit.read_mem_u32_be(1024));
}

#[test]
fn native_irq_iret_and_debugger_register_sync_match_cached() {
    let words = [i(0x04, 1, 1, 1), i(0x09, 0, 0, -8)];
    let mut cached = boot(CpuKind::Cached, 0, &words);
    let mut jit = boot(CpuKind::Jit, 0, &words);
    for cpu in [&mut cached, &mut jit] {
        cpu.write_mem_u32_be(0xffff_010c, 0x8000_0000); // IRET
        cpu.run(300);
    }
    native_ran(&jit);
    for n in 0..64 {
        for cpu in [&mut cached, &mut jit] {
            cpu.set_irq_input(true, 3);
            cpu.run(1);
            cpu.set_irq_input(false, 3);
            cpu.run(3);
            // Guest-visible CPU register MMIO can replace the next PC.
            cpu.write_mem_u32_be(0xffff_0200, n); // R0 remains zero
            cpu.set_pc(0);
        }
        same(&cached, &jit);
        assert!(jit.irq_enabled());
    }
    for cpu in [&mut cached, &mut jit] {
        cpu.write_mem_u32_be(0xffff_0280, 4); // replace PC via register MMIO
        cpu.write_mem_u32_be(0xffff_0204, 0x8000_0000); // R1
        cpu.run(3);
    }
    same(&cached, &jit);
    for cpu in [&mut cached, &mut jit] {
        cpu.write_mem_u32_be(0, r(0x3f, 0, 0, 0));
        cpu.write_mem_u32_be(4, i(4, 7, 7, 1));
        cpu.write_mem_u32_be(8, r(0x3f, 0, 0, 0));
        cpu.set_pc(0);
        cpu.run(1);
        assert!(!cpu.is_running());
        cpu.set_irq_input(true, 3);
        cpu.run(1);
        cpu.set_irq_input(false, 3);
        cpu.run(2);
    }
    same(&cached, &jit);
    assert_eq!(jit.read_reg(7), 1);
    assert!(!jit.is_running());
}

#[test]
fn native_blocked_cpu_batches_idle_devices_but_observes_timer_irq_deadline() {
    use cpt32::devices::pec::timing::connect_pec_timer_rtc;
    fn machine(kind: CpuKind) -> AnyCpu {
        let mut bus = Bus::new();
        connect_ram(&mut bus);
        connect_pec_timer_rtc(&mut bus);
        let mut cpu = AnyCpu::new(kind, bus);
        // Increment R1 in a native-eligible loop. IRQ4 increments R2 and IRETs.
        let words = [i(4, 1, 1, 1), i(9, 0, 0, -8)];
        let image: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
        cpu.load_program(0, &image);
        cpu
    }
    let mut jit = machine(CpuKind::Jit);
    assert_eq!(jit.kind(), CpuKind::Jit);
    jit.write_mem_u32_be(0xffff_0110, i(4, 2, 2, 1));
    jit.write_mem_u32_be(0xffff_0114, 0x8000_0000); // IRET
    jit.write_mem_u8(0xffff_0045, 1 << 4);
    jit.write_mem_u32_be(0x8004_0184, 1);
    jit.write_mem_u32_be(0x8004_0410, 73);
    jit.write_mem_u32_be(0x8004_0400, 12);
    jit.run(500);
    native_ran(&jit);
    assert_eq!(jit.irq_cause(), 4);
    assert!(jit.irq_enabled());
    assert!(jit.read_reg(1) > 0);
    assert_eq!(jit.read_reg(2), 1);
}

#[test]
fn profiling_and_debug_steps_remain_cached_and_observable() {
    let words = [i(0x04, 1, 1, 1), i(0x09, 0, 0, -8)];
    let mut cached = boot(CpuKind::Cached, 0, &words);
    let mut jit = boot(CpuKind::Jit, 0, &words);
    cached.run(300);
    jit.run(300);
    let before = jit.jit_stats().unwrap().native_instructions;
    cached.enable_profiling(true);
    jit.enable_profiling(true);
    cached.run(100);
    jit.run(100);
    same(&cached, &jit);
    assert_eq!(jit.jit_stats().unwrap().native_instructions, before);
    assert_eq!(jit.take_profile().unwrap().ticks, 100);
    for _ in 0..4 {
        assert_eq!(cached.step_once(), jit.step_once());
        same(&cached, &jit);
    }
}

#[test]
fn dma_rewrites_a_later_native_instruction_during_the_block() {
    use cpt32::devices::bmc::connect_bmc;
    use cpt32::devices::dmac::dmac::{DMAC_BASE, connect_dmac};
    fn machine(kind: CpuKind) -> AnyCpu {
        let mut bus = Bus::new();
        connect_ram(&mut bus);
        let bmc = connect_bmc(&mut bus);
        connect_dmac(&mut bus, bmc);
        let mut cpu = AnyCpu::new(kind, bus);
        let words = [i(4, 1, 1, 1), i(4, 2, 2, 1), i(4, 3, 3, 1), i(9, 0, 0, -16)];
        let image: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
        cpu.load_program(0, &image);
        cpu.run(400);
        cpu
    }
    for irq in [false, true] {
        let mut cached = machine(CpuKind::Cached);
        let mut jit = machine(CpuKind::Jit);
        native_ran(&jit);
        let original = jit.read_reg(2);
        for cpu in [&mut cached, &mut jit] {
            cpu.write_mem_u32_be(0x2000, i(4, 2, 2, 9));
            if irq {
                cpu.write_mem_u32_be(0xffff_0104, 0x8000_0000); // IRQ1 IRET
                cpu.write_mem_u8(0xffff_0045, 2); // IRQC source1 enabled
                cpu.write_mem_u32_be(DMAC_BASE + 0x0c, 1); // channel0 IRQ
            }
            let channel = DMAC_BASE + 0x100;
            cpu.write_mem_u32_be(channel, 0x2000);
            cpu.write_mem_u32_be(channel + 4, 4);
            cpu.write_mem_u32_be(channel + 8, 4);
            cpu.write_mem_u32_be(DMAC_BASE + 8, 1);
            cpu.write_mem_u32_be(channel + 12, 2 | (1 << 8));
            cpu.run(4);
        }
        same(&cached, &jit);
        assert_eq!(jit.read_mem_u32_be(4), i(4, 2, 2, 9));
        assert_eq!(jit.read_reg(2), original + 9);
        assert!(jit.jit_stats().unwrap().invalidations > 0);
        if irq {
            assert_eq!(jit.irq_cause(), 1);
        }
    }
}

#[test]
fn fallback_panics_return_to_rust_without_unwinding_through_native_code() {
    let words = [i(4, 1, 1, 1), i(9, 0, 0, -8)];
    let mut jit = boot(CpuKind::Jit, 0, &words);
    jit.run(300);
    native_ran(&jit);
    jit.write_mem_u32_be(0, r(0x23, 0, 0, 0)); // illegal opcode
    let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| jit.run(1)));
    assert!(error.is_err());
    // The module can still be safely reset and execute after the Rust panic.
    jit.write_mem_u32_be(0, words[0]);
    jit.reset(0);
    jit.run(300);
    assert_eq!(jit.read_reg(1), 150);
}

#[test]
fn non_previewable_instruction_devices_do_not_receive_speculative_reads() {
    use cpt32::bus::Device;
    use std::cell::Cell;
    use std::rc::Rc;
    struct Instructions {
        reads: Rc<Cell<usize>>,
    }
    impl Device for Instructions {
        fn read(&mut self, address: u32) -> u8 {
            self.reads.set(self.reads.get() + 1);
            i(9, 0, 0, -4).to_be_bytes()[address as usize]
        }
        fn write(&mut self, _: u32, _: u8) {}
        fn size(&self) -> u32 {
            4
        }
    }
    let reads = Rc::new(Cell::new(0));
    let mut bus = Bus::new();
    bus.add_device(
        0x2000_0000,
        "non-cacheable instructions",
        Box::new(Instructions {
            reads: reads.clone(),
        }),
    );
    let mut jit = AnyCpu::new(CpuKind::Jit, bus);
    jit.set_pc(0x2000_0000);
    jit.run(100);
    assert_eq!(reads.get(), 400);
    assert_eq!(jit.jit_stats().unwrap().native_instructions, 0);
    assert_eq!(jit.pc(), 0x2000_0000);
}

#[test]
fn long_cold_paths_do_not_evict_hot_native_blocks() {
    let words = [i(4, 1, 1, 1), i(9, 0, 0, -8)];
    let mut jit = boot(CpuKind::Jit, 0, &words);
    jit.run(300);
    native_ran(&jit);
    let before = jit.jit_stats().unwrap();
    // More than the dispatch capacity, all distinct cold NOP addresses in RAM.
    jit.set_pc(0x20000);
    jit.run(1100);
    jit.set_pc(0);
    jit.run(16);
    let after = jit.jit_stats().unwrap();
    assert_eq!(after.cache_resets, before.cache_resets);
    assert_eq!(after.compilations, before.compilations);
    assert_eq!(after.native_instructions, before.native_instructions + 16);
}

#[test]
fn idle_native_batches_preserve_prefix_budgets_mmio_and_late_code_changes() {
    let mut words = vec![i(4, 1, 1, 1); 15];
    words.push(i(9, 0, 0, -64));
    // Exercise an unaligned instruction crossing a page, every possible prefix
    // budget, and code replacement late enough to follow a multi-instruction prefix.
    for base in [0, 4094] {
        let mut cached = boot(CpuKind::Cached, base, &words);
        let mut jit = boot(CpuKind::Jit, base, &words);
        cached.run(4096);
        jit.run(4096);
        same(&cached, &jit);
        let stats = jit.jit_stats().unwrap();
        assert!(
            stats.native_batches > 0 && stats.batched_instructions > 1000,
            "{stats:?}"
        );
        for budget in 1..=65 {
            cached.run(budget);
            jit.run(budget);
            same(&cached, &jit);
            assert_eq!(jit.read_mem_u32_be(0xffff_0280), jit.pc());
            assert_eq!(jit.read_mem_u32_be(0xffff_0204), jit.read_reg(1));
        }
        let before = jit.jit_stats().unwrap().invalidations;
        for cpu in [&mut cached, &mut jit] {
            cpu.write_mem_u32_be(base + 40, i(4, 1, 1, 9));
            cpu.set_pc(base);
            cpu.run(128);
        }
        same(&cached, &jit);
        assert!(jit.jit_stats().unwrap().invalidations > before);
    }
}

#[test]
fn idle_native_batches_stop_before_every_timer_deadline_and_pending_external_irq() {
    use cpt32::devices::pec::timing::connect_pec_timer_rtc;
    fn machine(kind: CpuKind) -> AnyCpu {
        let mut bus = Bus::new();
        connect_ram(&mut bus);
        connect_pec_timer_rtc(&mut bus);
        let mut cpu = AnyCpu::new(kind, bus);
        let mut words = vec![i(4, 1, 1, 1); 15];
        words.push(i(9, 0, 0, -64));
        cpu.load_program(
            0,
            &words
                .iter()
                .flat_map(|w| w.to_be_bytes())
                .collect::<Vec<_>>(),
        );
        cpu.write_mem_u32_be(0xffff_0110, i(4, 2, 2, 1));
        cpu.write_mem_u32_be(0xffff_0114, 0x8000_0000);
        cpu.write_mem_u32_be(0xffff_010c, 0x8000_0000);
        cpu.run(4096);
        cpu
    }
    for deadline in 1..=65 {
        let mut cached = machine(CpuKind::Cached);
        let mut jit = machine(CpuKind::Jit);
        assert!(jit.jit_stats().unwrap().native_batches > 0);
        for cpu in [&mut cached, &mut jit] {
            cpu.write_mem_u8(0xffff_0045, 1 << 4);
            cpu.write_mem_u32_be(0x8004_0184, 1);
            cpu.write_mem_u32_be(0x8004_0410, deadline);
            cpu.write_mem_u32_be(0x8004_0400, 12);
            cpu.run(deadline as usize);
        }
        same(&cached, &jit);
        assert_eq!(jit.irq_cause(), 4);
        for cpu in [&mut cached, &mut jit] {
            cpu.run(128);
            // Mask the still-latched timer source and sample its low level
            // before creating a fresh external edge (CPU IRQ is edge-triggered).
            cpu.write_mem_u8(0xffff_0045, 0);
            cpu.run(1);
            cpu.set_irq_input(true, 3);
            cpu.run(16);
            cpu.set_irq_input(false, 3);
        }
        same(&cached, &jit);
        assert_eq!(jit.read_reg(2), 1);
        assert_eq!(jit.irq_cause(), 3);
    }
}
