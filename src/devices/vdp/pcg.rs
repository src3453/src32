// PCG (Programmable Character Generator) renderer for the VDP (Video Display Processor)

// PCG Specification:
// Screen resolution: 320x240 pixels
// Character size: 8x8 pixels
// Screen layout: 40 columns x 30 rows (320/8 = 40, 240/8 = 30)
// Number of characters: 256 (CP437 character set)
// Character data: rewritable font data stored in VRAM, 2 Font banks (Bank 0 and Bank 1), each bank can hold 256 characters (8 bytes per character, total 2048 bytes per bank)
// Color capabilities: 6-bit color, foreground and background colors can be set per character

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
// 0xF003: SCREEN_MODE:RW (Screen mode select) (0 = 40x30, 1 = 80x30)
// 0xF004: STATUS:R- (Status register) (0 = OK, 1 = Error)
// 0xF005: CURSOR_POS_X:RW (Cursor X position) (0-39 for 40 columns, 0-79 for 80 columns)
// 0xF006: CURSOR_POS_Y:RW (Cursor Y position) (0-29 for 30 rows)
// 0xF007: CURSOR_ENABLE:RW (Cursor enable) (0 = disable, 1 = enable)
// 0xF008: /CURSOR_LINES:RW (Cursor lines) (MSB: line 0, LSB: line 7; 0 = visible, 1 = invisible)
// 0xF009: CURSOR_BLINK_PERIOD:RW (Cursor blink period) (0 = none, 1~255 = blink period in frames (in each blink state, 1 means 2 frames in total))
// 0xFFFF: RESET:-W (Reset register) (write 1 to reset VDP state)

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::Path;
use std::rc::Rc;

use crate::devices::vdp::clut::CLUT_DEFAULT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcgScreenMode {
    Columns40 = 0,
    Columns80 = 1,
}

impl PcgScreenMode {
    pub fn from_u8(value: u8) -> Self {
        match value & 1 {
            0 => Self::Columns40,
            _ => Self::Columns80,
        }
    }

    pub fn columns(self) -> usize {
        match self {
            Self::Columns40 => 40,
            Self::Columns80 => 80,
        }
    }

    pub fn width(self) -> usize {
        self.columns() * PCG_CELL_WIDTH
    }

    pub fn height(self) -> usize {
        PCG_ROWS * PCG_CELL_HEIGHT
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
}

impl PcgRenderer {
    pub fn new(vram: Rc<RefCell<Vec<u8>>>) -> Self {
        Self { vram }
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
        if x >= width || y >= screen_mode.height() {
            return (0, 0, 0);
        }

        let vram = self.vram.borrow();
        let char_col = x / PCG_CELL_WIDTH;
        let char_row = y / PCG_CELL_HEIGHT;
        let char_index = char_row * columns + char_col;
        let row_in_char = y % PCG_CELL_HEIGHT;
        let bit_in_row = PCG_CELL_WIDTH - 1 - (x % PCG_CELL_WIDTH);
        let glyph = vram[PCG_TEXT_ADDR + char_index] as usize;
        let fg = vram[PCG_FG_ATTR_ADDR + char_index] & 0x3F;
        let bg = vram[PCG_BG_ATTR_ADDR + char_index] & 0x3F;
        let bank_base = if font_bank & 1 == 0 {
            PCG_FONT_BANK0_ADDR
        } else {
            PCG_FONT_BANK1_ADDR
        };
        let font_offset = bank_base + glyph * PCG_CELL_HEIGHT + row_in_char;
        let row_bits = vram[font_offset];
        let bit_set = ((row_bits >> bit_in_row) & 1) != 0;
        let use_fg = if swap_fg_bg { !bit_set } else { bit_set };
        let color = if use_fg { fg } else { bg };
        let mut rgb = Self::clut_rgb(&vram, color);

        if cursor_enable {
            let cursor_x = cursor_pos_x as usize;
            let cursor_y = cursor_pos_y as usize;
            if cursor_x < screen_mode.columns() && cursor_y < PCG_ROWS {
                let cell_x = x / PCG_CELL_WIDTH;
                let cell_y = y / PCG_CELL_HEIGHT;
                if cell_x == cursor_x && cell_y == cursor_y {
                    let line = y % PCG_CELL_HEIGHT;
                    let line_visible = ((cursor_lines >> (7 - line)) & 1) == 0;
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
