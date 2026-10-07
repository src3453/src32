// Graphic Plane VRAM Layout (From base address):
// In graphic mode each bitmap is tightly packed: palette at 0x00000 (8bpp),
// RGB555 at 0x100000 (16bpp big-endian, bit 15 reserved), RGB888 at 0x200000
// (24bpp RGB). 0x12C00 - 0x12EFF: 256 RGB888 CLUT entries (768 bytes).

use std::cell::RefCell;
use std::rc::Rc;

pub const GP_WIDTH: usize = 320;
pub const GP_HEIGHT: usize = 240;
pub const PALETTE_BITMAP_START_ADDR: usize = 0x00000;
pub const RGB555_BITMAP_START_ADDR: usize = 0x100000;
pub const RGB888_BITMAP_START_ADDR: usize = 0x200000;
pub const CLUT_SIZE: usize = 256;
pub const CLUT_ENTRY_SIZE: usize = 3; // RGB888
pub const CLUT_TOTAL_SIZE: usize = CLUT_SIZE * CLUT_ENTRY_SIZE; // 768 bytes
pub const CLUT_START_ADDR: usize = 0x12C00;

use crate::devices::vdp::clut::CLUT_DEFAULT;

pub struct Gp0 {
    vram: Rc<RefCell<Vec<u8>>>,
}

impl Gp0 {
    pub fn new(vram: Rc<RefCell<Vec<u8>>>) -> Self {
        Self { vram }
    }

    pub fn init_clut(&self) {
        let mut vram = self.vram.borrow_mut();
        let clut = &mut vram[CLUT_START_ADDR..CLUT_START_ADDR + CLUT_TOTAL_SIZE];
        clut[..CLUT_DEFAULT.len()].copy_from_slice(&CLUT_DEFAULT);
        // Fill the remaining palette entries with a deterministic RGB332 cube.
        for index in 64..CLUT_SIZE {
            let r = ((index >> 5) & 0x07) as u8;
            let g = ((index >> 2) & 0x07) as u8;
            let b = (index & 0x03) as u8;
            let offset = index * CLUT_ENTRY_SIZE;
            clut[offset] = ((r as u16 * 255 + 3) / 7) as u8;
            clut[offset + 1] = ((g as u16 * 255 + 3) / 7) as u8;
            clut[offset + 2] = ((b as u16 * 255 + 1) / 3) as u8;
        }
    }

    pub fn get_pixel(
        &self,
        x: usize,
        y: usize,
        color_mode: super::reg::BitmapColorMode,
    ) -> (u8, u8, u8) {
        if x >= GP_WIDTH || y >= GP_HEIGHT {
            return (0, 0, 0);
        }

        let vram = self.vram.borrow();
        let index = y * GP_WIDTH + x;
        match color_mode {
            super::reg::BitmapColorMode::Palette256 => {
                let pixel = vram[PALETTE_BITMAP_START_ADDR + index];
                let clut_index = CLUT_START_ADDR + pixel as usize * CLUT_ENTRY_SIZE;
                (vram[clut_index], vram[clut_index + 1], vram[clut_index + 2])
            }
            super::reg::BitmapColorMode::Rgb555 => {
                let offset = RGB555_BITMAP_START_ADDR + index * 2;
                let value = u16::from_be_bytes([vram[offset], vram[offset + 1]]);
                let r = ((value >> 10) & 0x1f) as u8;
                let g = ((value >> 5) & 0x1f) as u8;
                let b = (value & 0x1f) as u8;
                (
                    (r << 3) | (r >> 2),
                    (g << 3) | (g >> 2),
                    (b << 3) | (b >> 2),
                )
            }
            super::reg::BitmapColorMode::Rgb888 => {
                let offset = RGB888_BITMAP_START_ADDR + index * 3;
                (vram[offset], vram[offset + 1], vram[offset + 2])
            }
        }
    }
}
