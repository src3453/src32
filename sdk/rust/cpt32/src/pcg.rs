//! PCG text screen, a Rust port of `sdk/cpt32/src/pcg.c` (same registers,
//! ring-row scrolling and attribute planes). The C library keeps its state in
//! globals; here it is the [`Pcg`] value.

use crate::mmio::write8;

const CONTROL: u32 = 0x8000_0001;
const MODE: u32 = 0x8000_f003;
const CURSOR_X: u32 = 0x8000_f005;
const CURSOR_Y: u32 = 0x8000_f006;
const VRAM: u32 = 0x1000_0000;
const FG_PLANE: u32 = 0x1000;
const BG_PLANE: u32 = 0x2000;

pub struct Pcg {
    pub cur_x: u32,
    pub cur_y: u32,
    pub width: u32,
    pub height: u32,
    pub cell_height: u32,
    pub fg: u32,
    pub bg: u32,
    scroll_y: u32,
    vram_row: u32,
}

fn fill(address: u32, size: u32, value: u8) {
    for i in 0..size {
        write8(address + i, value);
    }
}

impl Pcg {
    /// `pcg_initPCG`: 40×30 with the 8×8 font, scroll and cursor reset.
    pub fn init() -> Pcg {
        write8(CONTROL, 1);
        write8(MODE, 0);
        write8(0x8000_f000, 0);
        for register in 0x8000_f00c..=0x8000_f00f {
            write8(register, 0);
        }
        let mut pcg = Pcg {
            cur_x: 0,
            cur_y: 0,
            width: 40,
            height: 30,
            cell_height: 8,
            fg: 0x3f,
            bg: 0,
            scroll_y: 0,
            vram_row: 0,
        };
        pcg.update_row();
        write8(0x8000_f007, 1);
        write8(0x8000_f009, 10);
        pcg.update_cursor();
        pcg
    }

    /// `pcg_col40`
    pub fn col40(&mut self) {
        self.width = 40;
        self.cell_height = 8;
        write8(MODE, 0);
    }

    /// `pcg_col80`
    pub fn col80(&mut self) {
        self.width = 80;
        self.cell_height = 8;
        write8(MODE, 1);
    }

    /// `pcg_mode80x16`
    pub fn mode80x16(&mut self) {
        self.width = 80;
        self.height = 30;
        self.cell_height = 16;
        write8(MODE, 3);
    }

    fn update_row(&mut self) {
        self.vram_row = (self.scroll_y / self.cell_height + self.cur_y) % self.height;
    }

    fn cell(&self) -> u32 {
        VRAM + self.vram_row * self.width + self.cur_x
    }

    fn update_cursor(&self) {
        let p = self.cell();
        write8(p + FG_PLANE, self.fg as u8);
        write8(p + BG_PLANE, self.bg as u8);
        write8(CURSOR_X, self.cur_x as u8);
        write8(CURSOR_Y, self.vram_row as u8);
    }

    /// `pcg_locate`
    pub fn locate(&mut self, x: u32, y: u32) {
        self.cur_x = x;
        self.cur_y = y;
        self.update_row();
        self.update_cursor();
    }

    /// `pcg_cls`: glyph 0 everywhere and both attribute planes set to `color`.
    pub fn cls(&mut self, color: u32) {
        fill(VRAM, 0x1000, 0);
        fill(VRAM + FG_PLANE, 0x1000, color as u8);
        fill(VRAM + BG_PLANE, 0x1000, color as u8);
        self.update_cursor();
    }

    /// Next line, scrolling the ring by one row at the bottom (`pcg_ln`).
    pub fn newline(&mut self) {
        self.cur_x = 0;
        self.cur_y += 1;
        self.vram_row = (self.vram_row + 1) % self.height;
        if self.cur_y >= self.height {
            self.scroll_y = (self.scroll_y + self.cell_height) % (self.height * self.cell_height);
            write8(0x8000_f00e, self.scroll_y as u8);
            self.cur_y = self.height - 1;
            let p = VRAM + self.vram_row * self.width;
            fill(p, self.width, 0);
            fill(p + FG_PLANE, self.width, 0x3f);
            fill(p + BG_PLANE, self.width, 0);
        }
        self.update_cursor();
    }

    /// `pcg_putcscr`: one glyph in the current colors, wrapping at the right edge.
    pub fn putc(&mut self, glyph: u8) {
        let p = self.cell();
        write8(p, glyph);
        write8(p + FG_PLANE, self.fg as u8);
        write8(p + BG_PLANE, self.bg as u8);
        self.cur_x += 1;
        if self.cur_x >= self.width {
            self.newline();
        }
        self.update_cursor();
    }

    /// Writes the bytes of `text`; unlike `pcg_prnscr`, `\n` starts a new line.
    pub fn print(&mut self, text: &str) {
        for &byte in text.as_bytes() {
            if byte == b'\n' {
                self.newline();
            } else {
                self.putc(byte);
            }
        }
    }

    /// `pcg_prnlnscr`
    pub fn println(&mut self, text: &str) {
        self.print(text);
        self.newline();
    }

    /// `pcg_colorfg`
    pub fn set_fg(&mut self, color: u32) {
        self.fg = color;
        self.update_cursor();
    }

    /// `pcg_colorbg`
    pub fn set_bg(&mut self, color: u32) {
        self.bg = color;
        self.update_cursor();
    }
}

impl core::fmt::Write for Pcg {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        self.print(text);
        Ok(())
    }
}
