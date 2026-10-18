use cpt32::bus::{Bus, connect_bmc_dmac, connect_devices_with_vdp};
use cpt32::devices::pec::idc::idc::connect_idc;
use cpt32::devices::pec::pec::connect_pec_input;
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::ram::connect_ram;
use cpt32::devices::sgc::sgc::SGC_MMIO_BASE;
use cpt32::devices::sgu::s3w2::S3w2Sound;
use cpt32::devices::vdp::vdp::{VDP_REG_BASE, VDP_VRAM_BASE, connect_vdp};
use std::hint::black_box;
use std::time::{Duration, Instant};

const SAMPLES: usize = 7;
const BUS_TICKS: usize = 400_000;
const FRAME_SAMPLES: usize = 120;
const VDP_TICKS: usize = 1_000_000;
const AUDIO_SAMPLES: usize = 800;
const VPU_FIFO_DATA: u32 = 0x8003_002c;
const VPU_FRAME_SAMPLES: usize = 10;

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

fn report(label: &str, iterations: usize, samples: usize, elapsed: Duration) {
    let seconds = elapsed.as_secs_f64();
    println!(
        "{label:<32} median={:>8.3} ms  {:>10.0} ops/s  ({iterations} ops x {samples} samples)",
        seconds * 1_000.0,
        iterations as f64 / seconds,
    );
}

fn benchmark(label: &str, iterations: usize, mut operation: impl FnMut()) {
    let mut results = Vec::with_capacity(SAMPLES);
    operation(); // Warm instruction/data caches before recording samples.
    for _ in 0..SAMPLES {
        let start = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        results.push(start.elapsed());
    }
    report(label, iterations, SAMPLES, median(results));
}

fn measure_audio_batch(sound: &mut S3w2Sound, left: &mut [i16], right: &mut [i16]) -> Duration {
    let mut checksum = 0i64;
    let mut mix_once = || {
        left.fill(0);
        right.fill(0);
        sound.clock_mixed_into(left, right);
        black_box((&*left, &*right));
        checksum = checksum.wrapping_add(i64::from(black_box(left[0])));
    };
    mix_once(); // Warm up before recording the sample.
    let start = Instant::now();
    for _ in 0..FRAME_SAMPLES {
        mix_once();
    }
    black_box(checksum);
    start.elapsed()
}

fn active_noise_sound(active_channels: usize) -> S3w2Sound {
    let mut sound = S3w2Sound::new();
    for channel in 0..8 {
        let base = 0x800 + channel as u32 * 0x20;
        sound.write_register(base + 0x19, 0);
    }
    for channel in 0..active_channels {
        let base = 0x800 + channel as u32 * 0x20;
        sound.write_register(base, 0x01);
        sound.write_register(base + 1, 0x70);
        sound.write_register(base + 2, 2); // Noise waveform.
        sound.write_register(base + 3, 255);
        sound.write_register(base + 4, 0xff);
        sound.write_register(base + 0x19, 1); // Start channel.
    }
    sound
}

fn connected_idle_bus() -> (
    Bus,
    tokio::sync::mpsc::UnboundedSender<cpt32::devices::pec::pec::HostInputEvent>,
) {
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    connect_devices_with_vdp(&mut bus);
    connect_idc(&mut bus, [None, None, None, None]);
    (bus, input)
}

fn enable_display(bus: &mut Bus) {
    bus.write_u8(VDP_REG_BASE, 1);
}

fn configure_pcg(bus: &mut Bus, graphics_mode: bool, overlay: bool) {
    bus.write_u8(VDP_REG_BASE + 1, u8::from(!graphics_mode));
    if graphics_mode {
        enable_display(bus);
        bus.write_u8(VDP_REG_BASE + 0xf00b, u8::from(overlay));
        bus.write_u8(VDP_REG_BASE + 0xf005, 20);
        bus.write_u8(VDP_REG_BASE + 0xf006, 15);
        bus.write_u8(VDP_REG_BASE + 0xf007, 1);
    } else {
        bus.write_u8(VDP_REG_BASE + 0xf000, 0);
        bus.write_u8(VDP_REG_BASE + 0xf003, 0);
        bus.write_u8(VDP_REG_BASE + 0xf005, 20);
        bus.write_u8(VDP_REG_BASE + 0xf006, 15);
        bus.write_u8(VDP_REG_BASE + 0xf007, 1);
    }

    for cell in 0..40 * 30 {
        bus.write_u8(VDP_VRAM_BASE + cell, b'A' + (cell % 26) as u8);
        bus.write_u8(VDP_VRAM_BASE + 0x1000 + cell, 0x0f);
        bus.write_u8(VDP_VRAM_BASE + 0x2000 + cell, 0);
    }
}

fn configure_sgc_sprites(bus: &mut Bus, count: usize) {
    const SAT_BASE: u32 = 0x1000;
    const PATTERN_BASE: u32 = 0x2000;
    bus.write_u8(VDP_REG_BASE, 1);
    bus.write_u32_be(SGC_MMIO_BASE + 0x10, SAT_BASE);
    bus.write_u32_be(SGC_MMIO_BASE + 0x14, count as u32);
    bus.write_u8(SGC_MMIO_BASE + 7, 1);
    for byte in 0..(32 * 32 / 2) {
        bus.write_u8(VDP_VRAM_BASE + PATTERN_BASE + byte, 0x11);
    }
    for index in 0..count {
        let x = (index % 8) * 40;
        let y = (index / 8) * 48;
        let mut sprite = [0u8; 16];
        sprite[0..2].copy_from_slice(&(x as i16).to_be_bytes());
        sprite[2..4].copy_from_slice(&(y as i16).to_be_bytes());
        sprite[4] = 32;
        sprite[5] = 32;
        sprite[6..10].copy_from_slice(&PATTERN_BASE.to_be_bytes());
        sprite[10] = 0;
        sprite[11] = index as u8;
        sprite[12] = 1;
        sprite[13] = 1;
        sprite[14] = 1;
        for (offset, byte) in sprite.into_iter().enumerate() {
            bus.write_u8(
                VDP_VRAM_BASE + SAT_BASE + (index * 16 + offset) as u32,
                byte,
            );
        }
    }
}

fn submit_flat_triangle(bus: &mut Bus, depth: i32) {
    bus.write_u32_be(VPU_FIFO_DATA, 0x1200_000b);
    for word in [
        0,
        0,
        depth as u32,
        319,
        0,
        depth as u32,
        160,
        239,
        depth as u32,
        0x00ff_8040,
        255,
    ] {
        bus.write_u32_be(VPU_FIFO_DATA, word);
    }
}

fn main() {
    println!("Emulator component benchmarks (release build, median of {SAMPLES} samples)");
    println!("Each workload gets one warm-up operation; timing excludes CPU profile hooks.");

    let mut empty_bus = Bus::new();
    benchmark("Bus tick, no mapped devices", BUS_TICKS, || {
        empty_bus.tick_devices();
    });

    let mut ram_bus = Bus::new();
    connect_ram(&mut ram_bus);
    benchmark("Bus tick, RAM only", BUS_TICKS, || {
        ram_bus.tick_devices();
    });

    let mut dma_bus = Bus::new();
    connect_ram(&mut dma_bus);
    connect_bmc_dmac(&mut dma_bus);
    benchmark("Bus tick, RAM + BMC/DMAC", BUS_TICKS, || {
        dma_bus.tick_devices();
    });

    let mut core_devices_bus = Bus::new();
    connect_devices_with_vdp(&mut core_devices_bus);
    benchmark("Bus tick, core devices idle", BUS_TICKS, || {
        core_devices_bus.tick_devices();
    });

    let mut pec_bus = Bus::new();
    connect_devices_with_vdp(&mut pec_bus);
    let _pec_input = connect_pec_input(&mut pec_bus);
    benchmark("Bus tick, core + PeC input", BUS_TICKS, || {
        pec_bus.tick_devices();
    });

    let mut idc_bus = Bus::new();
    connect_devices_with_vdp(&mut idc_bus);
    connect_idc(&mut idc_bus, [None, None, None, None]);
    benchmark("Bus tick, core + empty IDC", BUS_TICKS, || {
        idc_bus.tick_devices();
    });

    let (mut bus, _input) = connected_idle_bus();
    benchmark("Bus tick, standard devices idle", BUS_TICKS, || {
        bus.tick_devices();
    });

    let mut vdp_bus = Bus::new();
    let vdp = connect_vdp(&mut vdp_bus);
    vdp_bus.write_u8(VDP_REG_BASE, 1); // Enable display.
    let mut tick_checksum = 0u64;
    benchmark("VDP tick, SGC disabled", VDP_TICKS, || {
        vdp.borrow_mut().tick();
        tick_checksum = tick_checksum.wrapping_add(black_box(1));
    });
    black_box(tick_checksum);

    let mut compose_checksum = 0u64;
    benchmark("VDP full-frame composition", FRAME_SAMPLES, || {
        let vdp_ref = vdp.borrow();
        let framebuffer = vdp_ref.framebuffer();
        let (width, height) = framebuffer.dimensions();
        let (r, g, b) = framebuffer.get_pixel(width - 1, height - 1);
        compose_checksum =
            compose_checksum.wrapping_add(u64::from(r) + u64::from(g) + u64::from(b));
    });
    black_box(compose_checksum);

    let mut pcg_bus = Bus::new();
    let pcg_vdp = connect_vdp(&mut pcg_bus);
    configure_pcg(&mut pcg_bus, false, false);
    let mut pcg_checksum = 0u64;
    benchmark("PCG full-frame composition", FRAME_SAMPLES, || {
        let vdp_ref = pcg_vdp.borrow();
        let framebuffer = vdp_ref.framebuffer();
        let (r, g, b) = framebuffer.get_pixel(319, 239);
        pcg_checksum = pcg_checksum.wrapping_add(u64::from(r) + u64::from(g) + u64::from(b));
    });
    black_box(pcg_checksum);

    let mut overlay_bus = Bus::new();
    let overlay_vdp = connect_vdp(&mut overlay_bus);
    configure_pcg(&mut overlay_bus, true, true);
    let mut overlay_checksum = 0u64;
    benchmark("Graphics + PCG overlay composition", FRAME_SAMPLES, || {
        let vdp_ref = overlay_vdp.borrow();
        let framebuffer = vdp_ref.framebuffer();
        let (r, g, b) = framebuffer.get_pixel(319, 239);
        overlay_checksum =
            overlay_checksum.wrapping_add(u64::from(r) + u64::from(g) + u64::from(b));
    });
    black_box(overlay_checksum);

    // Exercise the expensive full-plane fill path once per VDP tick. FIFO
    // writes are setup for the SGC workload and included in its per-frame cost.
    let mut sgc_bus = Bus::new();
    let sgc_vdp = connect_vdp(&mut sgc_bus);
    sgc_bus.write_u8(VDP_REG_BASE, 1);
    sgc_bus.write_u8(SGC_MMIO_BASE + 7, 1);
    sgc_bus.write_u32_be(SGC_MMIO_BASE + 0x20, 0x0100_003f);
    benchmark("SGC screen fill + VDP tick", FRAME_SAMPLES, || {
        sgc_bus.write_u32_be(SGC_MMIO_BASE + 0x20, 0x1400_0000);
        sgc_vdp.borrow_mut().tick();
    });

    for sprite_count in [8usize, 32] {
        let mut sprite_bus = Bus::new();
        let sprite_vdp = connect_vdp(&mut sprite_bus);
        configure_sgc_sprites(&mut sprite_bus, sprite_count);
        let mut checksum = 0u64;
        benchmark(
            &format!("SGC render, {sprite_count} sprites"),
            FRAME_SAMPLES,
            || {
                let vdp_ref = sprite_vdp.borrow();
                let framebuffer = vdp_ref.framebuffer();
                let (r, g, b) = framebuffer.get_pixel(319, 239);
                checksum = checksum.wrapping_add(u64::from(r) + u64::from(g) + u64::from(b));
            },
        );
        black_box(checksum);
    }

    for triangle_count in [1usize, 8] {
        let mut vpu_bus = Bus::new();
        connect_vdp(&mut vpu_bus);
        let mut depth = 100_000;
        let mut checksum = 0u64;
        benchmark(
            &format!("VPU flat triangles, {triangle_count} per frame"),
            VPU_FRAME_SAMPLES,
            || {
                for _ in 0..triangle_count {
                    submit_flat_triangle(&mut vpu_bus, depth);
                    depth -= 1;
                    checksum = checksum.wrapping_add(black_box(depth as u64));
                }
            },
        );
        black_box(checksum);
    }

    let mut left = vec![0i16; AUDIO_SAMPLES];
    let mut right = vec![0i16; AUDIO_SAMPLES];
    let mut silent_sound = active_noise_sound(0);
    let mut one_channel_sound = active_noise_sound(1);
    let mut eight_channel_sound = active_noise_sound(8);
    let mut audio_results = [
        Vec::with_capacity(SAMPLES),
        Vec::with_capacity(SAMPLES),
        Vec::with_capacity(SAMPLES),
    ];
    for sample in 0..SAMPLES {
        // Rotate order to reduce CPU-frequency and thermal bias between cases.
        for offset in 0..3 {
            let case = (sample + offset) % 3;
            let elapsed = match case {
                0 => measure_audio_batch(&mut silent_sound, &mut left, &mut right),
                1 => measure_audio_batch(&mut one_channel_sound, &mut left, &mut right),
                _ => measure_audio_batch(&mut eight_channel_sound, &mut left, &mut right),
            };
            audio_results[case].push(elapsed);
        }
    }
    report(
        "SGU mix, no active channels",
        FRAME_SAMPLES,
        SAMPLES,
        median(std::mem::take(&mut audio_results[0])),
    );
    report(
        "SGU active noise, 1 channel",
        FRAME_SAMPLES,
        SAMPLES,
        median(std::mem::take(&mut audio_results[1])),
    );
    report(
        "SGU active noise, 8 channels",
        FRAME_SAMPLES,
        SAMPLES,
        median(std::mem::take(&mut audio_results[2])),
    );

    println!("Note: benchmarks exclude GPU upload/presentation and active disk/DMA traffic.");
}
