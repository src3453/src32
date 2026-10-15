//! Smoke tests for the sol TinyBASIC interpreter on the real SRC32 CPU.
//!
//! The interpreter boots, initializes the PCG console, renders its
//! help banner and REPL prompt, and executes a program typed through
//! PeC keyboard events. The PCG text plane (legacy mode) stores one
//! ASCII byte per cell, so the rendered output can be read back
//! directly from VRAM.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu::{CYCLES_PER_FRAME, Cpu};
use cpt32::devices::pec::pec::{HostInputEvent, connect_pec_input};
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::vdp::vdp::{VDP_VRAM_BASE, Vdp};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

/// Read the legacy PCG text plane (40x30 cells, one byte per cell)
/// as an ASCII string.
fn read_text_plane(cpu: &mut Cpu) -> String {
    let mut text = String::new();
    for cell in 0..40 * 30 {
        let byte = cpu.read_mem_u8(VDP_VRAM_BASE + cell as u32);
        if byte.is_ascii_graphic() || byte == b' ' {
            text.push(byte as char);
        } else {
            text.push(' ');
        }
    }
    text
}

fn load_cpu() -> (Cpu, Rc<RefCell<Vdp>>) {
    let mut bus = Bus::new();
    let _input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    let mut cpu = Cpu::new(bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin"))
        .expect("tinybasic.bin missing; assemble tinybasic.sol first");
    cpu.load_program(0, &image);
    (cpu, vdp)
}

#[test]
fn tinybasic_boot_renders_help_and_prompt() {
    let (mut cpu, vdp) = load_cpu();

    // Boot: initPCG, print the help banner, then the REPL reaches
    // its first "OK" / "> " prompt and busy-waits for keyboard input.
    for _ in 0..8 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }

    let text = read_text_plane(&mut cpu);
    assert!(
        text.contains("TinyBASIC"),
        "help banner not rendered:\n{text}"
    );
    assert!(text.contains("FOR/NEXT"), "help body not rendered:\n{text}");
    assert!(text.contains("OK"), "REPL prompt not rendered:\n{text}");
}

/// Send one key press (down then up) through the PeC input channel.
fn type_key(sender: &UnboundedSender<HostInputEvent>, usage: u8) {
    sender
        .send(HostInputEvent::Key { usage, down: true })
        .unwrap();
    sender
        .send(HostInputEvent::Key { usage, down: false })
        .unwrap();
}

/// Send a shifted key (e.g. '^' as shift+'6').
fn type_shifted_key(sender: &UnboundedSender<HostInputEvent>, usage: u8) {
    const LEFT_SHIFT: u8 = 225;
    sender
        .send(HostInputEvent::Key {
            usage: LEFT_SHIFT,
            down: true,
        })
        .unwrap();
    type_key(sender, usage);
    sender
        .send(HostInputEvent::Key {
            usage: LEFT_SHIFT,
            down: false,
        })
        .unwrap();
}

#[test]
fn tinybasic_runs_program_from_keyboard_input() {
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);

    let mut cpu = Cpu::new(bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin"))
        .expect("tinybasic.bin missing; assemble tinybasic.sol first");
    cpu.load_program(0, &image);

    // Queue a program and RUN it before the CPU starts: the REPL
    // drains the PeC event FIFO across its two read_line calls.
    //   10 print 2^10   ->  1024
    //   run
    // HID usage: letters a..z are 4..29, digits 1..9,0 are 30..39,
    // space 44, Enter 40. '^' is shift+'6' (usage 35).
    let usages: &[u8] = &[
        30, 39, 44, // "10 "
        19, 21, 12, 17, 23, // "print"
        44, // " "
        31, // "2"
        35, // "6" shifted -> '^'
        30, 39, // "10"
        40, // Enter
        21, 24, 17, // "run"
        40, // Enter
    ];
    for &usage in usages {
        if usage == 35 {
            type_shifted_key(&input, usage);
        } else {
            type_key(&input, usage);
        }
    }

    // Boot, print the help banner, drain both input lines, and
    // execute the program. 2^10 = 1024 only appears if the
    // interpreter actually evaluated the expression.
    for _ in 0..16 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }

    let text = read_text_plane(&mut cpu);
    assert!(text.contains("TinyBASIC"), "help banner missing:\n{text}");
    assert!(
        text.contains("1024"),
        "program output 2^10=1024 not rendered:\n{text}"
    );
}

#[test]
fn tinybasic_error_reports_line_number() {
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);

    let mut cpu = Cpu::new(bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin"))
        .expect("tinybasic.bin missing; assemble tinybasic.sol first");
    cpu.load_program(0, &image);

    // Queue a program whose line 10 divides by zero, then RUN
    // it. HID usage: digits 1,0 are 30,39, space 44, '/' 56,
    // Enter 40; letters p,r,i,n,t are 19,21,12,17,23.
    //   10 print 1/0   ->  ? ERROR IN LINE 10
    //   run
    let usages: &[u8] = &[
        30, 39, 44, // "10 "
        19, 21, 12, 17, 23, // "print"
        44, // " "
        30, 56, 39, // "1/0"
        40, // Enter
        21, 24, 17, // "run"
        40, // Enter
    ];
    for &usage in usages {
        type_key(&input, usage);
    }

    for _ in 0..16 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }

    let text = read_text_plane(&mut cpu);
    assert!(
        text.contains("? ERROR IN LINE 10"),
        "error line number not rendered:\n{text}"
    );
}

#[test]
fn tinybasic_print_field_width() {
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);

    let mut cpu = Cpu::new(bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin"))
        .expect("tinybasic.bin missing; assemble tinybasic.sol first");
    cpu.load_program(0, &image);

    // Queue a program using the PRINT #N field-width
    // specifier (used by the Star Trek sample), then RUN
    // it. HID usage: digits 1-9,0 are 30-39, letters
    // a-z are 4-29, space 44, '-' 45, ',' 54, Enter
    // 40; '"' is shift+\" (52) and '#' is shift+3
    // (33). Each key is a (usage, shifted) pair so
    // '#' (shifted 33) and '4' (plain 33) differ.
    //   10 print "x-",#5,42," y-",7
    //     ->  "x-   42 y-7"  (42 right-justified in 5)
    let keys: &[(u8, bool)] = &[
        (30, false),
        (39, false),
        (44, false), // "10 "
        (19, false),
        (21, false),
        (12, false),
        (17, false),
        (23, false), // "print"
        (44, false), // " "
        (52, true),  // "
        (27, false),
        (45, false), // "x-"
        (52, true),  // "
        (54, false), // ","
        (32, true),  // #
        (34, false), // "5"
        (54, false), // ","
        (33, false),
        (31, false), // "42"
        (54, false), // ","
        (52, true),  // "
        (44, false),
        (28, false),
        (45, false), // " y-"
        (52, true),  // "
        (54, false), // ","
        (36, false), // "7"
        (40, false), // Enter
        (21, false),
        (24, false),
        (17, false), // "run"
        (40, false), // Enter
    ];
    for &(usage, shifted) in keys {
        if shifted {
            type_shifted_key(&input, usage);
        } else {
            type_key(&input, usage);
        }
    }

    for _ in 0..16 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }

    let text = read_text_plane(&mut cpu);
    assert!(
        text.contains("x-   42 y-7"),
        "padded field output not rendered:\n{text}"
    );
}

#[test]
fn tinybasic_halts_until_input_irq() {
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);

    let mut cpu = Cpu::new(bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin"))
        .expect("tinybasic.bin missing; assemble tinybasic.sol first");
    cpu.load_program(0, &image);

    // Boot until the REPL reaches its prompt. tb_read_line must
    // then stop the CPU (HALT) instead of busy-polling the PeC
    // event FIFO.
    let mut halted = false;
    for _ in 0..8 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
        if !cpu.is_running() {
            halted = true;
            break;
        }
    }
    assert!(halted, "CPU did not halt while waiting for keyboard input");
    let text = read_text_plane(&mut cpu);
    assert!(text.contains("TinyBASIC"), "help banner missing:\n{text}");
    assert!(text.contains("OK"), "REPL prompt missing:\n{text}");

    // While halted with no events, HALT consumes clock cycles but
    // no instructions retire.
    let idle = cpu.instructions_executed();
    for _ in 0..4 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    assert_eq!(
        cpu.instructions_executed(),
        idle,
        "CPU retired instructions while waiting for input"
    );

    // A PeC key event must wake the CPU through IRQ3. The PeC
    // input thread polls its channel with a millisecond sleep, so
    // keep running frames until the wake lands.
    type_key(&input, 40);
    let deadline = Instant::now() + Duration::from_secs(2);
    while cpu.instructions_executed() == idle {
        assert!(
            Instant::now() < deadline,
            "PeC IRQ did not wake the halted CPU"
        );
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
}
