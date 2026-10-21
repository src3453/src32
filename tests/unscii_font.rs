use cpt32::bus::Bus;
use cpt32::devices::vdp::pcg::{
    PCG_FONT_8X16_ADDR, PCG_FONT_BANK_SIZE, PCG_FONT_BANK0_ADDR, PCG_FONT_BANK1_ADDR,
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
