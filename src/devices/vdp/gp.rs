// Graphic Plane VRAM Layout (From base address):
// In graphic mode:
// 0x00000 - 0x12BFF: 320x240@8bpp (MSB 2bits are reserved; 76,800 bytes) image data
// 0x12C00 - 0x12CBF: 64 palette entries (RGB888; 192 bytes)

pub struct Gp0 {
    pub vram: Vec<u8>,
}

impl Gp0 {
    pub fn new(size: usize) -> Self {
        Self {
            vram: vec![0; size],
        }
    }

    pub fn get_vram(&self) -> &[u8] {
        &self.vram
    }
}
