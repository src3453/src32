use cpt32::bus::Bus;
use cpt32::devices::vdp::vdp::{VDP_REG_BASE, VDP_VRAM_BASE, connect_vdp};

const RED: (u8, u8, u8) = (255, 0, 0);
const BLUE: (u8, u8, u8) = (0, 0, 200);

#[test]
fn mode3_wide_cells_read_the_font_at_the_register_offset() {
    let mut bus = Bus::new();
    let vdp = connect_vdp(&mut bus);
    bus.write_u8(VDP_REG_BASE + 0x01, 1); // PCG display mode
    bus.write_u8(VDP_REG_BASE + 0xF003, 3); // 80x30 cells of 8x16 glyphs
    for (i, byte) in [0x00, 0x00, 0x30, 0x00].into_iter().enumerate() {
        bus.write_u8(VDP_REG_BASE + 0xF010 + i as u32, byte);
    }
    assert_eq!(bus.read_u8(VDP_REG_BASE + 0xF012), 0x30);

    for (offset, value) in [
        (0x12C03, 255),
        (0x12C04, 0),
        (0x12C05, 0),
        (0x12C06, 0),
        (0x12C07, 0),
        (0x12C08, 200),
    ] {
        bus.write_u8(VDP_VRAM_BASE + offset, value);
    }
    // U+3042 in cells 0 (MSB) and 1 (LSB); cell 2 is an orphan MSB drawn from the 1-byte font.
    bus.write_u8(VDP_VRAM_BASE, 0x30);
    bus.write_u8(VDP_VRAM_BASE + 1, 0x42);
    bus.write_u8(VDP_VRAM_BASE + 2, 1);
    for cell in 0..3 {
        bus.write_u8(VDP_VRAM_BASE + 0x2000 + cell, 2);
    }
    bus.write_u8(VDP_VRAM_BASE + 0x1000, 1 | 0x40);
    bus.write_u8(VDP_VRAM_BASE + 0x1001, 1 | 0x80);
    bus.write_u8(VDP_VRAM_BASE + 0x1002, 1 | 0x40);
    let glyph = 0x300000 + 0x3042 * 32;
    bus.write_u8(VDP_VRAM_BASE + glyph, 0x80); // row 0, left half, leftmost pixel
    bus.write_u8(VDP_VRAM_BASE + glyph + 1, 0x01); // row 0, right half, rightmost pixel
    bus.write_u8(VDP_VRAM_BASE + 0x21000 + 16, 0xFF); // 1-byte glyph 1, row 0

    {
        let fb = vdp.borrow();
        let fb = fb.framebuffer();
        assert_eq!(fb.get_pixel(0, 0), RED);
        assert_eq!(fb.get_pixel(1, 0), BLUE);
        assert_eq!(fb.get_pixel(8, 0), BLUE);
        assert_eq!(fb.get_pixel(15, 0), RED);
        assert_eq!(fb.get_pixel(16, 0), RED);
    }

    // Moving the base past the end of VRAM leaves wide cells blank.
    for (i, byte) in [0xFF, 0xFF, 0xFF, 0x7F].into_iter().enumerate() {
        bus.write_u8(VDP_REG_BASE + 0xF010 + i as u32, byte);
    }
    assert_eq!(vdp.borrow().framebuffer().get_pixel(0, 0), BLUE);
}
