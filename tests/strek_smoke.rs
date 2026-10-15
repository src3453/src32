//! THROWAWAY diagnostics for the strek-tb.bas hang. Delete after use.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu::{CYCLES_PER_FRAME, Cpu};
use cpt32::devices::pec::idc::idc::{Idc, connect_idc};
use cpt32::devices::pec::pec::{HostInputEvent, connect_pec_input};
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::vdp::vdp::{VDP_VRAM_BASE, Vdp};
use std::cell::RefCell;
use std::rc::Rc;
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

fn setup(name: &str, bas: &[u8]) -> (Cpu, Rc<RefCell<Vdp>>, UnboundedSender<HostInputEvent>) {
    let root = std::env::temp_dir().join(format!("tb-probe-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("disk0.bas"), bas).unwrap();
    let config = root.join("config.yaml");
    std::fs::write(
        &config,
        "disk:\n  - slot: 0\n    type: fdd\n    image: disk0.bas\n    read_only: true\n    transfer_rate_bytes_per_second: 48000000\n    access_latency_ms: 0\n    incomplete_sector_policy: pad\n",
    )
    .unwrap();
    let drives = Idc::load_drives(&config).unwrap();
    let mut bus = Bus::new();
    let input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    connect_idc(&mut bus, drives);
    let mut cpu = Cpu::new(bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin"))
        .expect("tinybasic.bin missing; assemble tinybasic.sol first");
    cpu.load_program(0, &image);
    (cpu, vdp, input)
}

// Probe 1: does rnd() vary on the real CPU? Infinite loop printing
// rnd(99) values; the last text-plane rows reveal the sequence.
#[test]
fn probe_rnd() {
    let (mut cpu, vdp, input) = setup("rnd", b"10 print rnd(99)\n20 goto 10\n");
    // "load" Enter "run" Enter
    let usages: &[u8] = &[15, 18, 4, 7, 40, 21, 24, 17, 40];
    for &usage in usages {
        type_key(&input, usage);
    }
    for _ in 0..30 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let text = read_text_plane(&mut cpu);
    println!("=== PROBE RND ===\n{text}==================");
    let _ = std::fs::remove_dir_all(
        std::env::temp_dir().join(format!("tb-probe-{}-rnd", std::process::id())),
    );
}

// Probe 2: TRON trace of the Star Trek sample to find where it is stuck.
#[test]
fn probe_strek_tron() {
    let bas = std::fs::read("/Users/src3453/remote/Tiny-Basic/samples/strek-tb.bas").unwrap();
    let (mut cpu, vdp, input) = setup("strek", &bas);
    // "tron" Enter "load" Enter "run" Enter "n" Enter "n" Enter
    let usages: &[u8] = &[
        23, 21, 18, 17, 40, // "tron" Enter
        15, 18, 4, 7, 40, // "load" Enter
        21, 24, 17, 40, // "run" Enter
        17, 40, // "n" Enter
        17, 40, // "n" Enter
    ];
    for &usage in usages {
        type_key(&input, usage);
    }
    for _ in 0..120 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let text = read_text_plane(&mut cpu);
    println!("=== PROBE STREK TRON ===\n{text}==================");
    let _ = std::fs::remove_dir_all(
        std::env::temp_dir().join(format!("tb-probe-{}-strek", std::process::id())),
    );
}

// Probe 3: replicate the strek setup loop exactly, then print the
// accumulated b/k, the re-roll condition, and whether the if-goto fires.
#[test]
fn probe_setup() {
    let program = b"5 y = 2999\n\
        10 k = 0: b = 0: d = 30: for i = 0 to 63: j = abs(rnd(99) < 5): b = b + j\n\
        20 m = rnd(y): m = abs((m < 209) + (m < 99) + (m < 49) + (m < 24) + (m < 9) + (m < 2)): k = k + m\n\
        25 print i: print j: print m: print b: print k: next i: if (b < 2) + (k < 4) goto 15\n\
        30 print b\n35 print k\n40 print (b < 2) + (k < 4)\n\
        45 if (b < 2) + (k < 4) goto 60\n50 print \"NO REROLL\"\n55 goto 70\n\
        60 print \"REROLL\"\n70 end\n";
    let (mut cpu, vdp, input) = setup("setup", program);
    let usages: &[u8] = &[15, 18, 4, 7, 40, 21, 24, 17, 40];
    for &usage in usages {
        type_key(&input, usage);
    }
    for _ in 0..30 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let text = read_text_plane(&mut cpu);
    println!("=== PROBE SETUP ===\n{text}==================");
    let _ = std::fs::remove_dir_all(
        std::env::temp_dir().join(format!("tb-probe-{}-setup", std::process::id())),
    );
}
