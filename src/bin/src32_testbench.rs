//! Headless SRC32 execution harness for compiler pipeline smoke tests.
use std::env;
use std::fs;

use cpt32::bus::Bus;
use cpt32::bus::connect_devices_with_vdp;
use cpt32::cpu::{Cpu, CYCLES_PER_FRAME};

use cpt32::devices::pec::idc::idc::connect_idc_from_config;
use cpt32::devices::pec::pec::connect_pec_input;
use cpt32::devices::pec::rng::connect_rng;

use cpt32::music::guest::{STREAM_BASE, read_music_stream};
use std::path::Path;

fn integer(text: &str) -> u32 {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).expect("invalid hexadecimal integer")
    } else { text.parse().expect("invalid integer") }
}
fn gp0_noise_pixel(cpu: &mut Cpu, x: usize, y: usize) -> u8 {
    let index = cpu.read_mem_u8(0x1000_0000 + (y * 320 + x) as u32);
    cpu.read_mem_u8(0x1001_2C00 + u32::from(index) * 3)
}
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let path = args.first().expect("usage: src32_testbench PROGRAM.bin [expected] [--allow-running] [--music-stream PATH] [--frames N] [--expect-mem-u8 ADDRESS=VALUE]");
    let mut allow_running = false;
    let mut expect_sgc_overlay = false;
    let mut expect_gp_noise_background = false;
    let mut expected = 0;
    let mut music_stream = None;
    let mut frames = None;
    let mut memory_expectations = Vec::new();
    let mut options = args.iter().skip(1);
    while let Some(option) = options.next() {
        match option.as_str() {
            "--allow-running" => allow_running = true,
            "--expect-sgc-overlay" => expect_sgc_overlay = true,
            "--expect-gp-noise-background" => expect_gp_noise_background = true,
            "--music-stream" => music_stream = Some(options.next().expect("--music-stream requires PATH")),
            "--frames" => frames = Some(integer(options.next().expect("--frames requires N")) as usize),
            "--expect-mem-u8" => {
                let (address, value) = options.next().expect("--expect-mem-u8 requires ADDRESS=VALUE")
                    .split_once('=').expect("expected ADDRESS=VALUE");
                memory_expectations.push((integer(address), u8::try_from(integer(value)).expect("value must fit u8")));
            }
            _ if option.starts_with('-') => panic!("unknown option: {option}"),
            _ => expected = integer(option),
        }
    }
    let image = fs::read(path).expect("failed to read program binary");
    let stream = music_stream.map(|path| read_music_stream(Path::new(path), image.len()).expect("invalid music stream"));

    let mut bus = Bus::new();
    let _input_sender = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    connect_idc_from_config(&mut bus).expect("Invalid disk configuration");
    let mut cpu = Cpu::new(bus);
    cpu.load_program(0, &image);
    if let Some(stream) = stream {
        cpu.load_program(STREAM_BASE, &stream);
        // Finish validation and initial uploads before delivering the first VSYNC.
        cpu.run(50_000_000);
    }
    let cycles_per_frame = CYCLES_PER_FRAME as usize;
    let cycle_budget = if expect_gp_noise_background {
        50_000_000
    } else {
        5_000_000
    };
    let frame_count = frames.unwrap_or((cycle_budget + cycles_per_frame - 1) / cycles_per_frame);
    for frame in 0..frame_count {
        vdp.borrow_mut().tick();
        cpu.set_irq_source(0, true);
        cpu.set_irq_source(0, false);
        cpu.run(cycles_per_frame);
        for &(address, expected) in &memory_expectations {
            let actual = cpu.read_mem_u8(address);
            assert_eq!(actual, expected, "frame {frame}: memory 0x{address:08X}");
        }
    }

    let result = cpu.read_reg(1);
    assert!(
        allow_running || !cpu.is_running(),
        "program did not halt (pc=0x{:08X})",
        cpu.pc()
    );
    if allow_running && music_stream.is_some() {
        println!("PASS: {path} music stream; frames={frame_count}; memory checks={}; cycles={}", memory_expectations.len(), cpu.cycles());
    } else if allow_running {
        let vpu_status = cpu.read_mem_u32_be(0x8003_0004);
        assert_eq!(
            vpu_status & 0x38,
            0,
            "VPU reported an error: 0x{vpu_status:08X}"
        );
        let sgc_status = cpu.read_mem_u32_be(0x8001_0008);
        let vdp_ref = vdp.borrow();
        let framebuffer = vdp_ref.framebuffer();
        let has_cube_pixel =
            (0..240).any(|y| (0..320).any(|x| framebuffer.get_pixel(x, y) != (0, 0, 0)));
        assert!(
            has_cube_pixel,
            "VPU command stream produced no visible pixels"
        );
        if expect_sgc_overlay {
            assert_eq!(
                sgc_status & 0x18,
                0,
                "SGC reported a FIFO error: 0x{sgc_status:08X}"
            );
            let has_overlay_glyph = if expect_gp_noise_background {
                let mut found = false;
                for y in 8..24 {
                    for x in 8..16 {
                        let background = gp0_noise_pixel(&mut cpu, x, y);
                        if framebuffer.get_pixel(x, y) != (background, background, background) {
                            found = true;
                            break;
                        }
                    }
                    if found {
                        break;
                    }
                }
                found
            } else {
                (8..16).any(|x| (8..24).any(|y| framebuffer.get_pixel(x, y) != (0, 0, 0)))
            };
            assert!(has_overlay_glyph, "SGC text overlay produced no visible glyph");
            let has_white_overlay_pixel =
                (8..16).any(|x| (8..24).any(|y| framebuffer.get_pixel(x, y) == (255, 255, 255)));
            assert!(
                has_white_overlay_pixel,
                "SGC glyph CLUT color is not white"
            );
        }
        if expect_gp_noise_background {
            let noise_pixels = [(1, 1), (318, 1), (1, 238), (318, 238)].map(|(x, y)| {
                let expected = gp0_noise_pixel(&mut cpu, x, y);
                (expected, framebuffer.get_pixel(x, y))
            });
            assert!(
                noise_pixels
                    .iter()
                    .all(|&(expected, actual)| actual == (expected, expected, expected)),
                "GP0 noise is not visible through the transparent VPU background: {noise_pixels:?}"
            );
            assert!(
                noise_pixels.iter().any(|&(value, _)| value != 0)
                    && noise_pixels
                        .windows(2)
                        .any(|pair| pair[0].0 != pair[1].0),
                "GP0 bitmap has no visible noise variation: {noise_pixels:?}"
            );
            let mut has_vpu_cube_pixel = false;
            for y in 64..180 {
                for x in 80..240 {
                    let expected = gp0_noise_pixel(&mut cpu, x, y);
                    if framebuffer.get_pixel(x, y) != (expected, expected, expected) {
                        has_vpu_cube_pixel = true;
                        break;
                    }
                }
                if has_vpu_cube_pixel {
                    break;
                }
            }
            assert!(
                has_vpu_cube_pixel,
                "VPU cube produced no visible pixels over the GP0 backdrop"
            );
        }
        println!(
            "PASS: {path} rendered (running={}); cycles={}; VPU status=0x{vpu_status:08X}",
            cpu.is_running(),
            cpu.cycles()
        );
    } else {
        assert_eq!(
            result, expected,
            "R1 mismatch: got {result}, expected {expected}"
        );
        println!("PASS: {path} => R1={result} cycles={}", cpu.cycles());
    }
}
