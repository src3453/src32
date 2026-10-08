//! Headless SRC32 execution harness for compiler pipeline smoke tests.
use std::env;
use std::fs;

use cpt32::bus::Bus;
use cpt32::bus::connect_devices_with_vdp;
use cpt32::cpu::{Cpu, CYCLES_PER_FRAME};

use cpt32::devices::pec::idc::idc::connect_idc_from_config;
use cpt32::devices::pec::pec::connect_pec_input;
use cpt32::devices::pec::rng::connect_rng;

fn gp0_noise_pixel(cpu: &mut Cpu, x: usize, y: usize) -> u8 {
    let index = cpu.read_mem_u8(0x1000_0000 + (y * 320 + x) as u32);
    cpu.read_mem_u8(0x1001_2C00 + u32::from(index) * 3)
}
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let path = args
        .first()
        .expect("usage: src32_testbench PROGRAM.bin [expected] [--allow-running] [--expect-sgc-overlay] [--expect-gp-noise-background]");
    let allow_running = args.iter().any(|arg| arg == "--allow-running");
    let expect_sgc_overlay = args.iter().any(|arg| arg == "--expect-sgc-overlay");
    let expect_gp_noise_background = args.iter().any(|arg| arg == "--expect-gp-noise-background");
    let expected: u32 = args
        .iter()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .map_or(Ok(0), |value| value.parse())
        .expect("expected must be an unsigned integer");
    let image = fs::read(&path).expect("failed to read program binary");

    let mut bus = Bus::new();
    let _input_sender = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    connect_idc_from_config(&mut bus).expect("Invalid disk configuration");
    let mut cpu = Cpu::new(bus);
    cpu.load_program(0, &image);
    let cycles_per_frame = CYCLES_PER_FRAME as usize;
    let cycle_budget = if expect_gp_noise_background {
        50_000_000
    } else {
        5_000_000
    };
    let frame_count = (cycle_budget + cycles_per_frame - 1) / cycles_per_frame;
    for _ in 0..frame_count {
        vdp.borrow_mut().tick();
        cpu.set_irq_source(0, true);
        cpu.set_irq_source(0, false);
        cpu.run(cycles_per_frame);
    }

    let result = cpu.read_reg(1);
    assert!(
        allow_running || !cpu.is_running(),
        "program did not halt (pc=0x{:08X})",
        cpu.pc()
    );
    if allow_running {
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
