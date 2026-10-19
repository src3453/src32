use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::pec::idc::idc::connect_idc;
use cpt32::devices::pec::pec::connect_pec_input;
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::ram::connect_ram;
use std::error::Error;
use std::time::{Duration, Instant};

const DEFAULT_ITERATIONS: usize = 10_000;
const DEFAULT_SAMPLES: usize = 5;
const DATA_ADDRESS: u32 = 0x00F0_0000;

const RUN_CHUNK: usize = 64;
struct Sample {
    elapsed: Duration,
    running_cycles: u128,
    instructions: u128,
    stalls: Option<u128>,
    flushes: Option<u128>,
    cache_hits: Option<u128>,
    cache_misses: Option<u128>,
    registers: [u32; 32],
    pc: u32,
    memory: u32,
}

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

fn workload(iterations: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let count = u32::try_from(iterations).map_err(|_| "iteration count exceeds 32 bits")?;
    let mut image = Vec::with_capacity(10 * 4);
    image.extend_from_slice(&encode_i(0x3D, 9, 0, (count >> 16) as u16 as i16)); // LDIH R9, count high
    image.extend_from_slice(&encode_i(0x3C, 9, 0, count as u16 as i16)); // LDIL R9, count low
    image.extend_from_slice(&encode_i(0x04, 2, 0, 3)); // R2 = 3
    image.extend_from_slice(&encode_i(0x3D, 6, 0, 0x00F0)); // LDIH R6, data address high half
    image.extend_from_slice(&encode_i(0x3C, 6, 0, 0)); // LDIL R6, data address low half
    let loop_start = image.len() as i16;
    image.extend_from_slice(&encode_i(0x04, 1, 1, 1)); // increment R1
    image.extend_from_slice(&encode_r(0x03, 3, 1, 2)); // dependent ALU operation
    image.extend_from_slice(&encode_r(0x18, 4, 3, 2)); // multi-cycle MUL
    image.extend_from_slice(&encode_i(0x04, 5, 4, 1)); // consume MUL result
    image.extend_from_slice(&encode_i(0x02, 5, 6, 0)); // store forwarded value
    image.extend_from_slice(&encode_i(0x01, 7, 6, 0)); // load stored value
    image.extend_from_slice(&encode_r(0x03, 8, 7, 5)); // load-use dependency
    image.extend_from_slice(&encode_i(0x04, 9, 9, -1)); // decrement loop counter
    let branch_offset = loop_start - (image.len() as i16 + 4);
    image.extend_from_slice(&encode_i(0x08, 0, 9, branch_offset)); // repeat until R9 == 0
    image.extend_from_slice(&encode_r(0x3F, 0, 0, 0)); // HALT
    Ok(image)
}

fn run(kind: CpuKind, image: &[u8], platform: bool) -> Sample {
    let mut bus = Bus::new();
    let _input = if platform {
        let input = connect_pec_input(&mut bus);
        connect_rng(&mut bus);
        connect_devices_with_vdp(&mut bus);
        connect_idc(&mut bus, [None, None, None, None]);
        Some(input)
    } else {
        connect_ram(&mut bus);
        None
    };
    let mut cpu = AnyCpu::new(kind, bus);
    cpu.load_program(0, image);

    let start = Instant::now();
    while cpu.is_running() {
        cpu.run(RUN_CHUNK);
    }
    let elapsed = start.elapsed();

    let mut registers = [0; 32];
    for (index, register) in registers.iter_mut().enumerate() {
        *register = cpu.read_reg(index);
    }
    let pc = cpu.pc();
    let memory = cpu.read_mem_u32_be(DATA_ADDRESS);
    let (stalls, flushes) = cpu
        .pipeline_stats()
        .map_or((None, None), |(stalls, flushes)| {
            (Some(stalls), Some(flushes))
        });
    let (cache_hits, cache_misses) = cpu
        .instruction_cache_stats()
        .map_or((None, None), |(hits, misses)| (Some(hits), Some(misses)));

    Sample {
        elapsed,
        running_cycles: cpu.running_cycles(),
        instructions: cpu.instructions_executed(),
        stalls,
        flushes,
        cache_hits,
        cache_misses,
        registers,
        pc,
        memory,
    }
}

fn assert_same_architecture(scalar: &Sample, pipeline: &Sample) -> Result<(), Box<dyn Error>> {
    if scalar.registers != pipeline.registers
        || scalar.pc != pipeline.pc
        || scalar.memory != pipeline.memory
    {
        return Err("scalar and pipeline architectural state diverged".into());
    }
    if scalar.instructions != pipeline.instructions {
        return Err("scalar and pipeline retired different instruction counts".into());
    }
    Ok(())
}

fn assert_expected_workload(
    iterations: usize,
    image: &[u8],
    sample: &Sample,
) -> Result<(), Box<dyn Error>> {
    let count = iterations as u32;
    let expected_product = count.wrapping_add(3).wrapping_mul(3);
    let expected_stored = expected_product.wrapping_add(1);
    let expected_instructions = (iterations as u128) * 9 + 6;
    if sample.registers[1] != count
        || sample.registers[2] != 3
        || sample.registers[4] != expected_product
        || sample.registers[5] != expected_stored
        || sample.registers[6] != DATA_ADDRESS
        || sample.registers[7] != expected_stored
        || sample.registers[8] != expected_stored.wrapping_mul(2)
        || sample.memory != expected_stored
        || sample.pc != image.len() as u32
        || sample.instructions != expected_instructions
    {
        return Err("benchmark workload produced unexpected architectural state".into());
    }
    if let (Some(stalls), Some(flushes)) = (sample.stalls, sample.flushes) {
        if stalls != iterations as u128 * 3 || flushes != iterations.saturating_sub(1) as u128 {
            return Err("pipeline workload did not observe the expected hazards".into());
        }
    }
    if let Some(misses) = sample.cache_misses {
        if misses == 0 {
            return Err("cached workload did not exercise instruction-cache misses".into());
        }
    }
    Ok(())
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

fn report(label: &str, elapsed: Duration, sample: &Sample) {
    let seconds = elapsed.as_secs_f64();
    let instructions_per_second = sample.instructions as f64 / seconds;
    let cycles_per_instruction = sample.running_cycles as f64 / sample.instructions as f64;
    let mhz = sample.running_cycles as f64 / seconds / 1_000_000.0;
    println!(
        "{label:<8} median={:.3} ms  {:.0} instr/s  cycles={}  CPI={:.3}",
        elapsed.as_secs_f64() * 1_000.0,
        instructions_per_second,
        sample.running_cycles,
        cycles_per_instruction,
    );
    println!(
        "         effective={mhz:.3} MHz  48 MHz target={:.1}%",
        mhz / 48.0 * 100.0
    );
    if let (Some(stalls), Some(flushes)) = (sample.stalls, sample.flushes) {
        println!("         pipeline stalls={stalls}  flushes={flushes}");
    }
    if let (Some(hits), Some(misses)) = (sample.cache_hits, sample.cache_misses) {
        println!("         instruction cache hits={hits}  misses={misses}");
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let platform = args.iter().any(|arg| arg == "--platform");
    args.retain(|arg| arg != "--platform");
    if args.len() > 2 {
        return Err("usage: cpu_bench [iterations [samples]] [--platform]".into());
    }
    let iterations = args
        .first()
        .map_or(Ok(DEFAULT_ITERATIONS), |arg| arg.parse::<usize>())?;
    let samples = args
        .get(1)
        .map_or(Ok(DEFAULT_SAMPLES), |arg| arg.parse::<usize>())?;
    if iterations == 0 || samples == 0 {
        return Err("iterations and samples must be greater than zero".into());
    }

    let image = workload(iterations)?;
    let run = |kind, image: &[u8]| run(kind, image, platform);
    let mut scalar_results = Vec::with_capacity(samples);
    let mut pipeline_results = Vec::with_capacity(samples);
    let mut cached_results = Vec::with_capacity(samples);
    for index in 0..samples {
        // Rotate start order to reduce systematic warm-up and thermal bias.
        let (scalar, pipeline, cached) = match index % 3 {
            0 => (
                run(CpuKind::Scalar, &image),
                run(CpuKind::Pipelined, &image),
                run(CpuKind::Cached, &image),
            ),
            1 => {
                let pipeline = run(CpuKind::Pipelined, &image);
                let cached = run(CpuKind::Cached, &image);
                let scalar = run(CpuKind::Scalar, &image);
                (scalar, pipeline, cached)
            }
            _ => {
                let cached = run(CpuKind::Cached, &image);
                let scalar = run(CpuKind::Scalar, &image);
                let pipeline = run(CpuKind::Pipelined, &image);
                (scalar, pipeline, cached)
            }
        };
        assert_expected_workload(iterations, &image, &scalar)?;
        assert_expected_workload(iterations, &image, &pipeline)?;
        assert_expected_workload(iterations, &image, &cached)?;
        assert_same_architecture(&scalar, &pipeline)?;
        assert_same_architecture(&scalar, &cached)?;
        scalar_results.push(scalar);
        pipeline_results.push(pipeline);
        cached_results.push(cached);
    }

    let scalar = &scalar_results[0];
    let pipeline = &pipeline_results[0];
    let cached = &cached_results[0];
    let scalar_median = median(scalar_results.iter().map(|sample| sample.elapsed).collect());
    let pipeline_median = median(
        pipeline_results
            .iter()
            .map(|sample| sample.elapsed)
            .collect(),
    );
    let cached_median = median(cached_results.iter().map(|sample| sample.elapsed).collect());
    println!(
        "CPU comparison: {iterations} loop iterations, {samples} samples, {} guest instructions/sample",
        scalar.instructions,
    );
    println!(
        "Bus configuration: {}",
        if platform {
            "idle platform devices"
        } else {
            "RAM only"
        }
    );
    report("Scalar", scalar_median, scalar);
    report("Pipeline", pipeline_median, pipeline);
    report("Cached", cached_median, cached);
    println!(
        "Cached speedup vs Scalar: {:.3}x  vs Pipeline: {:.3}x",
        scalar_median.as_secs_f64() / cached_median.as_secs_f64(),
        pipeline_median.as_secs_f64() / cached_median.as_secs_f64(),
    );
    Ok(())
}
