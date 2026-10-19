use crate::{
    envelope::Envelope,
    filter::{Biquad, Master},
    synthesis::synth,
};
pub const PCM_SIZE: usize = 0x400000;
pub const REG_BASE: u32 = 0x400000;

struct Mt {
    state: [u32; 624],
    index: usize,
}
impl Mt {
    fn new() -> Self {
        let mut s = Self {
            state: [0; 624],
            index: 624,
        };
        for i in 1..624 {
            s.state[i] = 1812433253u32
                .wrapping_mul(s.state[i - 1] ^ (s.state[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        s
    }
    fn next(&mut self) -> u32 {
        if self.index == 624 {
            for i in 0..624 {
                let x = (self.state[i] & 0x80000000) | (self.state[(i + 1) % 624] & 0x7fffffff);
                self.state[i] = self.state[(i + 397) % 624]
                    ^ (x >> 1)
                    ^ if x & 1 != 0 { 0x9908b0df } else { 0 };
            }
            self.index = 0;
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c5680;
        y ^= (y << 15) & 0xefc60000;
        y ^= y >> 18;
        y
    }
}

struct Fifo {
    bytes: [u8; 4096],
    head: usize,
    len: usize,
    current: u8,
}
impl Default for Fifo {
    fn default() -> Self {
        Self {
            bytes: [0; 4096],
            head: 0,
            len: 0,
            current: 0,
        }
    }
}
impl Fifo {
    fn push(&mut self, value: u8) {
        if self.len < 4096 {
            self.bytes[(self.head + self.len) % 4096] = value;
            self.len += 1;
        }
    }
    fn pop(&mut self) -> u8 {
        if self.len > 0 {
            self.current = self.bytes[self.head];
            self.head = (self.head + 1) % 4096;
            self.len -= 1;
        }
        self.current
    }
}

pub struct Chip {
    regs: [u8; 1024],
    phases: [[u64; 8]; 8],
    wt_phase: [u64; 4],
    envelopes: [[Envelope; 8]; 8],
    gate: [bool; 8],
    previous: [f32; 8],
    tables: [[i8; 256]; 16],
    noise: [u8; 65536],
    filter: [Biquad; 4],
    master: Master,
    dirty: bool,
    fifo: [Fifo; 4],
    hs_steps: [[u64; 8]; 8],
    wt_steps: [u64; 4],
}
impl Default for Chip {
    fn default() -> Self {
        let mut chip = Self {
            regs: [0; 1024],
            phases: [[0; 8]; 8],
            wt_phase: [0; 4],
            envelopes: [[Envelope::default(); 8]; 8],
            gate: [false; 8],
            previous: [0.0; 8],
            tables: [[0; 256]; 16],
            noise: [0; 65536],
            filter: [Biquad::default(); 4],
            master: Master::default(),
            dirty: true,
            fifo: core::array::from_fn(|_| Fifo::default()),
            hs_steps: [[0; 8]; 8],
            wt_steps: [0; 4],
        };
        let mut mt = Mt::new();
        for value in &mut chip.noise {
            *value = (mt.next() & 1) as u8;
        }
        for i in 0..256 {
            let sine = |p: f32| libm::sin((p * 2.0) as f64 * core::f64::consts::PI) as f32;
            chip.tables[0][i] = (sine(i as f32 / 256.0) * 127.0) as i8;
            chip.tables[1][i] = (sine(i as f32 / 256.0).max(0.0) * 127.0) as i8;
            let triangle = if i < 128 {
                i as f32 / 64.0 - 1.0
            } else {
                i as f32 / -64.0 + 3.0
            };
            chip.tables[4][(i + 64) % 256] = (triangle * 127.0) as i8;
            chip.tables[5][(i + 128) % 256] = ((i as f32 / 128.0 - 1.0) * 127.0) as i8;
            chip.tables[7][(i + 64) % 256] = (triangle.max(0.0) * 127.0) as i8;
            chip.tables[8][(i + 128) % 256] = ((i as f32 / 128.0 - 1.0).max(0.0) * 127.0) as i8;
            chip.tables[9][i] = ((mt.next() & 1) as i32 * 255 - 128) as i8;
            if i < 128 {
                chip.tables[2][i] = (sine(i as f32 / 128.0) * 127.0) as i8;
                chip.tables[3][i] = 127;
                chip.tables[6][i] = 127;
                chip.tables[14][i] = (sine(i as f32 / 128.0).abs() * 127.0) as i8;
                chip.tables[15][i] = chip.tables[14][i];
            } else {
                chip.tables[3][i] = -128;
                chip.tables[15][i] = (-sine(i as f32 / 128.0).abs() * 127.0) as i8;
            }
        }
        chip
    }
}
fn word(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}
fn address(bytes: &[u8]) -> usize {
    ((bytes[0] as usize) << 16) | ((bytes[1] as usize) << 8) | bytes[2] as usize
}
fn period(value: u16) -> u64 {
    if value == 0 { 0 } else { 192000 / value as u64 }
}
/// Q32.32 turns, rounded once when converting the register-defined ratio.
fn hs_step(period: u64, ratio: u16) -> u64 {
    if period == 0 {
        return 0;
    }
    let denominator = period * 4096;
    ((4u64 << 32) * ratio as u64 + denominator / 2) / denominator
}
/// Q32.32 sample positions; 32 PCM/WT positions per oscillator turn.
fn wt_step(period: u64) -> u64 {
    if period == 0 {
        0
    } else {
        ((128u64 << 32) + period / 2) / period
    }
}

impl Chip {
    pub fn read(&self, offset: u32) -> u8 {
        let Some(offset) = offset.checked_sub(REG_BASE).map(|o| o as usize) else {
            return 0;
        };
        if (0x300..0x340).contains(&offset) {
            let fifo = &self.fifo[(offset - 0x300) / 16];
            return match offset % 16 {
                1 => (fifo.len >> 8) as u8,
                2 => fifo.len as u8,
                _ => 0,
            };
        }
        self.regs.get(offset).copied().unwrap_or(0)
    }
    pub fn write(&mut self, offset: u32, value: u8) {
        let Some(offset) = offset
            .checked_sub(REG_BASE)
            .map(|o| o as usize)
            .filter(|&o| o < 1024)
        else {
            return;
        };
        if (0x300..0x340).contains(&offset) {
            if offset % 16 == 0 {
                self.fifo[(offset - 0x300) / 16].push(value);
            }
            return;
        }
        if offset < 0x200 && offset % 64 == 0x1e {
            let ch = offset / 64;
            if value == 1 {
                for env in &mut self.envelopes[ch] {
                    env.trigger();
                }
                self.phases[ch] = [0; 8];
                self.gate[ch] = true;
            } else if value == 0 && self.gate[ch] {
                for env in &mut self.envelopes[ch] {
                    env.release();
                }
                self.gate[ch] = false;
            }
        }
        if (0x200..0x2c0).contains(&offset) && (offset - 0x200) % 48 == 8 {
            self.wt_phase[(offset - 0x200) / 48] = 0;
            return; // write strobe, always reads zero
        }
        self.regs[offset] = value;
        self.dirty = true;
    }
    fn configure(&mut self) {
        if !self.dirty {
            return;
        }
        for ch in 0..8 {
            let r = &self.regs[ch * 64..];
            let period = period(word(r));
            for op in 0..8 {
                self.hs_steps[ch][op] =
                    hs_step(period, if op == 0 { 4096 } else { word(&r[op * 2..]) });
            }
        }
        for ch in 0..4 {
            let r = &self.regs[0x200 + ch * 48..];
            for i in 0..256 {
                let p = r[16 + i / 8] as f32;
                let n = r[16 + (i / 8 + 1) % 32] as f32;
                self.tables[10 + ch][i] = ((p + (n - p) * (i % 8) as f32 / 8.0) as i32 - 128) as i8;
            }
            self.filter[ch].channel(r[4], r[5], r[6]);
            self.wt_steps[ch] = wt_step(period(word(r)));
        }
        self.master.configure(&self.regs[0x2c0..]);
        self.dirty = false;
    }
    pub fn sample(&mut self, pcm: &[u8]) -> [i16; 2] {
        self.configure();
        let mut result = [0.0f32; 12];
        for ch in 0..8 {
            let r = &self.regs[ch * 64..ch * 64 + 64];
            let mut volume = [0.0; 8];
            let mut wave = [0; 8];
            for op in 0..8 {
                // Original generator reads the previous envelope level before update.
                volume[op] =
                    (self.envelopes[ch][op].level * 255.0 * (r[16 + op] as f32 / 255.0) as f64)
                        as i32 as f32
                        / 32768.0;
                self.envelopes[ch][op].advance(&r[32 + op * 4..]);
                self.phases[ch][op] = self.phases[ch][op].wrapping_add(self.hs_steps[ch][op]);
                wave[op] = if op % 2 == 0 {
                    r[24 + op / 2] >> 4
                } else {
                    r[24 + op / 2] & 15
                };
            }
            let feedback = ((self.previous[ch] as f64 / 255.0 / 127.0 + 1.0).clamp(0.0, 2.0) - 1.0)
                as f32
                * (r[31] as f32 / 256.0 - 0.5)
                * 2.0;
            result[ch] = synth(r[28], self.phases[ch], volume, wave, &self.tables, feedback);
            self.previous[ch] = result[ch];
        }
        for ch in 0..4 {
            let r = &self.regs[0x200 + ch * 48..0x200 + (ch + 1) * 48];
            self.wt_phase[ch] = self.wt_phase[ch].wrapping_add(self.wt_steps[ch]);
            let index = self.wt_phase[ch] >> 32;
            let fraction = (self.wt_phase[ch] as u32) as f64 / 4294967296.0;
            let value = match r[3] {
                1 => {
                    let a = r[16 + (index % 32) as usize] as f64;
                    let b = r[16 + ((index + 1) % 32) as usize] as f64;
                    (a + (b - a) * fraction) as i32
                }
                2 => self.noise[(index % 65536) as usize] as i32 * 255,
                3 => self.noise[(index % 64) as usize] as i32 * 255,
                4 => r[16 + (index % 32) as usize] as i32,
                5 => self.fifo[ch].pop() as i32,
                0 => {
                    let start = address(&r[16..]);
                    let end = address(&r[19..]);
                    let loop_start = address(&r[22..]);
                    let at = |i: u64| {
                        // Retain legacy loop addressing (start + phase % loop length).
                        let pos = if start as u64 + index > end as u64
                            && loop_start < end
                            && loop_start != 0xffffff
                        {
                            start as u64 + i % (end - loop_start) as u64
                        } else {
                            (start as u64 + i).min(end as u64)
                        };
                        pcm.get(pos as usize).copied().unwrap_or(0) as f64
                    };
                    let a = at(index);
                    (a + (at(index + 1) - a) * fraction) as i32
                }
                _ => 0,
            } - 128;
            let filtered = self.filter[ch].process(value as f32);
            let gain = r[2] as f32 / 255.0;
            result[ch + 8] = if r[5] == 0 {
                value as f32 * 255.0 * gain
            } else {
                (filtered * 255.0 * gain).clamp(-32768.0, 32767.0)
            };
        }
        let mut mix = [0.0; 2];
        for ch in 0..12 {
            // Original '!mute == 1' means only zero is audible.
            if self.regs[0x2d0 + ch] != 0 {
                continue;
            }
            let pan = if ch < 8 {
                self.regs[ch * 64 + 29]
            } else {
                self.regs[0x200 + (ch - 8) * 48 + 7]
            };
            let pan = if pan == 0 { 255 } else { pan };
            let gain = result[ch] / 32768.0 * if ch >= 8 { 1.3 } else { 1.0 };
            mix[0] += gain * (pan >> 4) as f32 / 15.0;
            mix[1] += gain * (pan & 15) as f32 / 15.0;
        }
        self.master.process(mix, &self.regs[0x2c0..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_phase_carry_and_wrap_match_integer_model_over_a_long_run() {
        let mut chip = Chip::default();
        chip.write(REG_BASE, 1);
        chip.write(REG_BASE + 1, 184); // 440 Hz register
        chip.write(REG_BASE + 2, 0x17);
        chip.write(REG_BASE + 3, 0xab);
        chip.configure();
        let steps = chip.hs_steps[0];
        assert_eq!(steps[0], ((4u64 << 32) + 218) / 436);
        assert_eq!(
            steps[1],
            ((4u64 << 32) * 0x17ab + 436 * 2048) / (436 * 4096)
        );
        // Skip ahead by integer arithmetic, then exercise overflow in sample().
        let count = 48_000u64 * 60 * 60 * 24 * 365;
        chip.phases[0][0] = steps[0].wrapping_mul(count);
        chip.wt_phase[0] = u64::MAX - 5;
        chip.write(REG_BASE + 0x200, 1);
        chip.write(REG_BASE + 0x201, 184);
        chip.configure();
        let wt = chip.wt_steps[0];
        for n in 1..=256 {
            chip.sample(&[]);
            assert_eq!(chip.phases[0][0], steps[0].wrapping_mul(count + n));
            assert_eq!(chip.wt_phase[0], (u64::MAX - 5).wrapping_add(wt * n));
        }
        assert_eq!(hs_step(0, 65535), 0);
        assert_eq!(wt_step(0), 0);
    }
}
