// Graphic Plane VRAM Layout (From base address):
// In graphic mode:
// 0x00000 - 0x12BFF: 320x240@8bpp (MSB 2bits are reserved; 76,800 bytes) image data
// 0x12C00 - 0x12CBF: 64 CLUT (Color Look-Up Table) entries (RGB888; 192 bytes)

use std::cell::RefCell;
use std::rc::Rc;

pub const GP_WIDTH: usize = 320;
pub const GP_HEIGHT: usize = 240;
pub const CLUT_SIZE: usize = 64;
pub const CLUT_ENTRY_SIZE: usize = 3; // RGB888
pub const CLUT_TOTAL_SIZE: usize = CLUT_SIZE * CLUT_ENTRY_SIZE; // 192 bytes
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
        self.vram.borrow_mut()[CLUT_START_ADDR..CLUT_START_ADDR + CLUT_TOTAL_SIZE]
            .copy_from_slice(&CLUT_DEFAULT);
    }

    pub fn get_pixel(&self, x: usize, y: usize) -> (u8, u8, u8) {
        if x >= GP_WIDTH || y >= GP_HEIGHT {
            return (0, 0, 0);
        }

        let vram = self.vram.borrow();
        let index = y * GP_WIDTH + x;
        let pixel = vram[index] & 0x3F;
        let clut_index = CLUT_START_ADDR + (pixel as usize) * CLUT_ENTRY_SIZE;
        (vram[clut_index], vram[clut_index + 1], vram[clut_index + 2])
    }
}
