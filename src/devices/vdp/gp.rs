// Graphic Plane VRAM Layout (From base address):
// In graphic mode:
// 0x00000 - 0x12BFF: 320x240@8bpp (MSB 2bits are reserved; 76,800 bytes) image data
// 0x12C00 - 0x12CBF: 64 CLUT (Color Look-Up Table) entries (RGB888; 192 bytes)

pub const CLUT_SIZE: usize = 64;
pub const CLUT_ENTRY_SIZE: usize = 3; // RGB888
pub const CLUT_TOTAL_SIZE: usize = CLUT_SIZE * CLUT_ENTRY_SIZE; // 192 bytes
pub const CLUT_START_ADDR: usize = 0x12C00;

use crate::devices::vdp::clut::CLUT_DEFAULT;

pub struct Gp0 {
    pub vram: Vec<u8>,
}

impl Gp0 {
    pub fn new(size: usize) -> Self {
        Self {
            vram: vec![0; size],
        }
    }

    pub fn init_clut(&mut self) {
        self.vram[CLUT_START_ADDR..CLUT_START_ADDR + CLUT_TOTAL_SIZE].copy_from_slice(&CLUT_DEFAULT);
    }

    pub fn get_vram(&self) -> &[u8] {
        &self.vram
    }

    pub fn get_pixel(&self, x: usize, y: usize) -> (u8, u8, u8) { // convert 6bpp color to internal RGB888 for compositor
        let index = y * 320 + x;
        let pixel = self.vram[index]&0x3F; // 6-bit color index (0-63)
        // Simplified conversion to RGB (replace with actual color conversion logic)
        let clut = &self.vram[CLUT_START_ADDR..CLUT_START_ADDR + CLUT_TOTAL_SIZE]; // CLUT starts at 0x12C00
        (clut[(pixel as usize) * 3], clut[(pixel as usize) * 3 + 1], clut[(pixel as usize) * 3 + 2]) // return RGB888
    }
}