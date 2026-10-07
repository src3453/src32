pub const UNIFONT_CODEPOINT_COUNT: usize = 0x1_0000;
pub const UNIFONT_GLYPH_SIZE: usize = 32;
pub const UNIFONT_ROM_SIZE: usize = UNIFONT_CODEPOINT_COUNT * UNIFONT_GLYPH_SIZE;
pub const UNIFONT_REPLACEMENT_CODE_UNIT: u16 = 0xFFFD;

pub static UNIFONT_BMP_ROM: &[u8; UNIFONT_ROM_SIZE] =
    include_bytes!("../../../assets/unifont-bmp.chr");

/// Return one 16-pixel glyph row as two bytes, left pixel in the most significant bit.
/// Surrogate code units select U+FFFD; rows outside 0..16 return `None`.
pub fn lookup_row(rom: &[u8; UNIFONT_ROM_SIZE], code_unit: u16, row: usize) -> Option<[u8; 2]> {
    if row >= 16 {
        return None;
    }
    let codepoint = if (0xD800..=0xDFFF).contains(&code_unit) {
        UNIFONT_REPLACEMENT_CODE_UNIT
    } else {
        code_unit
    };

    let offset = codepoint as usize * UNIFONT_GLYPH_SIZE + row * 2;
    Some([rom[offset], rom[offset + 1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surrogate_rows_use_replacement_glyph_and_row_bounds_are_checked() {
        for row in 0..16 {
            assert_eq!(
                lookup_row(UNIFONT_BMP_ROM, 0xD800, row),
                lookup_row(UNIFONT_BMP_ROM, UNIFONT_REPLACEMENT_CODE_UNIT, row)
            );
        }
        assert_eq!(lookup_row(UNIFONT_BMP_ROM, 0x3042, 16), None);
    }
}
