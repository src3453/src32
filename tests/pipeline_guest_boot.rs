//! Pipeline-default guard: the real TinyBASIC guest (sol-compiled,
//! trampolines and all) boots and renders its banner on `AnyCpu::Pipelined`.
//! Requires tinybasic.bin (same as tests/tinybasic.rs): assemble first.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu::CYCLES_PER_FRAME;
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::pec::pec::connect_pec_input;
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::vdp::vdp::VDP_VRAM_BASE;

#[test]
fn pipeline_boots_tinybasic_guest() {
    let mut bus = Bus::new();
    let _input = connect_pec_input(&mut bus);
    connect_rng(&mut bus);
    let vdp = connect_devices_with_vdp(&mut bus);
    let mut cpu = AnyCpu::new(CpuKind::Pipelined, bus);
    let image = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tinybasic.bin"))
        .expect("tinybasic.bin missing");
    cpu.load_program(0, &image);
    for _ in 0..8 {
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    let mut text = String::new();
    for cell in 0..40 * 30 {
        let byte = cpu.read_mem_u8(VDP_VRAM_BASE + cell as u32);
        if byte.is_ascii_graphic() || byte == b' ' {
            text.push(byte as char);
        } else {
            text.push(' ');
        }
    }
    assert!(text.contains("TinyBASIC"), "no banner:\n{text}");
    assert!(text.contains("FOR/NEXT"), "no help body:\n{text}");
    assert!(text.contains("OK"), "no prompt:\n{text}");
}
