//! Micro-tests for the strek-tb.bas failure shape (temporary).
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu::{CYCLES_PER_FRAME, Cpu};
use cpt32::devices::pec::pec::{HostInputEvent, connect_pec_input};
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::vdp::vdp::VDP_VRAM_BASE;
use tokio::sync::mpsc::UnboundedSender;

fn read_text_plane(cpu: &mut Cpu) -> String {
    let mut text = String::new();
    for row in 0..30 {
        for cell in 0..80 {
            let byte = cpu.read_mem_u8(VDP_VRAM_BASE + (row * 80 + cell) as u32);
            text.push(if byte.is_ascii_graphic() || byte == b' ' {
                byte as char
            } else {
                ' '
            });
        }
        text.push('\n');
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

fn type_shifted(sender: &UnboundedSender<HostInputEvent>, usage: u8) {
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

// type an ASCII string via HID (supports what these micro-tests need)
fn type_str(sender: &UnboundedSender<HostInputEvent>, s: &str) {
    for c in s.chars() {
        match c {
            'a'..='z' => type_key(sender, (c as u8) - b'a' + 4),
            'A'..='Z' => type_shifted(sender, (c as u8) - b'A' + 4),
            ' ' => type_key(sender, 44),
            '\n' => type_key(sender, 40),
            '0'..='9' => {
                let u = if c == '0' { 39 } else { (c as u8) - b'1' + 30 };
                type_key(sender, u);
            }
            '=' => type_key(sender, 46),
            '+' => type_shifted(sender, 46),
            '-' => type_key(sender, 45),
            ',' => type_key(sender, 54),
            ':' => type_shifted(sender, 51),
            '@' => type_shifted(sender, 31),
            '*' => type_shifted(sender, 37),
            '(' => type_shifted(sender, 38),
            ')' => type_shifted(sender, 39),
            '<' => type_shifted(sender, 54),
            '"' => type_shifted(sender, 52),
            _ => panic!("unsupported char {c}"),
        }
    }
}

fn run_basic(prog_lines: &[&str]) -> String {
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    let mut cpu = Cpu::new(bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin"))
        .expect("tinybasic.bin missing");
    cpu.load_program(0, &image);
    // queue one line at a time with frames in between: the PeC event
    // FIFO holds 256 entries, so bulk-queueing long lines would drop keys
    for line in prog_lines {
        type_str(&input, line);
        type_key(&input, 40);
        for _ in 0..30 {
            vdp.borrow_mut().tick();
            cpu.run(CYCLES_PER_FRAME as usize);
        }
    }
    type_str(&input, "run");
    type_key(&input, 40);
    for _ in 0..60 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let text = read_text_plane(&mut cpu);
    // program output follows the "> run" echo; input echoes above it
    // would otherwise satisfy naive contains() checks
    println!(
        "cpu running={} instrs={} pec_status={:#x}",
        cpu.is_running(),
        cpu.instructions_executed(),
        u32::from_be_bytes([
            cpu.read_mem_u8(0x80040034),
            cpu.read_mem_u8(0x80040035),
            cpu.read_mem_u8(0x80040036),
            cpu.read_mem_u8(0x80040037),
        ])
    );
    match text.find("> run") {
        Some(i) => text[i..].to_string(),
        None => text,
    }
}

#[test]
fn reroll_shape_no_reroll() {
    let text = run_basic(&[
        "10 b = 5",
        "20 k = 9",
        "30 if (b < 2) + (k < 4) goto 60",
        "40 print \"STAY\"",
        "50 end",
        "60 print \"GO60\"",
    ]);
    println!("=== REROLL-FALSE ===\n{text}==================");
    assert!(text.contains("STAY"), "expected NO REROLL:\n{text}");
}

#[test]
fn reroll_shape_reroll() {
    let text = run_basic(&[
        "10 b = 1",
        "20 k = 9",
        "30 if (b < 2) + (k < 4) goto 60",
        "40 print \"STAY\"",
        "50 end",
        "60 print \"GO60\"",
    ]);
    println!("=== REROLL-TRUE ===\n{text}==================");
    assert!(text.contains("GO60"), "expected REROLL:\n{text}");
    assert!(
        !text.contains("STAY"),
        "should not print NO REROLL:\n{text}"
    );
}

#[test]
fn debug_long_line_storage() {
    // type ONE long line, then LIST to see exactly what got stored
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    let mut cpu = Cpu::new(bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin"))
        .expect("tinybasic.bin missing");
    cpu.load_program(0, &image);
    for _ in 0..8 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    type_str(
        &input,
        "15 m = rnd(y): m = abs((m < 209) + (m < 99) + (m < 49) + (m < 24) + (m < 9) + (m < 2)): k = k + m",
    );
    type_key(&input, 40);
    for _ in 0..10 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    type_str(&input, "list");
    type_key(&input, 40);
    for _ in 0..10 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let text = read_text_plane(&mut cpu);
    println!("=== LONG-LINE ===\n{text}==================");
    println!(
        "cpu running={} instrs={} pec_status={:#x}",
        cpu.is_running(),
        cpu.instructions_executed(),
        u32::from_be_bytes([
            cpu.read_mem_u8(0x80040034),
            cpu.read_mem_u8(0x80040035),
            cpu.read_mem_u8(0x80040036),
            cpu.read_mem_u8(0x80040037),
        ])
    );
}

#[test]
fn setup_loop_exact_shape() {
    // exact copy of strek-tb.bas lines 15/20/25 (renumbered), with DONE marker
    let text = run_basic(&[
        "5 y = 2999",
        "10 k = 0: b = 0: d = 30: for i = 0 to 63: j = abs(rnd(99) < 5): b = b + j",
        "15 m = rnd(y): m = abs((m < 209) + (m < 99) + (m < 49) + (m < 24) + (m < 9) + (m < 2)): k = k + m",
        "20 @(i) = -100 * m - 10 * j - rnd(8): next i: if (b < 2) + (k < 4) goto 10",
        "25 print b: print k: print \"DONE\"",
    ]);
    println!("=== SETUP-EXACT ===\n{text}==================");
    assert!(text.contains("DONE"), "setup loop did not finish:\n{text}");
}

#[test]
fn for_accumulates() {
    let text = run_basic(&[
        "10 k = 0",
        "20 for i = 0 to 63",
        "30 k = k + 1",
        "40 next i",
        "50 print k",
    ]);
    println!("=== FOR-ACC ===\n{text}==================");
    assert!(text.contains("64"), "expected 64:\n{text}");
}
