//! Temporary verification: load strek-tb.bas from disk and run it.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu::{CYCLES_PER_FRAME, Cpu};
use cpt32::devices::pec::idc::idc::{Idc, connect_idc};
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

#[test]
fn strek_loads_and_starts_game() {
    let bas = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../Tiny-Basic/samples/strek-tb.bas"
    ))
    .expect("strek-tb.bas missing");
    let root = std::env::temp_dir().join(format!("tb-strek-verify-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("disk0.bas"), &bas).unwrap();
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

    // "load" Enter "run" Enter "n" Enter "n" Enter
    // HID: l=15 o=18 a=4 d=7, r=21 u=24 n=17, Enter=40
    let usages: &[u8] = &[
        15, 18, 4, 7, 40, // load
        21, 24, 17, 40, // run
        17, 40, // n (no instructions)
        17, 40, // n (not difficult)
    ];
    for &u in usages {
        type_key(&input, u);
    }
    for _ in 0..400 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let text = read_text_plane(&mut cpu);
    println!("=== STREK TEXT ===\n{text}\n==================");
    assert!(text.contains("LOADED"), "LOAD failed:\n{text}");
    assert!(!text.contains("ERROR"), "BASIC error:\n{text}");
    assert!(
        text.contains("Klingons")
            || text.contains("klingon")
            || text.contains("Enterprise")
            || text.contains("Captain"),
        "game did not start:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
