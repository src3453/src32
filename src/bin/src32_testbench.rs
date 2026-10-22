//! Headless SRC32 execution harness for compiler pipeline smoke tests.
use std::env;
use std::fs;

use cpt32::bus::Bus;
use cpt32::bus::connect_devices_with_memory;
use cpt32::cpu::{CPU_PROFILE_STAGE_NAMES, CYCLES_PER_FRAME, CpuProfile};
use cpt32::cpu_select::{AnyCpu, resolve_cpu_kind, split_cpu_arg};

use cpt32::devices::pec::idc::idc::connect_idc_from_config;
use cpt32::devices::pec::pec::{HostInputEvent, connect_pec_input};
use cpt32::devices::pec::rng::connect_rng;

use cpt32::music::guest::{STREAM_BASE, read_music_stream};
use std::path::Path;

fn integer(text: &str) -> u32 {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).expect("invalid hexadecimal integer")
    } else {
        text.parse().expect("invalid integer")
    }
}
/// HID keyboard usages (page 0x07, US layout) that type `text`. Escapes:
/// `\n` Enter, `\b` Backspace, `\t` Tab, `\e` Esc, `\\` a backslash,
/// `\L` `\R` `\U` `\D` arrows, `\H` Home, `\E` End, `\X` Delete, `\C` Ctrl+C.
fn typed_keys(text: &str) -> Vec<HostInputEvent> {
    const SHIFT: u8 = 225;
    const CTRL: u8 = 224;
    fn usage(c: char) -> Option<(u8, bool)> {
        const PLAIN: &str = "1234567890\0\0\0\0 -=[]\\\0;'`,./";
        const SHIFTED: &str = "!@#$%^&*()\0\0\0\0 _+{}|\0:\"~<>?";
        match c {
            'a'..='z' => Some((4 + (c as u8 - b'a'), false)),
            'A'..='Z' => Some((4 + (c as u8 - b'A'), true)),
            _ => PLAIN
                .chars()
                .position(|p| p == c)
                .map(|i| (30 + i as u8, false))
                .or_else(|| {
                    SHIFTED
                        .chars()
                        .position(|p| p == c)
                        .map(|i| (30 + i as u8, true))
                }),
        }
    }
    let mut events = Vec::new();
    let mut press = |code: u8, modifier: Option<u8>| {
        if let Some(m) = modifier {
            events.push(HostInputEvent::Key {
                usage: m,
                down: true,
            });
        }
        events.push(HostInputEvent::Key {
            usage: code,
            down: true,
        });
        events.push(HostInputEvent::Key {
            usage: code,
            down: false,
        });
        if let Some(m) = modifier {
            events.push(HostInputEvent::Key {
                usage: m,
                down: false,
            });
        }
    };
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        let (code, modifier) = if c == '\\' {
            match chars.next().expect("--type: escape at end of text") {
                'n' => (40, None),
                'b' => (42, None),
                't' => (43, None),
                'e' => (41, None),
                'L' => (80, None),
                'R' => (79, None),
                'U' => (82, None),
                'D' => (81, None),
                'H' => (74, None),
                'E' => (77, None),
                'X' => (76, None),
                'C' => (6, Some(CTRL)),
                '\\' => (49, None),
                other => panic!("--type: unknown escape \\{other}"),
            }
        } else {
            let (code, shifted) = usage(c).unwrap_or_else(|| panic!("--type: no key for {c:?}"));
            (code, shifted.then_some(SHIFT))
        };
        press(code, modifier);
    }
    events
}

/// The visible PCG text screen, one line per row with trailing blanks
/// removed. Follows SCREEN_MODE and SCROLL_Y; modes 0, 1 and 3 only.
fn pcg_screen(cpu: &mut AnyCpu) -> Vec<String> {
    let mode = cpu.read_mem_u8(0x8000_F003);
    let (columns, cell_height) = match mode {
        0 => (40, 8),
        3 => (80, 16),
        _ => (80, 8),
    };
    let scroll_y =
        u32::from(cpu.read_mem_u8(0x8000_F00E)) | u32::from(cpu.read_mem_u8(0x8000_F00F)) << 8;
    let top = scroll_y / cell_height;
    (0..30)
        .map(|row| {
            let ring = (top + row) % 30;
            let line: String = (0..columns)
                .map(|column| {
                    let byte = cpu.read_mem_u8(0x1000_0000 + ring * columns + column);
                    if (0x20..0x7F).contains(&byte) {
                        byte as char
                    } else {
                        ' '
                    }
                })
                .collect();
            line.trim_end().to_string()
        })
        .collect()
}

fn gp0_noise_pixel(cpu: &mut AnyCpu, x: usize, y: usize) -> u8 {
    let index = cpu.read_mem_u8(0x1000_0000 + (y * 320 + x) as u32);
    cpu.read_mem_u8(0x1001_2C00 + u32::from(index) * 3)
}
fn main() {
    cpt32::init_logging();

    let args: Vec<String> = env::args().skip(1).collect();
    let path = args.first().expect("usage: src32_testbench PROGRAM.bin [expected] [--allow-running] [--music-stream PATH] [--frames N] [--irq-drain-cycles N] [--profile-cpu] [--skip-render-checks] [--expect-mem-u8 ADDRESS=VALUE] [--type FRAME=TEXT] [--dump-pcg] [--cpu scalar|cached|cached-relaxed|jit|pipeline]");
    let (args, cli_cpu) = split_cpu_arg(&args).expect("invalid --cpu option");
    let cpu_kind =
        resolve_cpu_kind(cli_cpu, Path::new("config.yaml")).expect("Invalid CPU configuration");
    eprintln!(
        "src32_testbench: CPU micro-architecture: {}",
        cpu_kind.as_str()
    );
    let mut allow_running = false;
    let mut expect_sgc_overlay = false;
    let mut expect_gp_noise_background = false;
    let mut expected = 0;
    let mut music_stream = None;
    let mut frames = None;
    let mut irq_drain_cycles = 0;
    let mut profile_cpu = false;
    let mut skip_render_checks = false;
    let mut memory_expectations = Vec::new();
    let mut typed: Vec<(usize, Vec<HostInputEvent>)> = Vec::new();
    let mut dump_pcg = false;
    let mut options = args.iter().skip(1);
    while let Some(option) = options.next() {
        match option.as_str() {
            "--allow-running" => allow_running = true,
            "--expect-sgc-overlay" => expect_sgc_overlay = true,
            "--expect-gp-noise-background" => expect_gp_noise_background = true,
            "--music-stream" => {
                music_stream = Some(options.next().expect("--music-stream requires PATH"))
            }
            "--frames" => {
                frames = Some(integer(options.next().expect("--frames requires N")) as usize)
            }
            "--irq-drain-cycles" => {
                irq_drain_cycles =
                    integer(options.next().expect("--irq-drain-cycles requires N")) as usize;
            }
            "--profile-cpu" => profile_cpu = true,
            "--skip-render-checks" => skip_render_checks = true,
            "--expect-mem-u8" => {
                let (address, value) = options
                    .next()
                    .expect("--expect-mem-u8 requires ADDRESS=VALUE")
                    .split_once('=')
                    .expect("expected ADDRESS=VALUE");
                memory_expectations.push((
                    integer(address),
                    u8::try_from(integer(value)).expect("value must fit u8"),
                ));
            }
            "--type" => {
                let (frame, text) = options
                    .next()
                    .expect("--type requires FRAME=TEXT")
                    .split_once('=')
                    .expect("expected FRAME=TEXT");
                typed.push((integer(frame) as usize, typed_keys(text)));
            }
            "--dump-pcg" => dump_pcg = true,
            _ if option.starts_with('-') => panic!("unknown option: {option}"),
            _ => expected = integer(option),
        }
    }
    let image = fs::read(path).expect("failed to read program binary");
    assert!(
        !skip_render_checks || !(expect_sgc_overlay || expect_gp_noise_background),
        "--skip-render-checks conflicts with explicit rendering expectations"
    );
    let stream = music_stream
        .map(|path| read_music_stream(Path::new(path), image.len()).expect("invalid music stream"));

    let mut bus = Bus::new();
    let input_sender = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let (ram_size, vram_size) =
        cpt32::memory_config::memory_sizes_from_config(Path::new("config.yaml"))
            .expect("Invalid memory configuration");
    let vdp = connect_devices_with_memory(&mut bus, ram_size, vram_size);
    connect_idc_from_config(&mut bus).expect("Invalid disk configuration");
    cpt32::devices::expansion::load_expansion_cards(Path::new("config.yaml"), &mut bus)
        .expect("Invalid expansion-card configuration");
    let bios_entry = cpt32::devices::bios::load_bios(Path::new("config.yaml"), &mut bus)
        .expect("Invalid BIOS configuration");
    let mut cpu = AnyCpu::new_configured(cpu_kind, bus, Path::new("config.yaml"))
        .expect("Invalid CPU configuration");
    cpu.enable_profiling(profile_cpu);
    if cpt32::wax::is_wax(&image) {
        let img = cpt32::wax::parse(&image).expect("invalid WAX program");
        assert!(!img.is_lib, ".wal library is not runnable");
        cpt32::wax::reject_protected_direct_load(&img).expect("unsupported WAX program");
        for block in &img.blocks {
            cpu.load_program(block.load_address, block.data);
        }
        cpu.set_pc(img.entry);
    } else {
        cpu.load_program(0, &image);
    }
    if let Some(entry) = bios_entry {
        cpu.set_pc(entry);
    }
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
    let mut cpu_profile = CpuProfile::default();
    for frame in 0..frame_count {
        let mut sent = false;
        for (_, events) in typed.iter().filter(|(at, _)| *at == frame) {
            for &event in events {
                input_sender.send(event).expect("PeC input thread stopped");
            }
            sent = true;
        }
        if sent {
            // The PeC input thread applies host events asynchronously.
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        vdp.borrow_mut().tick();
        cpu.set_irq_source(0, true);
        cpu.set_irq_source(0, false);
        cpu.run(cycles_per_frame);
        // Optional completed-handler checks, not a real-time playback budget.
        // Dense unoptimized SGUB frames can outlive one CPU frame. Drain only
        // the current IRQ, without ticking VDP or delivering another VSYNC.
        if irq_drain_cycles != 0 {
            let mut remaining = irq_drain_cycles;
            while !cpu.irq_enabled() && remaining != 0 {
                let chunk = remaining.min(cycles_per_frame);
                cpu.run(chunk);
                remaining -= chunk;
            }
            assert!(
                cpu.irq_enabled(),
                "frame {frame}: IRQ did not return within {irq_drain_cycles} extra cycles (pc=0x{:08X})",
                cpu.pc()
            );
        }
        if let Some(profile) = cpu.take_profile() {
            cpu_profile.accumulate(profile);
        }
        for &(address, expected) in &memory_expectations {
            let actual = cpu.read_mem_u8(address);
            assert_eq!(actual, expected, "frame {frame}: memory 0x{address:08X}");
        }
    }

    if profile_cpu {
        let total = cpu_profile.total();
        println!(
            "CPU profile: ticks={} measured={:.3} ms",
            cpu_profile.ticks,
            total.as_secs_f64() * 1_000.0
        );
        for (name, duration) in CPU_PROFILE_STAGE_NAMES.iter().zip(cpu_profile.stages) {
            let share = if total.is_zero() {
                0.0
            } else {
                duration.as_secs_f64() * 100.0 / total.as_secs_f64()
            };
            println!(
                "  {name:<24} {:>7.3} ms ({share:>5.1}%)",
                duration.as_secs_f64() * 1_000.0
            );
        }
        println!(
            "  Profiling adds per-stage timing overhead; do not use this run for wall-clock comparisons."
        );
    }

    if let Some(stats) = cpu.jit_stats() {
        println!("CPU JIT: {stats:?}");
    }
    if dump_pcg {
        println!("PCG SCREEN BEGIN");
        for line in pcg_screen(&mut cpu) {
            println!("{line}");
        }
        println!("PCG SCREEN END");
    }
    let result = cpu.read_reg(1);
    assert!(
        allow_running || !cpu.is_running(),
        "program did not halt (pc=0x{:08X})",
        cpu.pc()
    );
    if skip_render_checks {
        println!(
            "EXECUTED: {path}; frames={frame_count}; cycles={}; pc=0x{:08X}; R1={result}; running={}",
            cpu.cycles(),
            cpu.pc(),
            cpu.is_running()
        );
    } else if allow_running && profile_cpu {
        println!(
            "PROFILED: {path}; frames={frame_count}; cycles={}",
            cpu.cycles()
        );
    } else if allow_running && music_stream.is_some() {
        println!(
            "PASS: {path} music stream; frames={frame_count}; memory checks={}; cycles={}",
            memory_expectations.len(),
            cpu.cycles()
        );
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
            assert!(
                has_overlay_glyph,
                "SGC text overlay produced no visible glyph"
            );
            let has_white_overlay_pixel =
                (8..16).any(|x| (8..24).any(|y| framebuffer.get_pixel(x, y) == (255, 255, 255)));
            assert!(has_white_overlay_pixel, "SGC glyph CLUT color is not white");
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
                    && noise_pixels.windows(2).any(|pair| pair[0].0 != pair[1].0),
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
