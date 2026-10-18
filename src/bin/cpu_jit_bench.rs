//! Fixed-cycle Cached/JIT comparison, with cold and warm phases separated.
use cpt32::bus::Bus;
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::ram::connect_ram;
use std::error::Error;
use std::time::{Duration, Instant};

fn r(op: u32, rd: u32, a: u32, b: u32) -> u32 {
    (op << 26) | (rd << 21) | (a << 16) | (b << 11)
}
fn i(op: u32, rd: u32, a: u32, imm: i16) -> u32 {
    (op << 26) | (rd << 21) | (a << 16) | u32::from(imm as u16)
}
fn boot(kind: CpuKind, mixed: bool) -> AnyCpu {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let mut cpu = AnyCpu::new(kind, bus);
    let mut words = vec![
        i(4, 1, 1, 1),
        r(3, 3, 1, 2),
        r(0xe, 4, 3, 2),
        r(5, 5, 4, 1),
        r(0xc, 6, 5, 3),
        r(0xd, 7, 6, 4),
        r(0xf, 8, 7, 2),
        r(0x12, 9, 8, 2),
        i(4, 10, 10, -1),
    ];
    if mixed {
        words.extend([r(0x18, 11, 3, 2), i(2, 11, 0, 4096), i(1, 12, 0, 4096)]);
    }
    words.push(i(9, 0, 0, -((words.len() as i16 + 1) * 4)));
    let image: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    cpu.load_program(0, &image);
    cpu.write_reg(2, 3).unwrap();
    cpu
}
fn timed(cpu: &mut AnyCpu, cycles: usize) -> Duration {
    let start = Instant::now();
    cpu.run(cycles);
    start.elapsed()
}
fn same(a: &mut AnyCpu, b: &mut AnyCpu) {
    for reg in 0..32 {
        assert_eq!(a.read_reg(reg), b.read_reg(reg), "R{reg}");
    }
    assert_eq!(a.pc(), b.pc());
    assert_eq!(a.cycles(), b.cycles());
    assert_eq!(a.instructions_executed(), b.instructions_executed());
    assert_eq!(a.read_mem_u32_be(4096), b.read_mem_u32_be(4096));
}
fn median(values: &mut [Duration]) -> f64 {
    values.sort_unstable();
    values[values.len() / 2].as_secs_f64() * 1000.0
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() > 2 {
        return Err("usage: cpu_jit_bench [cycles_per_phase [samples]]".into());
    }
    let cycles = args
        .first()
        .map_or(Ok(400_000), |arg| arg.parse::<usize>())?;
    let samples = args.get(1).map_or(Ok(7), |arg| arg.parse::<usize>())?;
    if cycles == 0 || samples == 0 {
        return Err("cycles and samples must be positive".into());
    }
    for mixed in [false, true] {
        let mut cached_cold = Vec::new();
        let mut jit_cold = Vec::new();
        let mut cached_warm = Vec::new();
        let mut jit_warm = Vec::new();
        let mut last_stats = None;
        for sample in 0..samples {
            let mut cached = boot(CpuKind::Cached, mixed);
            let mut jit = boot(CpuKind::Jit, mixed);
            for phase in 0..2 {
                let (a, b) = if sample % 2 == 0 {
                    (timed(&mut cached, cycles), timed(&mut jit, cycles))
                } else {
                    let b = timed(&mut jit, cycles);
                    (timed(&mut cached, cycles), b)
                };
                same(&mut cached, &mut jit);
                if phase == 0 {
                    cached_cold.push(a);
                    jit_cold.push(b);
                } else {
                    cached_warm.push(a);
                    jit_warm.push(b);
                }
            }
            let stats = jit.jit_stats().unwrap();
            assert!(
                stats.native_instructions > 0,
                "JIT did not execute native code"
            );
            assert_eq!(stats.compile_failures, 0);
            last_stats = Some(stats);
        }
        let a = median(&mut cached_cold);
        let b = median(&mut jit_cold);
        let c = median(&mut cached_warm);
        let d = median(&mut jit_warm);
        println!(
            "{}: {cycles} cycles/phase, {samples} samples; cold includes compilation",
            if mixed {
                "ALU+MUL+memory"
            } else {
                "ALU+branch"
            }
        );
        println!(
            "  cold Cached={a:.3} ms JIT={b:.3} ms speedup={:.3}x",
            a / b
        );
        println!(
            "  warm Cached={c:.3} ms JIT={d:.3} ms speedup={:.3}x",
            c / d
        );
        println!("  last sample JIT stats: {:?}", last_stats.unwrap());
    }
    Ok(())
}
