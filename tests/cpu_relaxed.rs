use cpt32::bus::{Bus, connect_bmc_dmac};
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::dmac::dmac::DMAC_BASE;
use cpt32::devices::pec::timing::connect_pec_timer_rtc;
use cpt32::devices::ram::connect_ram;

fn i(op: u32, rd: u32, a: u32, imm: i16) -> u32 {
    (op << 26) | (rd << 21) | (a << 16) | u32::from(imm as u16)
}
fn machine(kind: CpuKind, words: &[u32]) -> AnyCpu {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    connect_bmc_dmac(&mut bus);
    connect_pec_timer_rtc(&mut bus);
    let mut cpu = AnyCpu::new(kind, bus);
    cpu.load_program(
        0,
        &words
            .iter()
            .flat_map(|w| w.to_be_bytes())
            .collect::<Vec<_>>(),
    );
    cpu
}
fn same(a: &AnyCpu, b: &AnyCpu) {
    for reg in 0..32 {
        assert_eq!(a.read_reg(reg), b.read_reg(reg), "R{reg}");
    }
    assert_eq!(a.pc(), b.pc());
    assert_eq!(a.cycles(), b.cycles());
    assert_eq!(a.instructions_executed(), b.instructions_executed());
    assert_eq!(a.is_running(), b.is_running());
    assert_eq!(a.epc(), b.epc());
    assert_eq!(a.irq_enabled(), b.irq_enabled());
}

#[test]
fn relaxed_preserves_variable_budgets_multicycle_and_mmio_time() {
    let words = [
        i(4, 1, 1, 1),
        0x6000_0000 | (2 << 21) | (1 << 16) | (1 << 11),
        i(1, 3, 4, 0),
        i(9, 0, 0, -16),
    ];
    let mut strict = machine(CpuKind::Cached, &words);
    let mut relaxed = machine(CpuKind::CachedRelaxed, &words);
    assert_eq!(relaxed.kind(), CpuKind::CachedRelaxed);
    for cpu in [&mut strict, &mut relaxed] {
        cpu.write_reg(4, 0x8004_0408).unwrap();
        cpu.write_mem_u32_be(0x8004_0184, 1);
        cpu.write_mem_u32_be(0x8004_0400, 6); // free-running counter
    }
    for budget in [0, 1, 3, 17, 31, 32, 33, 127, 4096] {
        strict.run(budget);
        relaxed.run(budget);
        same(&strict, &relaxed);
        assert_eq!(
            relaxed.read_mem_u32_be(0x8004_0408),
            relaxed.cycles() as u32
        );
    }
}

#[test]
fn relaxed_timer_irq_wakes_halt_and_iret_returns() {
    for halt in [false, true] {
        let words = if halt {
            vec![0xfc00_0000, i(4, 1, 1, 1), i(9, 0, 0, -8)]
        } else {
            vec![i(4, 1, 1, 1), i(9, 0, 0, -8)]
        };
        let mut strict = machine(CpuKind::Cached, &words);
        let mut relaxed = machine(CpuKind::CachedRelaxed, &words);
        for cpu in [&mut strict, &mut relaxed] {
            cpu.write_mem_u32_be(0xffff_0110, 0x8000_0000); // IRQ4 IRET
            cpu.write_mem_u8(0xffff_0045, 1 << 4);
            cpu.write_mem_u32_be(0x8004_0184, 1);
            cpu.write_mem_u32_be(0x8004_0410, 47);
            cpu.write_mem_u32_be(0x8004_0400, 12);
        }
        // Warm cache first, then reset timer so the deadline falls inside a batch.
        strict.run(100);
        relaxed.run(100);
        for cpu in [&mut strict, &mut relaxed] {
            cpu.write_mem_u32_be(0x8004_018c, 1);
            cpu.write_mem_u8(0xffff_0041, 1 << 4);
            cpu.write_mem_u32_be(0x8004_0400, 28);
        }
        for budget in [40, 6, 1, 1, 49] {
            strict.run(budget);
            relaxed.run(budget);
            same(&strict, &relaxed);
        }
        assert_eq!(relaxed.irq_cause(), 4);
        assert!(relaxed.irq_enabled());
        assert!(relaxed.read_reg(1) > 0);
    }
}

#[test]
fn relaxed_active_dma_retains_per_cycle_code_invalidation_and_irq() {
    let words = [i(4, 1, 1, 1), i(4, 2, 2, 1), i(4, 3, 3, 1), i(9, 0, 0, -16)];
    let mut strict = machine(CpuKind::Cached, &words);
    let mut relaxed = machine(CpuKind::CachedRelaxed, &words);
    for cpu in [&mut strict, &mut relaxed] {
        cpu.run(400);
        cpu.write_mem_u32_be(0x2000, i(4, 2, 2, 9));
        cpu.write_mem_u32_be(0xffff_0104, 0x8000_0000);
        cpu.write_mem_u8(0xffff_0045, 2);
        cpu.write_mem_u32_be(DMAC_BASE + 0x0c, 1);
        let channel = DMAC_BASE + 0x100;
        cpu.write_mem_u32_be(channel, 0x2000);
        cpu.write_mem_u32_be(channel + 4, 4);
        cpu.write_mem_u32_be(channel + 8, 4);
        cpu.write_mem_u32_be(DMAC_BASE + 8, 1);
        cpu.write_mem_u32_be(channel + 12, 2 | (1 << 8));
    }
    for _ in 0..12 {
        strict.run(1);
        relaxed.run(1);
        same(&strict, &relaxed);
    }
    assert_eq!(relaxed.read_mem_u32_be(4), i(4, 2, 2, 9));
    assert_eq!(relaxed.irq_cause(), 1);
}

#[test]
fn relaxed_halt_boundary_irq_syncs_the_handler_pc_before_next_cycle() {
    let mut cpu = machine(CpuKind::CachedRelaxed, &[0xfc00_0000, i(4, 1, 1, 1)]);
    cpu.write_mem_u32_be(0xffff_0110, 0x8000_0000);
    cpu.write_mem_u8(0xffff_0045, 1 << 4);
    cpu.write_mem_u32_be(0x8004_0184, 1);
    cpu.write_mem_u32_be(0x8004_0410, 1);
    cpu.write_mem_u32_be(0x8004_0400, 12);
    cpu.run(1);
    assert_eq!(cpu.pc(), 0xffff_0110);
    assert!(!cpu.irq_enabled());
    cpu.run(1);
    assert_eq!(cpu.pc(), 4);
    assert!(cpu.irq_enabled());
    assert_eq!(cpu.read_reg(1), 0);
    cpu.run(1);
    assert_eq!(cpu.read_reg(1), 1);
}

#[test]
fn relaxed_profile_and_debug_step_use_strict_device_progress() {
    let words = [i(4, 1, 1, 1), i(9, 0, 0, -8)];
    let mut strict = machine(CpuKind::Cached, &words);
    let mut relaxed = machine(CpuKind::CachedRelaxed, &words);
    relaxed.enable_profiling(true);
    strict.run(100);
    relaxed.run(100);
    same(&strict, &relaxed);
    assert_eq!(relaxed.take_profile().unwrap().ticks, 100);
    relaxed.enable_profiling(false);
    for _ in 0..3 {
        strict.step_once();
        relaxed.step_once();
        same(&strict, &relaxed);
    }
    strict.reset(0);
    relaxed.reset(0);
    strict.run(33);
    relaxed.run(33);
    same(&strict, &relaxed);
}
