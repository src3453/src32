//! Host cost of guest MMIO traffic to timing-insensitive devices: a loop of
//! 32-bit VRAM stores, a VDP register byte write, an RNG read and ALU work,
//! on the full idle platform. Prints host time and a result checksum per CPU
//! kind; run with `--features jit` to include the JIT.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::pec::idc::idc::connect_idc;
use cpt32::devices::pec::pec::connect_pec_input;
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::vdp::vdp::{VDP_REG_BASE, VDP_VRAM_BASE};
use std::error::Error;
use std::time::{Duration, Instant};

const RNG: u32 = 0x8004_0010;
const RUN_CHUNK: usize = 4096;

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

fn program(iterations: u32) -> Vec<u8> {
    let mut words = Vec::new();
    load_constant(&mut words, 9, iterations);
    load_constant(&mut words, 10, VDP_VRAM_BASE);
    load_constant(&mut words, 11, VDP_REG_BASE);
    load_constant(&mut words, 12, RNG);
    load_constant(&mut words, 13, 0xFFFC); // VRAM window mask
    let head = words.len() as i16;
    words.push(r(0x0c, 5, 1, 13)); // R5 = R1 & mask
    words.push(r(0x03, 5, 5, 10)); // VRAM address
    words.push(i(0x02, 1, 5, 0)); // ST R1 -> VRAM
    words.push(i(0x02, 4, 5, 4)); // ST R4 -> VRAM
    words.push(i(0x15, 1, 11, 3)); // STB border color register
    words.push(i(0x01, 3, 12, 0)); // LD RNG word (timing-insensitive MMIO)
    words.push(r(0x0e, 4, 4, 1)); // R4 ^= R1
    words.push(r(0x03, 4, 4, 1));
    words.push(i(0x04, 1, 1, 4)); // R1 += 4
    words.push(i(0x04, 9, 9, -1));
    words.push(i(0x08, 0, 9, (head - words.len() as i16 - 1) * 4)); // BNE R9, R0
    words.push(r(0x3f, 0, 0, 0));
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

fn run(kind: CpuKind, image: &[u8]) -> (Duration, u128, u64) {
    let mut bus = Bus::new();
    let _input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    connect_devices_with_vdp(&mut bus);
    connect_idc(&mut bus, std::array::from_fn(|_| None));
    let mut cpu = AnyCpu::new(kind, bus);
    cpu.load_program(0, image);
    let start = Instant::now();
    while cpu.is_running() {
        cpu.run(RUN_CHUNK);
    }
    let elapsed = start.elapsed();
    // R3 holds RNG data (host entropy), so it is excluded.
    let checksum = (0..0x4000).fold(u64::from(cpu.read_reg(4)), |sum, word| {
        sum.rotate_left(7) ^ u64::from(cpu.read_mem_u32_be(VDP_VRAM_BASE + word * 4))
    });
    (elapsed, cpu.instructions_executed(), checksum)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let iterations = args.first().map_or(Ok(100_000), |arg| arg.parse::<u32>())?;
    let samples = args.get(1).map_or(Ok(5), |arg| arg.parse::<usize>())?;
    let image = program(iterations);
    let mut kinds = vec![
        CpuKind::Pipelined,
        CpuKind::Scalar,
        CpuKind::Cached,
        CpuKind::CachedRelaxed,
    ];
    if cfg!(feature = "jit") {
        kinds.push(CpuKind::Jit);
    }
    println!("MMIO loop: {iterations} iterations, {samples} samples (median)");
    for kind in kinds {
        let mut runs: Vec<_> = (0..samples).map(|_| run(kind, &image)).collect();
        runs.sort_by_key(|run| run.0);
        let (elapsed, instructions, checksum) = runs[runs.len() / 2];
        assert!(
            runs.iter()
                .all(|run| run.1 == instructions && run.2 == checksum)
        );
        println!(
            "  {:<15} {:>9.3} ms  instructions={instructions}  checksum={checksum:016x}",
            format!("{kind:?}"),
            elapsed.as_secs_f64() * 1e3
        );
    }
    Ok(())
}
