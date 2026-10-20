//! Host cost of IDC/DMA-bound guest work: a polled DMA READ of whole sectors,
//! using the same register sequence as `sol/cpt32/idc.sol` `idc_read`.
//! Prints host time, emulated cycles and a buffer checksum per CPU kind.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::pec::idc::idc::{Idc, connect_idc};
use cpt32::devices::pec::pec::connect_pec_input;
use cpt32::devices::pec::rng::connect_rng;
use std::error::Error;
use std::path::Path;
use std::time::{Duration, Instant};

const DMAC: u32 = 0x8005_0000;
const IDC: u32 = 0x8004_1000;
const IDC_DATA: u32 = 0x8004_1024;
const BUFFER: u32 = 0x0010_0000;
const RUN_CHUNK: usize = 800_000;

fn r(op: u32, rd: u32, a: u32, b: u32) -> u32 {
    (op << 26) | (rd << 21) | (a << 16) | (b << 11)
}

fn i(op: u32, rd: u32, a: u32, imm: i16) -> u32 {
    (op << 26) | (rd << 21) | (a << 16) | u32::from(imm as u16)
}

fn load_constant(words: &mut Vec<u32>, rd: u32, value: u32) {
    words.push(i(0x3d, rd, 0, (value >> 16) as i16)); // LDIH
    words.push(i(0x3c, rd, 0, value as i16)); // LDIL
}

fn store(words: &mut Vec<u32>, value: u32, base: u32, offset: i16) {
    load_constant(words, 1, value);
    words.push(i(0x02, 1, base, offset)); // ST R1, [base + offset]
}

/// Polled `idc_read(0, 0, sectors, BUFFER)`, then HALT.
fn guest(sectors: u32) -> Vec<u8> {
    let mut words = Vec::new();
    load_constant(&mut words, 10, DMAC);
    load_constant(&mut words, 11, IDC);
    store(&mut words, 1, 10, 0x04); // DMAC global enable
    store(&mut words, 1, 10, 0x08); // channel 0 enable
    store(&mut words, 6, 10, 0x110); // clear channel 0 DONE/ERROR
    store(&mut words, 6, 11, 0x08); // clear IDC DONE/ERROR
    store(&mut words, 0, 11, 0x14); // drive select 0
    store(&mut words, 0, 11, 0x1c); // LBA 0
    store(&mut words, sectors, 11, 0x20);
    store(&mut words, IDC_DATA, 10, 0x100);
    store(&mut words, BUFFER, 10, 0x104);
    store(&mut words, sectors * 512, 10, 0x108);
    store(&mut words, 1, 11, 0x18); // READ
    store(&mut words, 0x104, 10, 0x10c); // DMA start
    words.push(i(0x04, 2, 0, 1)); // R2 = BUSY mask
    for (base, offset) in [(10, 0x110), (11, 0x08)] {
        let poll = words.len() as i16;
        words.push(i(0x01, 3, base, offset));
        words.push(r(0x0c, 3, 3, 2));
        words.push(i(0x08, 0, 3, (poll - words.len() as i16 - 1) * 4)); // BNE
    }
    words.push(r(0x3f, 0, 0, 0)); // HALT
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

fn write_config(dir: &Path, kind: &str, sectors: u32) -> Result<(), Box<dyn Error>> {
    let image: Vec<u8> = (0..sectors * 512)
        .map(|index| (index.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    std::fs::write(dir.join("disk.img"), image)?;
    std::fs::write(
        dir.join("config.yaml"),
        format!("disk:\n  - slot: 0\n    type: {kind}\n    image: disk.img\n"),
    )?;
    Ok(())
}

struct Sample {
    elapsed: Duration,
    cycles: u128,
    skipped: u128,
    checksum: u64,
}

fn run(kind: CpuKind, config: &Path, program: &[u8], sectors: u32) -> Sample {
    let mut bus = Bus::new();
    let _input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    connect_devices_with_vdp(&mut bus);
    connect_idc(&mut bus, Idc::load_drives(config).expect("drive config"));
    let mut cpu = AnyCpu::new(kind, bus);
    cpu.load_program(0, program);
    let start = Instant::now();
    while cpu.is_running() {
        cpu.run(RUN_CHUNK);
    }
    let elapsed = start.elapsed();
    let checksum = (0..sectors * 128).fold(0u64, |sum, word| {
        sum.rotate_left(5) ^ u64::from(cpu.read_mem_u32_be(BUFFER + word * 4))
    });
    assert_eq!(cpu.read_reg(3), 0, "transfer did not finish cleanly");
    Sample {
        elapsed,
        cycles: cpu.cycles(),
        skipped: cpu.spin_skipped_cycles(),
        checksum,
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let samples = args.first().map_or(Ok(3), |arg| arg.parse::<usize>())?;
    let dir = std::env::temp_dir().join(format!("cpt32-idc-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let mut kinds = vec![
        CpuKind::Pipelined,
        CpuKind::Scalar,
        CpuKind::Cached,
        CpuKind::CachedRelaxed,
    ];
    if cfg!(feature = "jit") {
        kinds.push(CpuKind::Jit);
    }
    for (drive, sectors) in [("fdd", 32), ("hdd", 2048)] {
        write_config(&dir, drive, sectors)?;
        let config = dir.join("config.yaml");
        let program = guest(sectors);
        println!("{drive}: polled DMA READ of {sectors} sectors, {samples} samples (median)");
        for &kind in &kinds {
            let mut runs: Vec<Sample> = (0..samples)
                .map(|_| run(kind, &config, &program, sectors))
                .collect();
            runs.sort_by_key(|sample| sample.elapsed);
            let median = &runs[runs.len() / 2];
            assert!(
                runs.iter()
                    .all(|sample| sample.checksum == median.checksum
                        && sample.cycles == median.cycles)
            );
            let seconds = median.elapsed.as_secs_f64();
            println!(
                "  {:<15} {:>9.3} ms  cycles={:>10}  spin-skipped={:>10}  effective={:>7.1} MHz  checksum={:016x}",
                format!("{kind:?}"),
                seconds * 1e3,
                median.cycles,
                median.skipped,
                median.cycles as f64 / seconds / 1e6,
                median.checksum
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
    Ok(())
}
