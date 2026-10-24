use cpt32::bus::Bus;
use cpt32::devices::vdp::pcg::{
    PCG_FONT_8X16_ADDR, PCG_FONT_8X16_SIZE, PCG_FONT_BANK_SIZE, PCG_FONT_BANK0_ADDR,
    PCG_FONT_BANK1_ADDR, PcgRenderer,
};
use cpt32::devices::vdp::vdp::{VDP_VRAM_BASE, connect_vdp};

#[test]
fn bundled_unscii_keeps_cp437_bank_layout_and_dedicated_8x16_storage() {
    let font = include_bytes!("../assets/UNSCII.F08");
    assert_eq!(font.len(), PCG_FONT_BANK_SIZE);
    assert_eq!(&font[..8], &[0; 8]); // NUL remains a blank glyph.
    assert_eq!(
        &font[65 * 8..66 * 8],
        &[0x18, 0x3c, 0x66, 0x66, 0x7e, 0x66, 0x66, 0]
    );
    assert_eq!(&font[0xdb * 8..0xdc * 8], &[0xff; 8]); // CP437 full block.

    let mut bus = Bus::new();
    let vdp = connect_vdp(&mut bus);
    bus.write_u8(VDP_VRAM_BASE + PCG_FONT_8X16_ADDR as u32, 0xa5);
    vdp.borrow()
        .load_pcg_font_from_file(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/UNSCII.F08"))
        .unwrap();
    for bank in [PCG_FONT_BANK0_ADDR, PCG_FONT_BANK1_ADDR] {
        for (offset, expected) in font.iter().enumerate() {
            assert_eq!(
                bus.read_u8(VDP_VRAM_BASE + (bank + offset) as u32),
                *expected
            );
        }
    }
    assert_eq!(bus.read_u8(VDP_VRAM_BASE + PCG_FONT_8X16_ADDR as u32), 0xa5);
}

#[test]
fn raw_8x16_font_loads_renders_and_rejects_invalid_size_without_mutation() {
    let mut bus = Bus::new();
    let vdp = connect_vdp(&mut bus);
    vdp.borrow()
        .load_pcg_font_from_file(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/UNSCII.F08"))
        .unwrap();
    let mut font = vec![0; PCG_FONT_8X16_SIZE];
    font[65 * 16] = 0x80;
    font[65 * 16 + 15] = 0x01;
    let path = std::env::temp_dir().join(format!("cpt32-font-8x16-{}.f16", std::process::id()));
    std::fs::write(&path, &font).unwrap();
    vdp.borrow().load_pcg_font_8x16_from_file(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    for (offset, expected) in font.iter().enumerate() {
        assert_eq!(
            bus.read_u8(VDP_VRAM_BASE + (PCG_FONT_8X16_ADDR + offset) as u32),
            *expected
        );
    }
    for bank in [PCG_FONT_BANK0_ADDR, PCG_FONT_BANK1_ADDR] {
        for (offset, expected) in include_bytes!("../assets/UNSCII.F08").iter().enumerate() {
            assert_eq!(
                bus.read_u8(VDP_VRAM_BASE + (bank + offset) as u32),
                *expected
            );
        }
    }
    // Select mode 3 and draw A in white on black through the real framebuffer.
    bus.write_u8(VDP_VRAM_BASE, 65);
    bus.write_u8(VDP_VRAM_BASE + 0x1000, 63);
    bus.write_u8(cpt32::devices::vdp::vdp::VDP_REG_BASE + 0x01, 1); // DisplayMode::PCG
    bus.write_u8(cpt32::devices::vdp::vdp::VDP_REG_BASE + 0xF003, 3);
    let display = vdp.borrow();
    let fb = display.framebuffer();
    assert_eq!(fb.dimensions(), (640, 480));
    assert_eq!(fb.get_pixel(0, 0), (255, 255, 255));
    assert_eq!(fb.get_pixel(7, 15), (255, 255, 255));
    assert_eq!(fb.get_pixel(1, 0), (0, 0, 0));

    let storage = std::rc::Rc::new(std::cell::RefCell::new(vec![0xa5; 0x22000]));
    let renderer = PcgRenderer::new(storage.clone());
    for size in [0, 2048, 4095, 4097, 8192] {
        let error = renderer.load_font_8x16_bytes(&vec![0; size]).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(storage.borrow().iter().all(|&byte| byte == 0xa5));
    }
}
