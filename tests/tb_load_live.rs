//! Repro: is the REPL alive after LOAD? With disk attached (scalar + pipelined).
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu::{CYCLES_PER_FRAME, Cpu};
use cpt32::devices::pec::idc::idc::{Idc, connect_idc};
use cpt32::devices::pec::pec::{HostInputEvent, connect_pec_input};
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::vdp::vdp::VDP_VRAM_BASE;
use tokio::sync::mpsc::UnboundedSender;

fn read_text_plane(cpu: &mut Cpu) -> String {
    let mut text = String::new();
    for cell in 0..80 * 30 {
        let byte = cpu.read_mem_u8(VDP_VRAM_BASE + cell as u32);
        if byte.is_ascii_graphic() || byte == b' ' {
            text.push(byte as char);
        } else {
            text.push('\n');
        }
    }
    text
}

fn type_key(sender: &UnboundedSender<HostInputEvent>, usage: u8) {
    sender
        .send(HostInputEvent::Key { usage, down: true })
        .unwrap();
    sender
        .send(HostInputEvent::Key { usage, down: false })
        .unwrap();
}

fn boot() -> (
    Cpu,
    std::rc::Rc<std::cell::RefCell<cpt32::devices::vdp::vdp::Vdp>>,
    UnboundedSender<HostInputEvent>,
) {
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    let root = std::env::temp_dir().join(format!("tb-loadlive-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("disk0.bas"), b"10 print 99\n").unwrap();
    std::fs::write(
            root.join("config.yaml"),
            "disk:\n  - slot: 0\n    type: fdd\n    image: disk0.bas\n    read_only: true\n    transfer_rate_bytes_per_second: 48000000\n    access_latency_ms: 0\n    incomplete_sector_policy: pad\n",
        )
        .unwrap();
    let drives = Idc::load_drives(&root.join("config.yaml")).unwrap();
    connect_idc(&mut bus, drives);
    let mut cpu = Cpu::new(bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin")).unwrap();
    cpu.load_program(0, &image);
    (cpu, vdp, input)
}

fn run_frames(
    cpu: &mut Cpu,
    vdp: &std::cell::RefCell<cpt32::devices::vdp::vdp::Vdp>,
    input: &UnboundedSender<HostInputEvent>,
    text: &str,
    frames: usize,
) {
    for c in text.chars() {
        match c {
            'a'..='z' => type_key(input, (c as u8) - b'a' + 4),
            '0' => type_key(input, 39),
            '1'..='9' => type_key(input, (c as u8) - b'1' + 30),
            ' ' => type_key(input, 44),
            '\n' => type_key(input, 40),
            '*' => {
                input
                    .send(HostInputEvent::Key {
                        usage: 225,
                        down: true,
                    })
                    .unwrap();
                type_key(input, 37);
                input
                    .send(HostInputEvent::Key {
                        usage: 225,
                        down: false,
                    })
                    .unwrap();
            }
            _ => panic!("bad char {c}"),
        }
    }
    for _ in 0..frames {
        vdp.borrow_mut().tick();
        cpu.set_irq_source(0, true);
        cpu.set_irq_source(0, false);
        cpu.run(CYCLES_PER_FRAME as usize);
    }
}
fn run_until_halted(
    cpu: &mut Cpu,
    vdp: &std::cell::RefCell<cpt32::devices::vdp::vdp::Vdp>,
    max_frames: usize,
) {
    for _ in 0..max_frames {
        if !cpu.is_running() {
            return;
        }
        vdp.borrow_mut().tick();
        cpu.set_irq_source(0, true);
        cpu.set_irq_source(0, false);
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    assert!(!cpu.is_running(), "CPU did not return to its input wait");
}

fn read_text_plane_any(cpu: &mut cpt32::cpu_select::AnyCpu) -> String {
    let mut text = String::new();
    for cell in 0..80 * 30 {
        let byte = cpu.read_mem_u8(VDP_VRAM_BASE + cell as u32);
        if byte.is_ascii_graphic() || byte == b' ' {
            text.push(byte as char);
        } else {
            text.push('\n');
        }
    }
    text
}

#[test]
fn tb_repl_alive_after_load_pipelined() {
    let root = std::env::temp_dir().join(format!("tb-loadpipe-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("disk0.bas"), b"10 print 99\n").unwrap();
    std::fs::write(
        root.join("config.yaml"),
        "disk:\n  - slot: 0\n    type: fdd\n    image: disk0.bas\n    read_only: true\n    transfer_rate_bytes_per_second: 48000000\n    access_latency_ms: 0\n    incomplete_sector_policy: pad\n",
    )
    .unwrap();
    let drives = Idc::load_drives(&root.join("config.yaml")).unwrap();
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    connect_idc(&mut bus, drives);
    let mut cpu = cpt32::cpu_select::AnyCpu::new(cpt32::cpu_select::CpuKind::Pipelined, bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin")).unwrap();
    cpu.load_program(0, &image);
    for c in "load\n".chars() {
        match c {
            'a'..='z' => type_key(&input, (c as u8) - b'a' + 4),
            '\n' => type_key(&input, 40),
            _ => panic!("bad"),
        }
    }
    for _ in 0..16 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let t1 = read_text_plane_any(&mut cpu);
    assert!(t1.contains("LOADED"), "LOAD failed on pipeline:\n{t1}");
    for c in "run\n".chars() {
        match c {
            'a'..='z' => type_key(&input, (c as u8) - b'a' + 4),
            '\n' => type_key(&input, 40),
            _ => panic!("bad"),
        }
        // Model GUI input arriving after the guest has returned to HALT;
        // do not prequeue the whole command before CPU execution.
        for _ in 0..3 {
            vdp.borrow_mut().tick();
            cpu.run(CYCLES_PER_FRAME as usize);
        }
    }
    for _ in 0..16 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let t1b = read_text_plane_any(&mut cpu);
    assert!(
        t1b.contains("99"),
        "RUN after LOAD failed on pipeline:\n{t1b}"
    );
    for c in "print 7*6\n".chars() {
        match c {
            'a'..='z' => type_key(&input, (c as u8) - b'a' + 4),
            '0' => type_key(&input, 39),
            '1'..='9' => type_key(&input, (c as u8) - b'1' + 30),
            ' ' => type_key(&input, 44),
            '*' => {
                input
                    .send(HostInputEvent::Key {
                        usage: 225,
                        down: true,
                    })
                    .unwrap();
                type_key(&input, 37);
                input
                    .send(HostInputEvent::Key {
                        usage: 225,
                        down: false,
                    })
                    .unwrap();
            }
            '\n' => type_key(&input, 40),
            _ => panic!("bad"),
        }
    }
    for _ in 0..16 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let t2 = read_text_plane_any(&mut cpu);
    println!("=== PIPE AFTER ===\n{t2}\n==================");
    assert!(t2.contains("42"), "REPL dead after LOAD on pipeline:\n{t2}");
}

#[test]
fn tb_repl_alive_after_load_with_disk() {
    let (mut cpu, vdp, input) = boot();
    run_frames(&mut cpu, &vdp, &input, "load\n", 16);
    let t1 = read_text_plane(&mut cpu);
    println!("=== AFTER LOAD ===\n{t1}\n==================");
    assert!(t1.contains("LOADED"), "LOAD failed:\n{t1}");
    for c in "run\n".chars() {
        let one = c.to_string();
        run_frames(&mut cpu, &vdp, &input, &one, 3);
    }
    for _ in 0..16 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let t_run = read_text_plane(&mut cpu);
    assert!(
        t_run.contains("99"),
        "RUN after LOAD failed on scalar:\n{t_run}"
    );
    for c in "print 7*6\n".chars() {
        let one = c.to_string();
        run_frames(&mut cpu, &vdp, &input, &one, 3);
    }
    for _ in 0..16 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let t2 = read_text_plane(&mut cpu);
    println!("=== SCALAR AFTER LOAD/RUN/INPUT ===\n{t2}\n===================================");
    assert!(t2.contains("42"), "REPL dead after LOAD on scalar:\n{t2}");
}

#[test]
fn tb_repl_accepts_input_after_full_size_load() {
    let root = std::env::temp_dir().join(format!("tb-load-input-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut disk_image = Vec::new();
    let filler = format!("100 REM {}\n", "X".repeat(100));
    let program = b"10 print 99\n";
    while disk_image.len() + filler.len() + program.len() <= 128 * 512 {
        disk_image.extend_from_slice(filler.as_bytes());
    }
    disk_image.extend_from_slice(program);
    disk_image.resize(128 * 512, b' ');
    std::fs::write(root.join("disk0.bas"), disk_image).unwrap();
    std::fs::write(
        root.join("config.yaml"),
        "disk:\n  - slot: 0\n    type: fdd\n    image: disk0.bas\n    read_only: true\n    transfer_rate_bytes_per_second: 48000000\n    access_latency_ms: 0\n    incomplete_sector_policy: pad\n",
    )
    .unwrap();
    let drives = Idc::load_drives(&root.join("config.yaml")).unwrap();
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    connect_idc(&mut bus, drives);
    let mut cpu = Cpu::new(bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin")).unwrap();
    cpu.load_program(0, &image);

    run_frames(&mut cpu, &vdp, &input, "load\n", 16);
    for _ in 0..128 {
        if read_text_plane(&mut cpu).contains("LOADED") {
            break;
        }
        run_frames(&mut cpu, &vdp, &input, "", 1);
    }
    let after_load = read_text_plane(&mut cpu);
    assert!(
        after_load.contains("LOADED"),
        "LOAD did not finish (running={}, pc=0x{:08X}, instructions={}):\n{after_load}",
        cpu.is_running(),
        cpu.pc(),
        cpu.instructions_executed()
    );
    for c in "print 7*6\n".chars() {
        run_frames(&mut cpu, &vdp, &input, &c.to_string(), 3);
    }
    for _ in 0..16 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }

    let text = read_text_plane(&mut cpu);
    assert!(
        text.contains("42"),
        "REPL did not accept keyboard input after full-size LOAD:\n{text}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

// A default LOAD reads 128 sectors (64 KiB); Pipeline CPU emulation can take
// about 100 seconds to finish this guest-side line scan.
#[test]
fn tb_repl_accepts_input_after_full_size_load_pipelined() {
    let root = std::env::temp_dir().join(format!("tb-load-input-pipe-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut disk_image = Vec::new();
    let filler = format!("100 REM {}\n", "X".repeat(100));
    let program = b"10 print 99\n";
    while disk_image.len() + filler.len() + program.len() <= 128 * 512 {
        disk_image.extend_from_slice(filler.as_bytes());
    }
    disk_image.extend_from_slice(program);
    disk_image.resize(128 * 512, b' ');
    std::fs::write(root.join("disk0.bas"), disk_image).unwrap();
    std::fs::write(
        root.join("config.yaml"),
        "disk:\n  - slot: 0\n    type: fdd\n    image: disk0.bas\n    read_only: true\n    transfer_rate_bytes_per_second: 48000000\n    access_latency_ms: 0\n    incomplete_sector_policy: pad\n",
    )
    .unwrap();
    let drives = Idc::load_drives(&root.join("config.yaml")).unwrap();
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    connect_idc(&mut bus, drives);
    let mut cpu = cpt32::cpu_select::AnyCpu::new(cpt32::cpu_select::CpuKind::Pipelined, bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin")).unwrap();
    cpu.load_program(0, &image);

    for _ in 0..8 {
        vdp.borrow_mut().tick();
        cpu.set_irq_source(0, true);
        cpu.set_irq_source(0, false);
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    type_pipeline_text(&mut cpu, &vdp, &input, "load");
    input
        .send(HostInputEvent::Key {
            usage: 40,
            down: true,
        })
        .unwrap();
    // Match the GUI: Enter key-up may arrive while the synchronous LOAD scan
    // is still executing, after key-down has already started the command.
    vdp.borrow_mut().tick();
    cpu.set_irq_source(0, true);
    cpu.set_irq_source(0, false);
    cpu.run(CYCLES_PER_FRAME as usize);
    input
        .send(HostInputEvent::Key {
            usage: 40,
            down: false,
        })
        .unwrap();
    vdp.borrow_mut().tick();
    cpu.set_irq_source(0, true);
    cpu.set_irq_source(0, false);
    cpu.run(CYCLES_PER_FRAME as usize);
    let after_load = wait_pipeline_text(&mut cpu, &vdp, "LOADED", 128);
    assert!(
        after_load.contains("LOADED"),
        "large LOAD did not finish:\n{after_load}"
    );
    wait_pipeline_halted(&mut cpu, &vdp, 256);

    type_pipeline_text(&mut cpu, &vdp, &input, "print 42\n");
    let after_input = wait_pipeline_text(&mut cpu, &vdp, "42", 64);
    assert!(
        after_input.contains("42"),
        "Pipeline REPL did not accept input after full-size LOAD:\n{after_input}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn tb_repl_accepts_input_after_list_and_run() {
    let (mut cpu, vdp, input) = boot();
    run_frames(&mut cpu, &vdp, &input, "10 print 99\n", 16);

    run_frames(&mut cpu, &vdp, &input, "list\n", 1);
    run_frames(&mut cpu, &vdp, &input, "print 42\n", 1);
    run_until_halted(&mut cpu, &vdp, 256);
    assert!(
        read_text_plane(&mut cpu).contains("10 print 99"),
        "LIST did not finish before returning to input"
    );
    assert!(
        read_text_plane(&mut cpu).contains("42"),
        "keyboard input queued during LIST was not accepted"
    );

    run_frames(&mut cpu, &vdp, &input, "run\n", 16);
    run_until_halted(&mut cpu, &vdp, 256);
    assert!(
        read_text_plane(&mut cpu).contains("99"),
        "RUN did not finish before returning to input"
    );

    run_frames(&mut cpu, &vdp, &input, "print 42\n", 16);
    assert!(
        read_text_plane(&mut cpu).contains("42"),
        "keyboard input was not accepted after RUN"
    );
}

#[test]
fn tb_repl_accepts_input_after_list_and_run_pipelined() {
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    let mut cpu = cpt32::cpu_select::AnyCpu::new(cpt32::cpu_select::CpuKind::Pipelined, bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin")).unwrap();
    cpu.load_program(0, &image);

    for _ in 0..8 {
        vdp.borrow_mut().tick();
        cpu.set_irq_source(0, true);
        cpu.set_irq_source(0, false);
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    type_pipeline_text(&mut cpu, &vdp, &input, "10 print 99\n");
    type_pipeline_text(&mut cpu, &vdp, &input, "list\n");
    type_pipeline_text(&mut cpu, &vdp, &input, "print 42\n");
    wait_pipeline_halted(&mut cpu, &vdp, 256);
    assert!(
        read_text_plane_any(&mut cpu).contains("10 print 99"),
        "LIST did not finish before returning to input"
    );

    assert!(
        read_text_plane_any(&mut cpu).contains("42"),
        "keyboard input was not accepted after LIST"
    );

    type_pipeline_text(&mut cpu, &vdp, &input, "run\n");
    wait_pipeline_halted(&mut cpu, &vdp, 256);
    assert!(
        read_text_plane_any(&mut cpu).contains("99"),
        "RUN did not finish before returning to input"
    );

    type_pipeline_text(&mut cpu, &vdp, &input, "print 42\n");
    assert!(
        read_text_plane_any(&mut cpu).contains("42"),
        "keyboard input was not accepted after RUN"
    );
}

fn wait_pipeline_halted(
    cpu: &mut cpt32::cpu_select::AnyCpu,
    vdp: &std::cell::RefCell<cpt32::devices::vdp::vdp::Vdp>,
    max_frames: usize,
) {
    for _ in 0..max_frames {
        if !cpu.is_running() {
            return;
        }
        vdp.borrow_mut().tick();
        cpu.set_irq_source(0, true);
        cpu.set_irq_source(0, false);
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    assert!(
        !cpu.is_running(),
        "pipeline CPU did not return to input wait"
    );
}
#[test]
fn tb_repl_accepts_input_after_configured_hdd_load() {
    let config = concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml");
    let drives =
        Idc::load_drives(std::path::Path::new(config)).expect("configured HDD could not be opened");
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    connect_idc(&mut bus, drives);
    let mut cpu = cpt32::cpu_select::AnyCpu::new(cpt32::cpu_select::CpuKind::Pipelined, bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin")).unwrap();
    cpu.load_program(0, &image);

    for _ in 0..8 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    for command in ["load\n", "print 7*6\n"] {
        for c in command.chars() {
            match c {
                'a'..='z' => type_key(&input, (c as u8) - b'a' + 4),
                '0' => type_key(&input, 39),
                '1'..='9' => type_key(&input, (c as u8) - b'1' + 30),
                ' ' => type_key(&input, 44),
                '\n' => type_key(&input, 40),
                '*' => {
                    input
                        .send(HostInputEvent::Key {
                            usage: 225,
                            down: true,
                        })
                        .unwrap();
                    type_key(&input, 37);
                    input
                        .send(HostInputEvent::Key {
                            usage: 225,
                            down: false,
                        })
                        .unwrap();
                }
                _ => panic!("unsupported char {c}"),
            }
            for _ in 0..3 {
                vdp.borrow_mut().tick();
                cpu.run(CYCLES_PER_FRAME as usize);
            }
            if command == "load\n" && c == '\n' {
                for _ in 0..64 {
                    if read_text_plane_any(&mut cpu).contains("LOADED") {
                        break;
                    }
                    vdp.borrow_mut().tick();
                    cpu.run(CYCLES_PER_FRAME as usize);
                }
                let text = read_text_plane_any(&mut cpu);
                assert!(
                    text.contains("LOADED"),
                    "configured HDD LOAD did not finish:\n{text}"
                );
            }
        }
    }
    for _ in 0..16 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let text = read_text_plane_any(&mut cpu);
    assert!(
        text.contains("42"),
        "keyboard input failed after configured HDD LOAD:\n{text}"
    );

    type_pipeline_text(&mut cpu, &vdp, &input, "run\n");
    assert!(
        wait_pipeline_text(&mut cpu, &vdp, "do you want instructions?", 128)
            .contains("do you want instructions?"),
        "loaded game did not reach its first INPUT"
    );
    type_pipeline_text(&mut cpu, &vdp, &input, "n\n");
    assert!(
        wait_pipeline_text(&mut cpu, &vdp, "difficult game", 128).contains("difficult game"),
        "loaded game did not accept the first answer"
    );
    type_pipeline_text(&mut cpu, &vdp, &input, "n\n");
    assert!(
        wait_pipeline_text(&mut cpu, &vdp, "Captain:", 128).contains("Captain:"),
        "loaded game did not reach Captain INPUT"
    );
    type_pipeline_text(&mut cpu, &vdp, &input, "q\n");
    let after_quit = wait_pipeline_text(&mut cpu, &vdp, "Another game?", 128);
    assert!(
        after_quit.contains("Another game?"),
        "loaded game did not accept Captain input:\n{after_quit}"
    );
}

fn type_pipeline_text(
    cpu: &mut cpt32::cpu_select::AnyCpu,
    vdp: &std::cell::RefCell<cpt32::devices::vdp::vdp::Vdp>,
    input: &UnboundedSender<HostInputEvent>,
    text: &str,
) {
    for c in text.chars() {
        match c {
            'a'..='z' => type_key(input, (c as u8) - b'a' + 4),
            '0' => type_key(input, 39),
            '1'..='9' => type_key(input, (c as u8) - b'1' + 30),
            ' ' => type_key(input, 44),
            '\n' => type_key(input, 40),
            _ => panic!("unsupported char {c}"),
        }
        for _ in 0..3 {
            vdp.borrow_mut().tick();
            cpu.set_irq_source(0, true);
            cpu.set_irq_source(0, false);
            cpu.run(CYCLES_PER_FRAME as usize);
        }
    }
}

fn wait_pipeline_text(
    cpu: &mut cpt32::cpu_select::AnyCpu,
    vdp: &std::cell::RefCell<cpt32::devices::vdp::vdp::Vdp>,
    needle: &str,
    max_frames: usize,
) -> String {
    for _ in 0..max_frames {
        let text = read_text_plane_any(cpu);
        if text.contains(needle) {
            return text;
        }
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    read_text_plane_any(cpu)
}
