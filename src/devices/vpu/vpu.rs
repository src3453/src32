//! FIFO command interpreter and RGBA plane for the VPU.

pub(crate) const VPU_MMIO_BASE: u32 = 0x80030000;
pub(crate) const VPU_MMIO_SIZE: u32 = 0x10000;
pub const VPU_WIDTH: usize = 320;
pub const VPU_HEIGHT: usize = 240;
const FIFO_CAPACITY: usize = 4096;

pub struct Vpu {
    pub(crate) plane: Vec<u8>,
    pub(crate) fifo: Vec<u32>,
    pub(crate) command: Vec<u32>,
    pub(crate) expected: Option<usize>,
    pub(crate) status: u32,
    pub(crate) write_latch: u32,
    pub(crate) write_mask: u8,
}

impl Vpu {
    pub fn new() -> Self {
        Self { plane: vec![0; VPU_WIDTH * VPU_HEIGHT * 4], fifo: Vec::new(), command: Vec::new(), expected: None, status: 0, write_latch: 0, write_mask: 0 }
    }

    pub fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
        if x >= VPU_WIDTH || y >= VPU_HEIGHT { return (0, 0, 0, 0); }
        let i = (y * VPU_WIDTH + x) * 4;
        (self.plane[i], self.plane[i + 1], self.plane[i + 2], self.plane[i + 3])
    }

    pub(crate) fn push(&mut self, word: u32) {
        if self.fifo.len() == FIFO_CAPACITY { self.status |= 1 << 5; return; }
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
                if opcode == 0 { self.status |= 1 << 2; continue; }
                if count > FIFO_CAPACITY { self.status |= 1 << 3; continue; }
                self.command.clear();
                self.command.push(opcode as u32);
                self.expected = Some(count);
                if count == 0 { self.execute(); }
            } else {
                self.command.push(word);
                let left = self.expected.unwrap() - 1;
                self.expected = Some(left);
                if left == 0 { self.execute(); }
            }
        }
        self.status &= !1;
    }

    fn execute(&mut self) {
        let opcode = self.command[0] as u8;
        let p = &self.command[1..];
        match opcode {
            0x04 if p.len() == 3 => {
                let flags = p[0];
                if flags & 1 != 0 {
                    let rgba = p[1].to_be_bytes();
                    for px in self.plane.chunks_exact_mut(4) { px.copy_from_slice(&rgba); }
                }
                if flags & !1 != 0 { self.status |= 1 << 3; }
            }
            // The interpreter deliberately rejects commands it cannot safely consume.
            0x01 | 0x02 | 0x03 | 0x10 | 0x11 | 0x7f => self.status |= 1 << 3,
            _ => self.status |= 1 << 3,
        }
        self.expected = None;
        self.command.clear();
    }

}
