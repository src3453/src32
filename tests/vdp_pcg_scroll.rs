use cpt32::bus::Bus;
use cpt32::devices::vdp::pcg::{PcgRenderer, PcgScreenMode};
use cpt32::devices::vdp::vdp::{VDP_REG_BASE, VDP_VRAM_BASE, connect_vdp};
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn pcg_cursor_exchanges_palette_colors_in_all_modes() {
    let vram = Rc::new(RefCell::new(vec![0; 0x40000]));
    let renderer = PcgRenderer::new(vram.clone());
    let fg = (17, 83, 149);
    let bg = (29, 101, 211);
    {
        let mut ram = vram.borrow_mut();
        ram[0x1000] = 1;
        ram[0x2000] = 2;
        ram[0x12c03..0x12c09].copy_from_slice(&[17, 83, 149, 29, 101, 211]);
        ram[0x20000..0x20008].fill(0xAA);
        ram[0x21000..0x21010].fill(0xAA);
    }
    for mode in [
        PcgScreenMode::Columns40,
        PcgScreenMode::Columns80,
        PcgScreenMode::Unifont16x16,
        PcgScreenMode::Font8x16,
    ] {
        // U+3042 supplies both set and clear bits in the ROM font.
        vram.borrow_mut()[0] = if mode == PcgScreenMode::Unifont16x16 {
            0x30
        } else {
            0
        };
        vram.borrow_mut()[1] = 0x42;
        for swap in [false, true] {
            let mut seen = [false; 2];
            for y in 0..mode.cell_height() {
                for x in 0..mode.cell_width() {
                    let pixel = |enabled, lines, period, tick, cx| {
                        renderer
                            .get_pixel(x, y, mode, 0, swap, cx, 0, enabled, lines, period, tick, 0)
                    };
                    let normal = pixel(false, 0, 0, 0, 0);
                    seen[usize::from(normal == fg)] = true;
                    let exchanged = if normal == fg { bg } else { fg };
                    assert_eq!(pixel(true, 0, 0, 0, 0), exchanged);
                    assert_eq!(pixel(true, 0xFF, 0, 0, 0), normal);
                    assert_eq!(pixel(true, 0, 2, 2, 0), normal);
                    assert_eq!(pixel(true, 0, 2, 4, 0), exchanged);
                    assert_eq!(pixel(true, 0, 0, 0, 1), normal);
                    assert_eq!(pixel(true, 0, 0, 0, 255), normal);
                    let masked = if y < mode.cell_height() / 8 {
                        normal
                    } else {
                        exchanged
                    };
                    assert_eq!(pixel(true, 0x80, 0, 0, 0), masked);
                }
            }
            assert_eq!(seen, [true, true]);
        }
    }
    // Equal cell colors must remain equal, even while the cursor is visible.
    vram.borrow_mut()[0x2000] = 1;
    assert_eq!(
        renderer.get_pixel(
            0,
            0,
            PcgScreenMode::Columns40,
            0,
            false,
            0,
            0,
            true,
            0,
            0,
            0,
            0
        ),
        fg
    );
}

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

    // Jiskan16 あ has a set bit at (6, 1); scroll moves it to (7, 2).
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
        assert_eq!(framebuffer.get_pixel(7, 2), (255, 0, 0));
    }

    bus.write_u8(VDP_REG_BASE + 0x01, 0); // Graphics mode
    bus.write_u8(VDP_REG_BASE + 0xF00B, 1); // Enable the PCG overlay
    assert_eq!(vdp.borrow().framebuffer().get_pixel(7, 2), (255, 0, 0));
    // The cursor stays at tilemap cell (0, 0) through scroll and GP overlay.
    bus.write_u8(VDP_REG_BASE + 0xF007, 1);
    assert_eq!(vdp.borrow().framebuffer().get_pixel(7, 2), (0, 0, 200));
    bus.write_u8(VDP_REG_BASE + 0xF002, 1);
    assert_eq!(vdp.borrow().framebuffer().get_pixel(7, 2), (255, 0, 0));
}
