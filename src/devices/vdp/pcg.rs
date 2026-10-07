// PCG (Programmable Character Generator) renderer for the VDP (Video Display Processor)

// PCG Specification:
// Screen modes: 40x30 or 80x30 cells of 8x8 glyphs; mode 2 is 20x15 cells of 16x16 BMP glyphs.
// Display area: 320x240 pixels for the 40-column and 16x16 modes.
// Modes 0/1 use two rewritable 256-glyph font banks in VRAM; mode 2 uses the shared Unifont ROM.
// Color capabilities: 6-bit color, foreground and background colors can be set per character.

// VRAM Layout for PCG (from base address):
// 0x00000 - 0x004AF: Text data (40 columns x 30 rows = 1200 bytes)
// 0x01000 - 0x014AF: Attribute data 1 (FG color, 40 columns x 30 rows = 1200 bytes)
// 0x02000 - 0x024AF: Attribute data 2 (BG color, 40 columns x 30 rows = 1200 bytes)
// 0x12C00 - 0x12CBF: 64 CLUT (Color Look-Up Table) entries (RGB888; 192 bytes)
// 0x20000 - 0x207FF: Font bank 0 (256 characters x 8 bytes = 2048 bytes)
// 0x20800 - 0x20FFF: Font bank 1 (256 characters x 8 bytes = 2048 bytes)

// VDP PCG Mode specific registers (from base address):
// 0xF000: /PCG_ENABLE:RW (PCG enable) (0 = enable, 1 = disable)
// 0xF001: PCG_FONT_BANK:RW (PCG Font bank select) (0 = Bank 0, 1 = Bank 1)
// 0xF002: SWAP_FGBG:RW (Swap FG/BG colors) (0 = normal, 1 = swap)
// 0xF003: SCREEN_MODE:RW (0 = 40x30, 1 = 80x30, 2 = 20x15 cells of 16x16 glyphs)
// 0xF004: STATUS:R- (Status register) (0 = OK, 1 = Error)
// 0xF005: CURSOR_POS_X:RW (mode-dependent: 0-39, 0-79, or 0-19)
// 0xF006: CURSOR_POS_Y:RW (mode-dependent: 0-29 or 0-14)
// 0xF007: CURSOR_ENABLE:RW (Cursor enable) (0 = disable, 1 = enable)
// 0xF008: /CURSOR_LINES:RW (Cursor lines) (MSB: line 0, LSB: line 7; 0 = visible, 1 = invisible)
// 0xF009: CURSOR_BLINK_PERIOD:RW (Cursor blink period) (0 = none, 1~255 = blink period in frames (in each blink state, 1 means 2 frames in total))
// 0xFFFF: RESET:-W (Reset register) (write 1 to reset VDP state)

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::Path;
use std::rc::Rc;

use crate::devices::vdp::chr_rom::{UNIFONT_BMP_ROM, lookup_row};
use crate::devices::vdp::clut::CLUT_DEFAULT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PcgScreenMode {
    Columns40 = 0,
    Columns80 = 1,
    Unifont16x16 = 2,
}

impl PcgScreenMode {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Columns40),
            1 => Some(Self::Columns80),
            2 => Some(Self::Unifont16x16),
            _ => None,
        }
    }

    pub fn columns(self) -> usize {
        match self {
            Self::Columns40 => 40,
            Self::Columns80 => 80,
            Self::Unifont16x16 => 20,
        }
    }

    pub fn rows(self) -> usize {
        match self {
            Self::Unifont16x16 => 15,
            Self::Columns40 | Self::Columns80 => PCG_ROWS,
        }
    }

    pub fn cell_width(self) -> usize {
        match self {
            Self::Unifont16x16 => 16,
            Self::Columns40 | Self::Columns80 => PCG_CELL_WIDTH,
        }
    }

    pub fn cell_height(self) -> usize {
        match self {
            Self::Unifont16x16 => 16,
            Self::Columns40 | Self::Columns80 => PCG_CELL_HEIGHT,
        }
    }

    pub fn width(self) -> usize {
        self.columns() * self.cell_width()
    }

    pub fn height(self) -> usize {
        self.rows() * self.cell_height()
    }
}

pub const PCG_WIDTH: usize = 320;
pub const PCG_HEIGHT: usize = 240;
pub const PCG_COLUMNS: usize = 40;
pub const PCG_ROWS: usize = 30;
pub const PCG_CELL_WIDTH: usize = 8;
pub const PCG_CELL_HEIGHT: usize = 8;
pub const PCG_TEXT_SIZE: usize = PCG_COLUMNS * PCG_ROWS;
pub const PCG_ATTR_SIZE: usize = PCG_TEXT_SIZE;
pub const PCG_CLUT_START_ADDR: usize = 0x12C00;
pub const PCG_FONT_BANK_SIZE: usize = 256 * 8;
pub const PCG_FONT_TOTAL_SIZE: usize = PCG_FONT_BANK_SIZE * 2;
pub const PCG_FONT_BANK0_ADDR: usize = 0x20000;
pub const PCG_FONT_BANK1_ADDR: usize = 0x20800;
pub const PCG_TEXT_ADDR: usize = 0x00000;
pub const PCG_FG_ATTR_ADDR: usize = 0x01000;
pub const PCG_BG_ATTR_ADDR: usize = 0x02000;

pub struct PcgRenderer {
    vram: Rc<RefCell<Vec<u8>>>,
    unifont_rom: &'static [u8; crate::devices::vdp::chr_rom::UNIFONT_ROM_SIZE],
}

impl PcgRenderer {
    pub fn new(vram: Rc<RefCell<Vec<u8>>>) -> Self {
        Self {
            vram,
            unifont_rom: UNIFONT_BMP_ROM,
        }
    }

    pub fn init_clut(&self) {
        self.vram.borrow_mut()[PCG_CLUT_START_ADDR..PCG_CLUT_START_ADDR + CLUT_DEFAULT.len()]
            .copy_from_slice(&CLUT_DEFAULT);
    }

    pub fn load_font_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let font_data = fs::read(path)?;
        self.load_font_bytes(&font_data)
    }

    pub fn load_font_bytes(&self, font_data: &[u8]) -> io::Result<()> {
        let mut vram = self.vram.borrow_mut();

        match font_data.len() {
            PCG_FONT_BANK_SIZE => {
                vram[PCG_FONT_BANK0_ADDR..PCG_FONT_BANK0_ADDR + PCG_FONT_BANK_SIZE]
                    .copy_from_slice(font_data);
                vram[PCG_FONT_BANK1_ADDR..PCG_FONT_BANK1_ADDR + PCG_FONT_BANK_SIZE]
                    .copy_from_slice(font_data);
                Ok(())
            }
            PCG_FONT_TOTAL_SIZE => {
                vram[PCG_FONT_BANK0_ADDR..PCG_FONT_BANK0_ADDR + PCG_FONT_BANK_SIZE]
                    .copy_from_slice(&font_data[..PCG_FONT_BANK_SIZE]);
                vram[PCG_FONT_BANK1_ADDR..PCG_FONT_BANK1_ADDR + PCG_FONT_BANK_SIZE]
                    .copy_from_slice(&font_data[PCG_FONT_BANK_SIZE..]);
                Ok(())
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Invalid PCG font size: expected {} or {} bytes, got {}",
                    PCG_FONT_BANK_SIZE,
                    PCG_FONT_TOTAL_SIZE,
                    font_data.len()
                ),
            )),
        }
    }

    fn clut_rgb(vram: &[u8], color_index: u8) -> (u8, u8, u8) {
        let clut_index = PCG_CLUT_START_ADDR + (color_index as usize & 0x3F) * 3;
        (vram[clut_index], vram[clut_index + 1], vram[clut_index + 2])
    }

    pub fn get_pixel(
        &self,
        x: usize,
        y: usize,
        screen_mode: PcgScreenMode,
        font_bank: u8,
        swap_fg_bg: bool,
        cursor_pos_x: u8,
        cursor_pos_y: u8,
        cursor_enable: bool,
        cursor_lines: u8,
        cursor_blink_period: u8,
        cursor_blink_tick: u64,
    ) -> (u8, u8, u8) {
        let width = screen_mode.width();
        let columns = screen_mode.columns();
        let cell_width = screen_mode.cell_width();
        let cell_height = screen_mode.cell_height();
        if x >= width || y >= screen_mode.height() {
            return (0, 0, 0);
        }

        let vram = self.vram.borrow();
        let char_col = x / cell_width;
        let char_row = y / cell_height;
        let char_index = char_row * columns + char_col;
        let row_in_char = y % cell_height;
        let local_x = x % cell_width;
        let is_unifont_mode = screen_mode == PcgScreenMode::Unifont16x16;
        let bit_set = if is_unifont_mode {
            let text_offset = PCG_TEXT_ADDR + char_index * 2;
            let code_unit = u16::from_be_bytes([vram[text_offset], vram[text_offset + 1]]);
            let row = lookup_row(self.unifont_rom, code_unit, row_in_char)
                .expect("PCG mode has a 16-row glyph");
            row[local_x / 8] & (1 << (7 - local_x % 8)) != 0
        } else {
            let glyph = vram[PCG_TEXT_ADDR + char_index] as usize;
            let bank_base = if font_bank & 1 == 0 {
                PCG_FONT_BANK0_ADDR
            } else {
                PCG_FONT_BANK1_ADDR
            };
            let font_offset = bank_base + glyph * PCG_CELL_HEIGHT + row_in_char;
            let row_bits = vram[font_offset];
            row_bits & (1 << (PCG_CELL_WIDTH - 1 - local_x)) != 0
        };
        let fg = vram[PCG_FG_ATTR_ADDR + char_index] & 0x3F;
        let bg = vram[PCG_BG_ATTR_ADDR + char_index] & 0x3F;
        let use_fg = if swap_fg_bg { !bit_set } else { bit_set };
        let color = if use_fg { fg } else { bg };
        let mut rgb = Self::clut_rgb(&vram, color);

        if cursor_enable {
            let cursor_x = cursor_pos_x as usize;
            let cursor_y = cursor_pos_y as usize;
            if cursor_x < screen_mode.columns() && cursor_y < screen_mode.rows() {
                let cell_x = x / cell_width;
                let cell_y = y / cell_height;
                if cell_x == cursor_x && cell_y == cursor_y {
                    let line = y % cell_height;
                    let cursor_line = if is_unifont_mode { line / 2 } else { line };
                    let line_visible = ((cursor_lines >> (7 - cursor_line)) & 1) == 0;
                    let cursor_visible = if cursor_blink_period == 0 {
                        true
                    } else {
                        ((cursor_blink_tick / cursor_blink_period as u64) & 1) == 0
                    };

                    if line_visible && cursor_visible {
                        rgb = (255 - rgb.0, 255 - rgb.1, 255 - rgb.2);
                    }
                }
            }
        }

        rgb
    }
}
