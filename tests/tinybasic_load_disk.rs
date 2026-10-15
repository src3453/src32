//! Regression test for TinyBASIC LOAD from a small disk
//! image.
//!
//! `LOAD` used to default to 128 sectors (64 KiB), which
//! exceeded the capacity of any small image and made the
//! IDC reject the LBA range (ERROR_CODE=3) with
//! "DISK READ ERROR". LOAD now identifies the drive and
//! clamps the sector count to the image size. The image
//! here is also deliberately not a multiple of 512 bytes,
//! so the `incomplete_sector_policy: pad` config path is
//! exercised too.
use cpt32::bus::{connect_devices_with_vdp, Bus};
use cpt32::cpu::{Cpu, CYCLES_PER_FRAME};
use cpt32::devices::pec::idc::idc::{connect_idc, Idc};
use cpt32::devices::pec::pec::{connect_pec_input, HostInputEvent};
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::vdp::vdp::VDP_VRAM_BASE;
use tokio::sync::mpsc::UnboundedSender;

fn read_text_plane(cpu: &mut Cpu) -> String {
    let mut text = String::new();
    for cell in 0..40 * 30 {
        let byte = cpu.read_mem_u8(VDP_VRAM_BASE + cell as u32);
        text.push(if byte.is_ascii_graphic() || byte == b' ' {
            byte as char
        } else {
            ' '
        });
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
fn tinybasic_load_from_small_nonaligned_image() {
    let root = std::env::temp_dir().join(format!("tb-load-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    // Deliberately not a multiple of 512 bytes.
    let bas = b"10 PRINT \"HELLO\"\n20 GOTO 10\n";
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
    let image =
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin"))
            .expect("tinybasic.bin missing; assemble tinybasic.sol first");
    cpu.load_program(0, &image);

    // HID: letters a..z are 4..29, Enter is 40.
    //   "load"  Enter   "list"  Enter
    let usages: &[u8] = &[15, 18, 4, 7, 40, 15, 12, 22, 23, 40];
    for &usage in usages {
        type_key(&input, usage);
    }

    for _ in 0..16 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }

    let text = read_text_plane(&mut cpu);
    assert!(text.contains("LOADED"), "LOAD did not succeed:\n{text}");
    assert!(
        !text.contains("DISK READ ERROR"),
        "LOAD reported a disk error:\n{text}"
    );
    assert!(
        text.contains("HELLO"),
        "loaded program not listed:\n{text}"
    );

    std::fs::remove_dir_all(&root).unwrap();
}
