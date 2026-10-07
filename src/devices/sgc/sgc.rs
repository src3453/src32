//! Screen graphics controller: sprite rasterizer and FIFO-driven 2D renderer.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use crate::devices::vdp::chr_rom::{UNIFONT_BMP_ROM, lookup_row};

pub const SGC_MMIO_BASE: u32 = 0x8001_0000;
pub const SGC_MMIO_SIZE: u32 = 0x1_0000;
pub const SGC_MAX_SPRITES: usize = 256;
const ENTRY_SIZE: usize = 16;
const VRAM_SIZE: usize = 0x40_0000;
const CLUT_START: usize = 0x12_c00;
const FRAME_WIDTH: i32 = 320;
const FRAME_HEIGHT: i32 = 240;
const FRAME_PIXELS: usize = (FRAME_WIDTH * FRAME_HEIGHT) as usize;
const FIFO_CAPACITY: usize = 256;

const STATUS_BUSY: u32 = 1 << 0;
const STATUS_INVALID_CONFIG: u32 = 1 << 1;
const STATUS_INVALID_SPRITE: u32 = 1 << 2;
const STATUS_FIFO_FULL: u32 = 1 << 3;
const STATUS_FIFO_ERROR: u32 = 1 << 4;

const OP_SET_COLOR: u8 = 0x01;
const OP_LINE: u8 = 0x10;
const OP_RECT: u8 = 0x11;
const OP_TRIANGLE: u8 = 0x12;
const OP_GLYPH: u8 = 0x13;
const OP_NOP: u8 = 0xff;

pub struct Sgc {
    vram: Rc<RefCell<Vec<u8>>>,
    enabled: bool,
    sat_base: u32,
    sprite_count: u32,
    output_gp: u8,
    status: Cell<u32>,
    fifo: RefCell<VecDeque<u32>>,
    fifo_write_latch: u32,
    fifo_write_mask: u8,
    color_index: Cell<u8>,
    command_pixels: RefCell<Vec<[u8; 4]>>,
}

impl Sgc {
    pub fn new(vram: Rc<RefCell<Vec<u8>>>) -> Self {
        Self {
            vram,
            enabled: false,
            sat_base: 0,
            sprite_count: 0,
            output_gp: 1,
            status: Cell::new(0),
            fifo: RefCell::new(VecDeque::with_capacity(FIFO_CAPACITY)),
            fifo_write_latch: 0,
            fifo_write_mask: 0,
            color_index: Cell::new(0),
            command_pixels: RefCell::new(vec![[0, 0, 0, 0]; FRAME_PIXELS]),
        }
    }

    pub fn output_gp(&self) -> u8 {
        self.output_gp
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn tick(&self) {
        if self.enabled {
            self.execute_fifo();
        }
    }

    /// Rasterizes enabled sprites over the persistent FIFO command plane.
    pub fn render_frame(&self) -> Vec<[u8; 4]> {
        if !self.enabled {
            return Vec::new();
        }
        let count = self.sprite_count as usize;
        let sat_start = self.sat_base as usize;
        let sat_end = sat_start.checked_add(count.saturating_mul(ENTRY_SIZE));
        let valid_sat = sat_start & 15 == 0
            && count <= SGC_MAX_SPRITES
            && sat_end.is_some_and(|end| end <= VRAM_SIZE);
        if !valid_sat {
            self.set_error(STATUS_INVALID_CONFIG);
        }
        let vram = self.vram.borrow();
        let mut output = vec![[0, 0, 0, 0]; FRAME_PIXELS];
        if valid_sat && count != 0 {
            let mut order: Vec<(u8, usize)> = (0..count)
                .map(|i| (vram[sat_start + i * ENTRY_SIZE + 11], i))
                .collect();
            order.sort_unstable();
            for (_, index) in order {
                let entry =
                    &vram[sat_start + index * ENTRY_SIZE..sat_start + (index + 1) * ENTRY_SIZE];
                let flags = entry[14];
                if flags & 1 == 0 {
                    continue;
                }
                let width = entry[4] as usize;
                let height = entry[5] as usize;
                let sx = entry[12] as usize;
                let sy = entry[13] as usize;
                if width == 0
                    || width > 64
                    || height == 0
                    || height > 64
                    || !(1..=4).contains(&sx)
                    || !(1..=4).contains(&sy)
                    || flags & !7 != 0
                    || entry[15] != 0
                {
                    self.set_error(STATUS_INVALID_SPRITE);
                    continue;
                }
                let pattern = u32::from_be_bytes([entry[6], entry[7], entry[8], entry[9]]) as usize;
                let pixel_count = width * height;
                let byte_count = pixel_count.div_ceil(2);
                let Some(pattern_end) = pattern.checked_add(byte_count) else {
                    self.set_error(STATUS_INVALID_SPRITE);
                    continue;
                };
                if pattern_end > vram.len() {
                    self.set_error(STATUS_INVALID_SPRITE);
                    continue;
                }

                let x0 = i16::from_be_bytes([entry[0], entry[1]]) as i32;
                let y0 = i16::from_be_bytes([entry[2], entry[3]]) as i32;
                let palette = entry[10] & 0x3f;
                for dy in 0..height * sy {
                    let y = y0 + dy as i32;
                    if !(0..FRAME_HEIGHT).contains(&y) {
                        continue;
                    }
                    let source_y = dy / sy;
                    let source_y = if flags & 4 != 0 {
                        height - 1 - source_y
                    } else {
                        source_y
                    };
                    for dx in 0..width * sx {
                        let x = x0 + dx as i32;
                        if !(0..FRAME_WIDTH).contains(&x) {
                            continue;
                        }
                        let source_x = dx / sx;
                        let source_x = if flags & 2 != 0 {
                            width - 1 - source_x
                        } else {
                            source_x
                        };
                        let texel_index = source_y * width + source_x;
                        let packed = vram[pattern + texel_index / 2];
                        let color_index = if texel_index & 1 == 0 {
                            packed >> 4
                        } else {
                            packed & 0x0f
                        };
                        if color_index == 0 {
                            continue;
                        }
                        let clut =
                            CLUT_START + ((palette as usize + color_index as usize) & 0x3f) * 3;
                        output[y as usize * FRAME_WIDTH as usize + x as usize] =
                            [vram[clut], vram[clut + 1], vram[clut + 2], 255];
                    }
                }
            }
        }

        for (dst, src) in output.iter_mut().zip(self.command_pixels.borrow().iter()) {
            if src[3] != 0 {
                *dst = *src;
            }
        }
        output
    }

    fn execute_fifo(&self) {
        let mut fifo = self.fifo.borrow_mut();
        let vram = self.vram.borrow();
        let mut pixels = self.command_pixels.borrow_mut();
        loop {
            let Some(&header) = fifo.front() else { break };
            let opcode = (header >> 24) as u8;
            let length = match opcode {
                OP_SET_COLOR | OP_NOP => 1,
                OP_LINE | OP_RECT => 5,
                OP_TRIANGLE => 7,
                OP_GLYPH => 4,
                _ => 1,
            };
            if fifo.len() < length {
                break;
            }
            let mut words = [0u32; 7];
            for word in words.iter_mut().take(length) {
                *word = fifo.pop_front().expect("FIFO length checked");
            }
            match opcode {
                OP_SET_COLOR => {
                    if header & 0x00ff_ffc0 != 0 {
                        self.set_error(STATUS_FIFO_ERROR);
                    } else {
                        self.color_index.set((header & 0x3f) as u8);
                    }
                }
                OP_LINE | OP_RECT => {
                    if header & 0x00ff_ffff != 0 {
                        self.set_error(STATUS_FIFO_ERROR);
                        continue;
                    }
                    let Some(coords) = coordinates(&words[1..5]) else {
                        self.set_error(STATUS_FIFO_ERROR);
                        continue;
                    };
                    let color = color_rgba(&vram, self.color_index.get());
                    if opcode == OP_LINE {
                        draw_line(&mut pixels, coords, color);
                    } else if coords[2] <= coords[0] || coords[3] <= coords[1] {
                        self.set_error(STATUS_FIFO_ERROR);
                    } else {
                        draw_rect(&mut pixels, coords, color);
                    }
                }
                OP_TRIANGLE => {
                    if header & 0x00ff_ffff != 0 {
                        self.set_error(STATUS_FIFO_ERROR);
                        continue;
                    }
                    let Some(coords) = coordinates(&words[1..7]) else {
                        self.set_error(STATUS_FIFO_ERROR);
                        continue;
                    };
                    let color = color_rgba(&vram, self.color_index.get());
                    if !draw_triangle(&mut pixels, coords, color) {
                        self.set_error(STATUS_FIFO_ERROR);
                    }
                }
                OP_GLYPH => {
                    if header & 0x00ff_ffff != 0
                        || words[1] & 0xffff_0000 != 0
                        || words[2] & 0xffff_0000 != 0
                        || words[3] & 0xffff_0000 != 0
                    {
                        self.set_error(STATUS_FIFO_ERROR);
                        continue;
                    }
                    let code_unit = words[1] as u16;
                    let x0 = words[2] as u16 as i16 as i32;
                    let y0 = words[3] as u16 as i16 as i32;
                    let color = color_rgba(&vram, self.color_index.get());
                    draw_glyph(&mut pixels, code_unit, x0, y0, color);
                }
                OP_NOP => {
                    if header & 0x00ff_ffff != 0 {
                        self.set_error(STATUS_FIFO_ERROR);
                    }
                }
                _ => self.set_error(STATUS_FIFO_ERROR),
            }
        }
    }

    pub fn read_register(&self, addr: u32) -> u8 {
        let aligned = addr & !3;
        let fifo_len = self.fifo.borrow().len() as u32;
        let value = match aligned {
            0x00 => 0x5347_4331,
            0x04 => self.enabled as u32,
            0x08 => {
                (self.status.get() & !STATUS_BUSY)
                    | if self.enabled && fifo_len != 0 {
                        STATUS_BUSY
                    } else {
                        0
                    }
            }
            0x10 => self.sat_base,
            0x14 => self.sprite_count,
            0x18 => self.output_gp as u32,
            0x20 => 0,
            0x24 => (fifo_len << 16) | (FIFO_CAPACITY as u32 - fifo_len),
            0x28 => 0,
            _ => 0,
        };
        let shift = (3 - (addr & 3)) * 8;
        (value >> shift) as u8
    }

    pub fn write_register(&mut self, addr: u32, value: u8) {
        let aligned = addr & !3;
        let lane_shift = (3 - (addr & 3)) * 8;
        match aligned {
            0x04 => {
                if addr & 3 != 3 {
                    if value != 0 {
                        self.set_error(STATUS_INVALID_CONFIG);
                    }
                } else {
                    if value & 2 != 0 {
                        self.reset();
                    }
                    if value & !3 != 0 {
                        self.set_error(STATUS_INVALID_CONFIG);
                    }
                    self.enabled = value & 1 != 0;
                }
            }
            0x08 => self
                .status
                .set(self.status.get() & !((value as u32) << lane_shift)),
            0x10 => Self::write_u32_byte(&mut self.sat_base, addr, value),
            0x14 => Self::write_u32_byte(&mut self.sprite_count, addr, value),
            0x18 => {
                if addr & 3 != 3 {
                    if value != 0 {
                        self.set_error(STATUS_INVALID_CONFIG);
                    }
                } else if value < 8 {
                    self.output_gp = value;
                } else {
                    self.set_error(STATUS_INVALID_CONFIG);
                }
            }
            0x20 => self.write_fifo_byte(addr, value),
            0x28 => {
                if addr & 3 != 3 {
                    if value != 0 {
                        self.set_error(STATUS_INVALID_CONFIG);
                    }
                } else {
                    if value & 1 != 0 {
                        if !self.fifo.borrow().is_empty() || self.fifo_write_mask != 0 {
                            self.set_error(STATUS_FIFO_ERROR);
                        }
                        self.fifo.borrow_mut().clear();
                        self.fifo_write_latch = 0;
                        self.fifo_write_mask = 0;
                    }
                    if value & !1 != 0 {
                        self.set_error(STATUS_INVALID_CONFIG);
                    }
                }
            }
            _ => {}
        }
    }

    fn write_fifo_byte(&mut self, addr: u32, value: u8) {
        let lane = (addr & 3) as u8;
        let shift = (3 - lane) * 8;
        self.fifo_write_latch =
            (self.fifo_write_latch & !(0xff << shift)) | ((value as u32) << shift);
        self.fifo_write_mask |= 1 << lane;
        if self.fifo_write_mask == 0x0f {
            let word = self.fifo_write_latch;
            self.fifo_write_latch = 0;
            self.fifo_write_mask = 0;
            let mut fifo = self.fifo.borrow_mut();
            if fifo.len() == FIFO_CAPACITY {
                self.set_error(STATUS_FIFO_FULL);
            } else {
                fifo.push_back(word);
            }
        }
    }

    fn write_u32_byte(target: &mut u32, addr: u32, value: u8) {
        let shift = (3 - (addr & 3)) * 8;
        *target = (*target & !(0xff << shift)) | ((value as u32) << shift);
    }

    fn reset(&mut self) {
        self.enabled = false;
        self.sat_base = 0;
        self.sprite_count = 0;
        self.output_gp = 1;
        self.status.set(0);
        self.fifo.borrow_mut().clear();
        self.fifo_write_latch = 0;
        self.fifo_write_mask = 0;
        self.color_index.set(0);
        self.command_pixels.borrow_mut().fill([0, 0, 0, 0]);
    }

    fn set_error(&self, bit: u32) {
        self.status.set(self.status.get() | bit);
    }
}

fn coordinates(words: &[u32]) -> Option<[i32; 6]> {
    let mut coords = [0; 6];
    for (out, word) in coords.iter_mut().zip(words) {
        if word & 0xffff_0000 != 0 {
            return None;
        }
        *out = *word as u16 as i16 as i32;
    }
    Some(coords)
}

fn color_rgba(vram: &[u8], index: u8) -> [u8; 4] {
    let offset = CLUT_START + (index as usize & 0x3f) * 3;
    [vram[offset], vram[offset + 1], vram[offset + 2], 255]
}

fn draw_line(pixels: &mut [[u8; 4]], coords: [i32; 6], color: [u8; 4]) {
    let (mut x0, mut y0, x1, y1) = (coords[0], coords[1], coords[2], coords[3]);
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        put_pixel(pixels, x0, y0, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let twice_error = 2 * error;
        if twice_error >= dy {
            error += dy;
            x0 += sx;
        }
        if twice_error <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

fn draw_rect(pixels: &mut [[u8; 4]], coords: [i32; 6], color: [u8; 4]) {
    let x0 = coords[0].clamp(0, FRAME_WIDTH);
    let y0 = coords[1].clamp(0, FRAME_HEIGHT);
    let x1 = coords[2].clamp(0, FRAME_WIDTH);
    let y1 = coords[3].clamp(0, FRAME_HEIGHT);
    for y in y0..y1 {
        let start = y as usize * FRAME_WIDTH as usize + x0 as usize;
        let end = y as usize * FRAME_WIDTH as usize + x1 as usize;
        pixels[start..end].fill(color);
    }
}

fn edge(a: (i32, i32), b: (i32, i32), p: (i32, i32)) -> i64 {
    (b.0 - a.0) as i64 * (p.1 - a.1) as i64 - (b.1 - a.1) as i64 * (p.0 - a.0) as i64
}

fn draw_triangle(pixels: &mut [[u8; 4]], coords: [i32; 6], color: [u8; 4]) -> bool {
    let a = (coords[0], coords[1]);
    let b = (coords[2], coords[3]);
    let c = (coords[4], coords[5]);
    let area = edge(a, b, c);
    if area == 0 {
        return false;
    }
    let min_x = a.0.min(b.0).min(c.0).clamp(0, FRAME_WIDTH - 1);
    let max_x = a.0.max(b.0).max(c.0).clamp(0, FRAME_WIDTH - 1);
    let min_y = a.1.min(b.1).min(c.1).clamp(0, FRAME_HEIGHT - 1);
    let max_y = a.1.max(b.1).max(c.1).clamp(0, FRAME_HEIGHT - 1);
    if a.0.max(b.0).max(c.0) < 0
        || a.1.max(b.1).max(c.1) < 0
        || a.0.min(b.0).min(c.0) >= FRAME_WIDTH
        || a.1.min(b.1).min(c.1) >= FRAME_HEIGHT
    {
        return true;
    }
    let orientation = area.signum();
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let point = (x, y);
            if edge(a, b, point) * orientation >= 0
                && edge(b, c, point) * orientation >= 0
                && edge(c, a, point) * orientation >= 0
            {
                put_pixel(pixels, x, y, color);
            }
        }
    }
    true
}

fn draw_glyph(pixels: &mut [[u8; 4]], code_unit: u16, x0: i32, y0: i32, color: [u8; 4]) {
    for row in 0..16 {
        let bits = lookup_row(UNIFONT_BMP_ROM, code_unit, row).expect("row is in glyph bounds");
        for x in 0..16 {
            if bits[x / 8] & (0x80 >> (x & 7)) != 0 {
                put_pixel(pixels, x0 + x as i32, y0 + row as i32, color);
            }
        }
    }
}

fn put_pixel(pixels: &mut [[u8; 4]], x: i32, y: i32, color: [u8; 4]) {
    if (0..FRAME_WIDTH).contains(&x) && (0..FRAME_HEIGHT).contains(&y) {
        pixels[y as usize * FRAME_WIDTH as usize + x as usize] = color;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_sgc() -> Sgc {
        let vram = Rc::new(RefCell::new(vec![0; VRAM_SIZE]));
        {
            let mut memory = vram.borrow_mut();
            memory[CLUT_START + 3..CLUT_START + 6].copy_from_slice(&[240, 20, 10]);
            memory[CLUT_START + 6..CLUT_START + 9].copy_from_slice(&[5, 200, 30]);
        }
        let mut sgc = Sgc::new(vram);
        sgc.write_register(0x07, 1);
        sgc
    }

    fn push_word(sgc: &mut Sgc, word: u32) {
        for (lane, byte) in word.to_be_bytes().into_iter().enumerate() {
            sgc.write_register(0x20 + lane as u32, byte);
        }
    }

    fn read_word(sgc: &Sgc, offset: u32) -> u32 {
        (0..4).fold(0, |word, lane| {
            (word << 8) | sgc.read_register(offset + lane) as u32
        })
    }

    #[test]
    fn fifo_rect_clips_right_and_bottom_exclusive_and_keeps_pixels_between_frames() {
        let mut sgc = enabled_sgc();
        push_word(&mut sgc, 0x0100_0001);
        push_word(&mut sgc, 0x1100_0000);
        push_word(&mut sgc, (-1i16 as u16) as u32);
        push_word(&mut sgc, 1);
        push_word(&mut sgc, 2);
        push_word(&mut sgc, 3);

        assert_eq!(read_word(&sgc, 0x24), 0x0006_00fa);
        sgc.tick();
        let frame = sgc.render_frame();
        assert_eq!(frame[1 * 320], [240, 20, 10, 255]);
        assert_eq!(frame[2 * 320 + 1], [240, 20, 10, 255]);
        assert_eq!(frame[3 * 320], [0, 0, 0, 0]);
        assert_eq!(sgc.render_frame()[1 * 320], [240, 20, 10, 255]);
    }

    #[test]
    fn fifo_line_includes_clipped_endpoints_and_triangle_includes_edges() {
        let mut sgc = enabled_sgc();
        push_word(&mut sgc, 0x0100_0001);
        push_word(&mut sgc, 0x1000_0000);
        for coordinate in [(-1i16 as u16) as u32, 2, 2, 2] {
            push_word(&mut sgc, coordinate);
        }
        push_word(&mut sgc, 0x0100_0002);
        push_word(&mut sgc, 0x1200_0000);
        for coordinate in [5, 2, 7, 2, 5, 4] {
            push_word(&mut sgc, coordinate);
        }
        sgc.tick();
        let frame = sgc.render_frame();
        assert_eq!(frame[2 * 320], [240, 20, 10, 255]);
        assert_eq!(frame[2 * 320 + 2], [240, 20, 10, 255]);
        assert_eq!(frame[2 * 320 + 3], [0, 0, 0, 0]);
        assert_eq!(frame[2 * 320 + 5], [5, 200, 30, 255]);
        assert_eq!(frame[3 * 320 + 6], [5, 200, 30, 255]);
    }

    #[test]
    fn disabled_sgc_holds_fifo_and_fifo_clear_preserves_drawn_pixels() {
        let mut sgc = enabled_sgc();
        sgc.enabled = false;
        push_word(&mut sgc, 0x0100_0001);
        push_word(&mut sgc, 0x1100_0000);
        for coordinate in [2, 2, 4, 4] {
            push_word(&mut sgc, coordinate);
        }
        sgc.tick();
        assert_eq!(read_word(&sgc, 0x24), 0x0006_00fa);
        assert_eq!(read_word(&sgc, 0x08) & STATUS_BUSY, 0);

        sgc.write_register(0x07, 1);
        sgc.tick();
        assert_eq!(read_word(&sgc, 0x24), 0x0000_0100);
        assert_eq!(sgc.render_frame()[2 * 320 + 2], [240, 20, 10, 255]);
        sgc.write_register(0x2b, 1);
        assert_eq!(sgc.render_frame()[2 * 320 + 2], [240, 20, 10, 255]);

        push_word(&mut sgc, 0x1000_0000);
        push_word(&mut sgc, 2);
        sgc.write_register(0x2b, 1);
        assert_ne!(sgc.status.get() & STATUS_FIFO_ERROR, 0);
        assert_eq!(read_word(&sgc, 0x24), 0x0000_0100);
    }
    #[test]
    fn incomplete_command_stays_queued_until_all_payload_words_arrive() {
        let mut sgc = enabled_sgc();
        push_word(&mut sgc, 0x0100_0002);
        push_word(&mut sgc, 0x1100_0000);
        push_word(&mut sgc, 4);
        push_word(&mut sgc, 4);
        sgc.tick();
        assert_eq!(read_word(&sgc, 0x24), 0x0003_00fd);
        assert_eq!(sgc.render_frame()[5 * 320 + 5], [0, 0, 0, 0]);

        push_word(&mut sgc, 7);
        push_word(&mut sgc, 6);
        sgc.tick();
        let frame = sgc.render_frame();
        assert_eq!(frame[4 * 320 + 5], [5, 200, 30, 255]);
        assert_eq!(frame[6 * 320 + 6], [0, 0, 0, 0]);
    }

    #[test]
    fn fifo_overflow_is_sticky_and_dropped_word_does_not_change_accepted_commands() {
        let mut sgc = enabled_sgc();
        for _ in 0..FIFO_CAPACITY {
            push_word(&mut sgc, 0xff00_0000);
        }
        push_word(&mut sgc, 0x0100_0001);
        assert_ne!(sgc.status.get() & STATUS_FIFO_FULL, 0);
        assert_eq!(read_word(&sgc, 0x24), 0x0100_0000);
        sgc.write_register(0x0b, STATUS_FIFO_FULL as u8);
        assert_eq!(sgc.status.get() & STATUS_FIFO_FULL, 0);
    }

    #[test]
    fn degenerate_triangle_sets_fifo_error_and_following_glyph_executes() {
        let mut sgc = enabled_sgc();
        push_word(&mut sgc, 0x0100_0001);
        push_word(&mut sgc, 0x1200_0000);
        for coordinate in [1, 1, 2, 2, 3, 3] {
            push_word(&mut sgc, coordinate);
        }
        push_word(&mut sgc, 0x1300_0000);
        push_word(&mut sgc, 0x0000_3042);
        push_word(&mut sgc, 10);
        push_word(&mut sgc, 10);
        sgc.tick();
        assert_ne!(sgc.status.get() & STATUS_FIFO_ERROR, 0);

        let row = lookup_row(UNIFONT_BMP_ROM, 0x3042, 1).unwrap();
        let x = (0..16)
            .find(|x| row[x / 8] & (0x80 >> (x & 7)) != 0)
            .unwrap();
        assert_eq!(sgc.render_frame()[11 * 320 + 10 + x], [240, 20, 10, 255]);
    }
}
