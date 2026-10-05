//! Sprite controller: VRAM SAT reader and indexed sprite rasterizer.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const SC_MMIO_BASE: u32 = 0x8001_0000;
pub const SC_MMIO_SIZE: u32 = 0x1_0000;
pub const SC_MAX_SPRITES: usize = 256;
const ENTRY_SIZE: usize = 16;
const VRAM_SIZE: usize = 0x40_0000;
const CLUT_START: usize = 0x12_c00;

const STATUS_INVALID_CONFIG: u32 = 1 << 1;
const STATUS_INVALID_SPRITE: u32 = 1 << 2;

pub struct Sc {
    vram: Rc<RefCell<Vec<u8>>>,
    enabled: bool,
    sat_base: u32,
    sprite_count: u32,
    output_gp: u8,
    status: Cell<u32>,
}

impl Sc {
    pub fn new(vram: Rc<RefCell<Vec<u8>>>) -> Self {
        Self {
            vram,
            enabled: false,
            sat_base: 0,
            sprite_count: 0,
            output_gp: 1,
            status: Cell::new(0),
        }
    }

    pub fn output_gp(&self) -> u8 {
        self.output_gp
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Rasterizes enabled sprites into a transparent 320x240 RGBA plane.
    pub fn render_frame(&self) -> Vec<[u8; 4]> {
        if !self.enabled || self.sprite_count == 0 {
            return Vec::new();
        }

        let count = self.sprite_count as usize;
        let sat_start = self.sat_base as usize;
        let sat_end = sat_start.checked_add(count.saturating_mul(ENTRY_SIZE));
        if sat_start & 15 != 0
            || count > SC_MAX_SPRITES
            || sat_end.is_none_or(|end| end > VRAM_SIZE)
        {
            self.set_error(STATUS_INVALID_CONFIG);
            return Vec::new();
        }

        let vram = self.vram.borrow();
        let mut output = vec![[0, 0, 0, 0]; 320 * 240];
        let mut order: Vec<(u8, usize)> = (0..count)
            .map(|i| (vram[sat_start + i * ENTRY_SIZE + 11], i))
            .collect();
        order.sort_unstable();
        for (_, index) in order {
            let entry = &vram[sat_start + index * ENTRY_SIZE..sat_start + (index + 1) * ENTRY_SIZE];
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
                if !(0..240).contains(&y) {
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
                    if !(0..320).contains(&x) {
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
                    let clut = CLUT_START + ((palette as usize + color_index as usize) & 0x3f) * 3;
                    output[y as usize * 320 + x as usize] =
                        [vram[clut], vram[clut + 1], vram[clut + 2], 255];
                }
            }
        }
        output
    }

    pub fn read_register(&self, addr: u32) -> u8 {
        let aligned = addr & !3;
        let value = match aligned {
            0x00 => 0x5343_3031,
            0x04 => self.enabled as u32,
            0x08 => self.status.get(),
            0x10 => self.sat_base,
            0x14 => self.sprite_count,
            0x18 => self.output_gp as u32,
            _ => 0,
        };
        let shift = (3 - (addr & 3)) * 8;
        (value >> shift) as u8
    }

    pub fn write_register(&mut self, addr: u32, value: u8) {
        let aligned = addr & !3;
        let lane_shift = (3 - (addr & 3)) * 8;
        match aligned {
            0x04 if addr & 3 == 3 => {
                if value & 2 != 0 {
                    self.reset();
                }
                if value & !3 != 0 {
                    self.set_error(STATUS_INVALID_CONFIG);
                }
                self.enabled = value & 1 != 0;
            }
            0x08 => self
                .status
                .set(self.status.get() & !((value as u32) << lane_shift)),
            0x10 => Self::write_u32_byte(&mut self.sat_base, addr, value),
            0x14 => Self::write_u32_byte(&mut self.sprite_count, addr, value),
            0x18 if addr & 3 == 3 => {
                if value < 8 {
                    self.output_gp = value;
                } else {
                    self.set_error(STATUS_INVALID_CONFIG);
                }
            }
            _ => {}
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
    }

    fn set_error(&self, bit: u32) {
        self.status.set(self.status.get() | bit);
    }
}
