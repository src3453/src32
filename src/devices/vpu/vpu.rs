//! FIFO command interpreter and RGBA plane for the VPU.

pub(crate) const VPU_MMIO_BASE: u32 = 0x80030000;
pub(crate) const VPU_MMIO_SIZE: u32 = 0x10000;
pub const VPU_WIDTH: usize = 320;
pub const VPU_HEIGHT: usize = 240;
const FIFO_CAPACITY: usize = 4096;

pub struct Vpu {
    pub(crate) plane: Vec<u8>,
    depth: Vec<i32>,
    pub(crate) fifo: Vec<u32>,
    pub(crate) command: Vec<u32>,
    pub(crate) expected: Option<usize>,
    pub(crate) status: u32,
    pub(crate) write_latch: u32,
    pub(crate) write_mask: u8,
    pub(crate) output_gp: u8,
}

impl Vpu {
    pub fn new() -> Self {
        Self {
            plane: vec![0; VPU_WIDTH * VPU_HEIGHT * 4],
            depth: vec![i32::MAX; VPU_WIDTH * VPU_HEIGHT],
            fifo: Vec::new(),
            command: Vec::new(),
            expected: None,
            status: 0,
            write_latch: 0,
            write_mask: 0,
            output_gp: 7,
        }
    }

    pub fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
        if x >= VPU_WIDTH || y >= VPU_HEIGHT {
            return (0, 0, 0, 0);
        }
        let i = (y * VPU_WIDTH + x) * 4;
        (
            self.plane[i],
            self.plane[i + 1],
            self.plane[i + 2],
            self.plane[i + 3],
        )
    }

    pub(crate) fn push(&mut self, word: u32) {
        if self.fifo.len() == FIFO_CAPACITY {
            self.status |= 1 << 5;
            return;
        }
        self.fifo.push(word);
        self.status |= 1;
        self.pump();
    }

    fn pump(&mut self) {
        while !self.fifo.is_empty() {
            let word = self.fifo.remove(0);
            if self.expected.is_none() {
                let opcode = (word >> 24) as u8;
                let count = (word & 0x00ff_ffff) as usize;
                if opcode == 0 {
                    self.status |= 1 << 2;
                    continue;
                }
                if count > FIFO_CAPACITY {
                    self.status |= 1 << 3;
                    continue;
                }
                self.command.clear();
                self.command.push(opcode as u32);
                self.expected = Some(count);
                if count == 0 {
                    self.execute();
                }
            } else {
                self.command.push(word);
                let left = self.expected.unwrap() - 1;
                self.expected = Some(left);
                if left == 0 {
                    self.execute();
                }
            }
        }
        self.status &= !1;
    }

    fn execute(&mut self) {
        let opcode = self.command[0] as u8;
        let p = self.command[1..].to_vec();
        match opcode {
            0x04 if p.len() == 3 => {
                let flags = p[0];
                if flags & 1 != 0 {
                    let rgba = p[1].to_be_bytes();
                    for px in self.plane.chunks_exact_mut(4) {
                        px.copy_from_slice(&rgba);
                    }
                }
                if flags & 2 != 0 {
                    self.depth.fill(i32::MAX);
                }
                if flags & !3 != 0 {
                    self.status |= 1 << 3;
                }
            }
            // DRAW_FLAT_TRIANGLE: three (x, y, depth) integer vertices,
            // packed RGB, and an 8-bit diffuse intensity. Color is constant
            // across the face (flat shading); no texture or interpolation.
            0x12 if p.len() == 11 => self.draw_flat_triangle(&p),
            // The interpreter deliberately rejects commands it cannot safely consume.
            0x01 | 0x02 | 0x03 | 0x10 | 0x11 | 0x7f => self.status |= 1 << 3,
            _ => self.status |= 1 << 3,
        }
        self.expected = None;
        self.command.clear();
    }

    fn draw_flat_triangle(&mut self, p: &[u32]) {
        let x = [p[0] as i32, p[3] as i32, p[6] as i32];
        let y = [p[1] as i32, p[4] as i32, p[7] as i32];
        let z = [p[2] as i32, p[5] as i32, p[8] as i32];
        let rgb = p[9].to_be_bytes();
        let shade = p[10].min(255) as u16;
        let color = [
            (rgb[1] as u16 * shade / 255) as u8,
            (rgb[2] as u16 * shade / 255) as u8,
            (rgb[3] as u16 * shade / 255) as u8,
            255,
        ];
        let edge = |a: usize, b: usize, px: i32, py: i32| {
            (px - x[a]) as i64 * (y[b] - y[a]) as i64 - (py - y[a]) as i64 * (x[b] - x[a]) as i64
        };
        let area = edge(0, 1, x[2], y[2]);
        if area == 0 {
            return;
        }
        let min_x = x.iter().copied().min().unwrap().max(0) as usize;
        let max_x = x.iter().copied().max().unwrap().min(VPU_WIDTH as i32 - 1);
        let min_y = y.iter().copied().min().unwrap().max(0) as usize;
        let max_y = y.iter().copied().max().unwrap().min(VPU_HEIGHT as i32 - 1);
        if max_x < min_x as i32 || max_y < min_y as i32 {
            return;
        }
        for py in min_y..=max_y as usize {
            for px in min_x..=max_x as usize {
                let px_i = px as i32;
                let py_i = py as i32;
                let w0 = edge(1, 2, px_i, py_i);
                let w1 = edge(2, 0, px_i, py_i);
                let w2 = edge(0, 1, px_i, py_i);
                let inside = if area > 0 {
                    w0 >= 0 && w1 >= 0 && w2 >= 0
                } else {
                    w0 <= 0 && w1 <= 0 && w2 <= 0
                };
                if !inside {
                    continue;
                }
                let d = ((w0 * z[0] as i64 + w1 * z[1] as i64 + w2 * z[2] as i64) / area) as i32;
                let index = py * VPU_WIDTH + px;
                if d < self.depth[index] {
                    self.depth[index] = d;
                    self.plane[index * 4..index * 4 + 4].copy_from_slice(&color);
                }
            }
        }
    }
}
