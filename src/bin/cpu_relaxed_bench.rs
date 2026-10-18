//! Unprofiled, fixed-budget comparison with attached idle platform devices.
use cpt32::bus::{Bus, connect_bmc_dmac};
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::pec::idc::idc::connect_idc;
use cpt32::devices::pec::pec::connect_pec_input;
use cpt32::devices::ram::connect_ram;
use std::time::{Duration, Instant};

fn i(op: u32, rd: u32, a: u32, imm: i16) -> u32 {
    (op << 26) | (rd << 21) | (a << 16) | u32::from(imm as u16)
}
fn boot(kind: CpuKind, mixed: bool) -> AnyCpu {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    connect_bmc_dmac(&mut bus);
    let _input = connect_pec_input(&mut bus);
    connect_idc(&mut bus, std::array::from_fn(|_| None));
    let mut cpu = AnyCpu::new(kind, bus);
    let mut words = vec![i(4, 1, 1, 1), i(4, 2, 2, 3), i(4, 3, 3, -1)];
    if mixed {
        words.extend([i(2, 1, 0, 4096), i(1, 4, 0, 4096)]);
    }
    words.push(i(9, 0, 0, -((words.len() as i16 + 1) * 4)));
    cpu.load_program(
        0,
        &words
            .iter()
            .flat_map(|w| w.to_be_bytes())
            .collect::<Vec<_>>(),
    );
    cpu.run(1024); // populate instruction caches outside the measurement
    cpu
}
fn timed(cpu: &mut AnyCpu, cycles: usize) -> Duration {
    let start = Instant::now();
    cpu.run(cycles);
    start.elapsed()
}
fn median(values: &mut [Duration]) -> f64 {
    values.sort_unstable();
    values[values.len() / 2].as_secs_f64() * 1000.0
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() > 2 {
        return Err("usage: cpu_relaxed_bench [cycles [samples]]".into());
    }
    let cycles = args.first().map_or(Ok(400_000), |v| v.parse::<usize>())?;
    let samples = args.get(1).map_or(Ok(7), |v| v.parse::<usize>())?;
    if cycles == 0 || samples == 0 {
        return Err("cycles and samples must be positive".into());
    }
    for mixed in [false, true] {
        let mut strict_times = Vec::new();
        let mut relaxed_times = Vec::new();
        for sample in 0..samples {
            let mut strict = boot(CpuKind::Cached, mixed);
            let mut relaxed = boot(CpuKind::CachedRelaxed, mixed);
            let (a, b) = if sample % 2 == 0 {
                (timed(&mut strict, cycles), timed(&mut relaxed, cycles))
            } else {
                let b = timed(&mut relaxed, cycles);
                (timed(&mut strict, cycles), b)
            };
            for reg in 0..32 {
                assert_eq!(strict.read_reg(reg), relaxed.read_reg(reg));
            }
            assert_eq!(strict.pc(), relaxed.pc());
            assert_eq!(strict.cycles(), relaxed.cycles());
            assert_eq!(
                strict.instructions_executed(),
                relaxed.instructions_executed()
            );
            assert_eq!(strict.read_mem_u32_be(4096), relaxed.read_mem_u32_be(4096));
            strict_times.push(a);
            relaxed_times.push(b);
        }
        let a = median(&mut strict_times);
        let b = median(&mut relaxed_times);
        println!(
            "{}: {cycles} cycles, {samples} samples, idle BMC/DMAC/PeC/timer/IDC; Cached={a:.3} ms relaxed={b:.3} ms speedup={:.3}x",
            if mixed { "ALU+RAM" } else { "ALU+branch" },
            a / b
        );
    }
    Ok(())
}
