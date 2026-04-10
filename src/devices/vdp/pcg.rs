pub struct PcgRenderer {
    pub vram: Vec<u8>,
}

pub const PCG_CHAR_SIZE: usize = 8 * 8; // 8x8 pixels per character
pub const PCG_CHAR_COUNT: usize = 256; // 256 characters
pub const PCG_FONT_DATA_ADDR: u32 = 0x00000000; // VRAM address where font data starts