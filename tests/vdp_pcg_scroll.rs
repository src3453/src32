use cpt32::bus::Bus;
use cpt32::devices::vdp::vdp::{connect_vdp, VDP_REG_BASE, VDP_VRAM_BASE};

#[test]
fn pcg_scroll_wraps_both_axes_and_exposes_sixteen_bit_mmio_registers() {
    let mut bus = Bus::new();
    let vdp = connect_vdp(&mut bus);

    bus.write_u8(VDP_REG_BASE + 0x01, 1); // PCG display mode
    bus.write_u8(VDP_REG_BASE + 0xF003, 2); // 20x15 cells of 16x16 glyphs
    bus.write_u8(VDP_VRAM_BASE, 0x30); // U+3042 in UTF-16BE
    bus.write_u8(VDP_VRAM_BASE + 1, 0x42);
    bus.write_u8(VDP_VRAM_BASE + 0x01000, 1); // foreground CLUT index
    bus.write_u8(VDP_VRAM_BASE + 0x02000, 2); // background CLUT index
    bus.write_u8(VDP_VRAM_BASE + 0x12C03, 255);
    bus.write_u8(VDP_VRAM_BASE + 0x12C04, 0);
    bus.write_u8(VDP_VRAM_BASE + 0x12C05, 0);
    bus.write_u8(VDP_VRAM_BASE + 0x12C06, 0);
    bus.write_u8(VDP_VRAM_BASE + 0x12C07, 0);
    bus.write_u8(VDP_VRAM_BASE + 0x12C08, 200);

    // Offsets near the right and bottom edges force both coordinates to wrap.
    for (offset, value) in [(0xF00C, 0x3F), (0xF00D, 0x01), (0xF00E, 0xEF), (0xF00F, 0)] {
        bus.write_u8(VDP_REG_BASE + offset, value);
    }

    assert_eq!(bus.read_u8(VDP_REG_BASE + 0xF00C), 0x3F);
    assert_eq!(bus.read_u8(VDP_REG_BASE + 0xF00D), 0x01);
    assert_eq!(bus.read_u8(VDP_REG_BASE + 0xF00E), 0xEF);
    assert_eq!(bus.read_u8(VDP_REG_BASE + 0xF00F), 0x00);

    {
        let vdp = vdp.borrow();
        let framebuffer = vdp.framebuffer();
        assert_eq!(framebuffer.get_pixel(6, 2), (255, 0, 0));
    }

    bus.write_u8(VDP_REG_BASE + 0x01, 0); // Graphics mode
    bus.write_u8(VDP_REG_BASE + 0xF00B, 1); // Enable the PCG overlay
    let vdp = vdp.borrow();
    let framebuffer = vdp.framebuffer();
    assert_eq!(framebuffer.get_pixel(6, 2), (255, 0, 0));
}
