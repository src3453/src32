// 34SI22: 2ch/2ch SQV4/LPCM sound interface chip (doc/spec_34SI22.md).
//
// An MSM6258-style record/playback interface: two playback channels stream
// PCM8 / PCM16 / SQV4 from memory to the DAC mixer, two record channels
// encode the ADC into memory. The core knows nothing about its host: memory is
// a 24-bit byte-addressed `Si22Memory`, the ADC sample of each 48 kHz tick is
// an argument, and `tick` returns the saturated DAC pair. The CPT32 binding
// (PCM RAM, SGU mixer, OS audio input) lives in sgu.rs.
//
// This file is also included unmodified by the FPGA comparison runner
// (src32-sv sim/sgu_golden), so it depends only on sqv4.rs.

use super::sqv4::{self, State as Sqv4State, Variant as Sqv4Variant};

/// Register window of the chip (6-bit address).
pub const SI22_REG_COUNT: u32 = 0x40;
/// Playback volume, Q15, 4 dB per step, 15 = 0 dB, 0 = mute.
pub const VOL_LUT: [i32; 16] = [
    0, 52, 82, 130, 207, 328, 519, 823, 1305, 2068, 3277, 5193, 8231, 13045, 20675, 32768,
];
/// Record gain, Q8, 4 dB per step, 0 = 0 dB, 15 = +60 dB.
pub const GAIN_LUT: [i32; 16] = [
    256, 406, 643, 1019, 1615, 2560, 4057, 6430, 10192, 16153, 25600, 40573, 64304, 101915, 161525,
    256000,
];
const AGC_HIGH: u32 = 0x5000;
const AGC_LOW: u32 = 0x1400;
const AGC_RELEASE_TICKS: u32 = 9600;
const VOX_HOLD_UNIT: u32 = 1024;
const SQV4_HEADER: u32 = 16;
const SQV4_FRAME_SAMPLES: u32 = sqv4::FRAGMENT_SAMPLES as u32;
const BASE_RATE: u32 = 48_000;
const ADDR_MASK: u32 = 0xFF_FFFF;

pub const ST_STOPPED: u8 = 0;
pub const ST_RUNNING: u8 = 1;
pub const ST_WAITING: u8 = 2;
pub const ST_FINISHED: u8 = 3;

/// External memory seen by the chip (24-bit byte addresses).
pub trait Si22Memory {
    fn read(&mut self, address: u32) -> u8;
    fn write(&mut self, address: u32, value: u8);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Pcm8,
    Pcm16,
    Sqv4(Sqv4Variant),
}

impl Format {
    /// Format field value; 5..7 are reserved and act as PCM8.
    pub fn from_field(value: u8) -> Self {
        match value & 7 {
            1 => Self::Pcm16,
            2 => Self::Sqv4(Sqv4Variant::H),
            3 => Self::Sqv4(Sqv4Variant::L),
            4 => Self::Sqv4(Sqv4Variant::LPlus),
            _ => Self::Pcm8,
        }
    }
}

fn frame_size(variant: Sqv4Variant) -> u32 {
    variant.fragment_size() as u32
}

fn magnitude(sample: i16) -> u32 {
    i32::from(sample).unsigned_abs()
}

fn sat16(value: i32) -> i16 {
    value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

#[derive(Debug, Clone, Copy, Default)]
struct PlaySqv4 {
    variant: Option<Sqv4Variant>,
    total: u32,
    index: u32,
    state: Sqv4State,
    frame_volume: i32,
}

#[derive(Debug, Clone, Default)]
pub struct PlayChannel {
    pub start: u32,
    pub stop: u32,
    pub div: u8,
    pub volume: u8,
    pub l_disable: bool,
    pub r_disable: bool,
    pub format_field: u8,
    pub status: u8,
    format: Format,
    addr: u32,
    end: u32,
    cnt: u8,
    sample: i16,
    header_pending: bool,
    sqv4: PlaySqv4,
}

#[derive(Debug, Clone, Default)]
pub struct RecChannel {
    pub start: u32,
    pub stop: u32,
    pub div: u8,
    pub gain: u8,
    pub format_field: u8,
    pub status: u8,
    auto: bool,
    format: Format,
    base: u32,
    addr: u32,
    end: u32,
    cnt: u8,
    quiet: u32,
    agc: u8,
    release: u32,
    count: u32,
    /// SQV4 samples of the fragment being recorded; it is encoded and written
    /// once complete, because its volume is the fragment's peak.
    fragment: Vec<i16>,
}

/// One 34SI22 chip.
#[derive(Debug, Clone, Default)]
pub struct Si22 {
    pub play: [PlayChannel; 2],
    pub rec: [RecChannel; 2],
    pub agc_enable: [bool; 2],
    /// STATUS_CTRL bit2 (PLAY_ERR) and bits 4..7 (DONE), as register bits.
    pub flags: u8,
    pub vox_threshold: u8,
    pub vox_hold: u8,
    /// Raw ADC sample (L, R) of the last tick: REC_A_ADC / REC_B_ADC.
    pub adc: (i16, i16),
}

fn addr_byte(value: u32, offset: u32) -> u8 {
    (value >> ((2 - offset) * 8)) as u8
}

fn set_addr_byte(value: &mut u32, offset: u32, byte: u8) {
    let shift = (2 - offset) * 8;
    *value = (*value & !(0xFF << shift)) | (u32::from(byte) << shift);
}

impl Si22 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// True while a record channel is armed or recording (the host opens its input then).
    pub fn input_active(&self) -> bool {
        self.rec
            .iter()
            .any(|c| c.status == ST_RUNNING || c.status == ST_WAITING)
    }

    /// Register read. Reads have no side effects.
    pub fn read_register(&self, offset: u32) -> u8 {
        let p = &self.play;
        let r = &self.rec;
        match offset {
            0x00..=0x02 => addr_byte(p[0].start, offset),
            0x03..=0x05 => addr_byte(p[0].stop, offset - 3),
            0x06..=0x08 => addr_byte(p[1].start, offset - 6),
            0x09..=0x0B => addr_byte(p[1].stop, offset - 9),
            0x0C => {
                p[0].status
                    | (p[1].status << 2)
                    | (u8::from(p[0].l_disable) << 4)
                    | (u8::from(p[0].r_disable) << 5)
                    | (u8::from(p[1].l_disable) << 6)
                    | (u8::from(p[1].r_disable) << 7)
            }
            0x0D => p[0].volume | (p[1].volume << 4),
            0x0E => p[0].div,
            0x0F => p[1].div,
            0x10..=0x12 => addr_byte(r[0].start, offset - 0x10),
            0x13..=0x15 => addr_byte(r[0].stop, offset - 0x13),
            0x16..=0x18 => addr_byte(r[1].start, offset - 0x16),
            0x19..=0x1B => addr_byte(r[1].stop, offset - 0x19),
            0x1C => r[0].status | (r[1].status << 2),
            0x1D => r[0].gain | (r[1].gain << 4),
            0x1E => r[0].div,
            0x1F => r[1].div,
            0x20 => u8::from(self.agc_enable[0]) | (u8::from(self.agc_enable[1]) << 1) | self.flags,
            0x21 => p[0].format_field | (p[1].format_field << 4),
            0x22 => r[0].format_field | (r[1].format_field << 4),
            0x23 => self.vox_threshold,
            0x24 => self.vox_hold,
            0x25 => (self.adc.0 >> 8) as u8,
            0x26 => self.adc.0 as u8,
            0x27 => (self.adc.1 >> 8) as u8,
            0x28 => self.adc.1 as u8,
            _ => 0,
        }
    }

    /// Register write. Stopping an SQV4 recording writes its header to `mem`.
    pub fn write_register(&mut self, offset: u32, value: u8, mem: &mut dyn Si22Memory) {
        match offset {
            0x00..=0x02 => set_addr_byte(&mut self.play[0].start, offset, value),
            0x03..=0x05 => set_addr_byte(&mut self.play[0].stop, offset - 3, value),
            0x06..=0x08 => set_addr_byte(&mut self.play[1].start, offset - 6, value),
            0x09..=0x0B => set_addr_byte(&mut self.play[1].stop, offset - 9, value),
            0x0C => {
                for ch in 0..2 {
                    let c = &mut self.play[ch];
                    c.l_disable = value & (0x10 << (2 * ch)) != 0;
                    c.r_disable = value & (0x20 << (2 * ch)) != 0;
                    self.play_control(ch, (value >> (2 * ch)) & 3);
                }
            }
            0x0D => {
                self.play[0].volume = value & 0x0F;
                self.play[1].volume = value >> 4;
            }
            0x0E => self.play[0].div = value,
            0x0F => self.play[1].div = value,
            0x10..=0x12 => set_addr_byte(&mut self.rec[0].start, offset - 0x10, value),
            0x13..=0x15 => set_addr_byte(&mut self.rec[0].stop, offset - 0x13, value),
            0x16..=0x18 => set_addr_byte(&mut self.rec[1].start, offset - 0x16, value),
            0x19..=0x1B => set_addr_byte(&mut self.rec[1].stop, offset - 0x19, value),
            0x1C => {
                for ch in 0..2 {
                    self.rec_control(ch, (value >> (2 * ch)) & 3, mem);
                }
            }
            0x1D => {
                self.rec[0].gain = value & 0x0F;
                self.rec[1].gain = value >> 4;
            }
            0x1E => self.rec[0].div = value,
            0x1F => self.rec[1].div = value,
            0x20 => {
                self.agc_enable = [value & 1 != 0, value & 2 != 0];
                self.flags &= !(value & 0xF4);
            }
            0x21 => {
                self.play[0].format_field = value & 7;
                self.play[1].format_field = (value >> 4) & 7;
            }
            0x22 => {
                self.rec[0].format_field = value & 7;
                self.rec[1].format_field = (value >> 4) & 7;
            }
            0x23 => self.vox_threshold = value,
            0x24 => self.vox_hold = value,
            _ => {}
        }
    }

    fn play_control(&mut self, ch: usize, ctrl: u8) {
        let c = &mut self.play[ch];
        match ctrl {
            0 => c.status = ST_STOPPED,
            1 if c.status == ST_STOPPED || c.status == ST_FINISHED => {
                c.status = ST_RUNNING;
                c.format = Format::from_field(c.format_field);
                c.addr = c.start & ADDR_MASK;
                c.end = c.stop & ADDR_MASK;
                c.cnt = 0;
                c.sample = 0;
                c.header_pending = matches!(c.format, Format::Sqv4(_));
                c.sqv4 = PlaySqv4::default();
            }
            _ => {}
        }
    }

    fn rec_control(&mut self, ch: usize, ctrl: u8, mem: &mut dyn Si22Memory) {
        let status = self.rec[ch].status;
        let idle = status == ST_STOPPED || status == ST_FINISHED;
        match ctrl {
            0 => {
                if !idle {
                    self.rec_close(ch, mem);
                }
                self.rec[ch].status = ST_STOPPED;
            }
            1 | 2 if idle => {
                let c = &mut self.rec[ch];
                c.status = if ctrl == 1 { ST_RUNNING } else { ST_WAITING };
                c.auto = ctrl == 2;
                c.format = Format::from_field(c.format_field);
                c.base = c.start & ADDR_MASK;
                c.end = c.stop & ADDR_MASK;
                c.addr = match c.format {
                    Format::Sqv4(_) => c.base + SQV4_HEADER,
                    _ => c.base,
                };
                c.cnt = 0;
                c.quiet = 0;
                c.agc = c.gain;
                c.release = 0;
                c.count = 0;
            }
            _ => {}
        }
    }

    /// One 48 kHz tick: playback A, B, then record A, B; returns (DAC_L, DAC_R).
    pub fn tick(&mut self, adc: (i16, i16), mem: &mut dyn Si22Memory) -> (i16, i16) {
        self.adc = adc;
        for ch in 0..2 {
            self.play_step(ch, mem);
        }
        self.rec_step(0, adc.0, mem);
        self.rec_step(1, adc.1, mem);
        let (mut left, mut right) = (0i32, 0i32);
        for c in &self.play {
            if c.status != ST_RUNNING {
                continue;
            }
            let y = (i32::from(c.sample) * VOL_LUT[c.volume as usize]) >> 15;
            if !c.l_disable {
                left += y;
            }
            if !c.r_disable {
                right += y;
            }
        }
        (sat16(left), sat16(right))
    }

    fn play_step(&mut self, ch: usize, mem: &mut dyn Si22Memory) {
        let c = &mut self.play[ch];
        if c.status != ST_RUNNING {
            return;
        }
        if c.cnt > 0 {
            c.cnt -= 1;
            return;
        }
        c.cnt = c.div;
        match Self::fetch(c, mem) {
            Ok(sample) => c.sample = sample,
            Err(header_error) => {
                c.status = ST_FINISHED;
                c.sample = 0;
                self.flags |= 0x10 << ch;
                if header_error {
                    self.flags |= 0x04;
                }
            }
        }
    }

    /// Next playback sample; `Err(true)` for an invalid SQV4 header, `Err(false)` at the end.
    fn fetch(c: &mut PlayChannel, mem: &mut dyn Si22Memory) -> Result<i16, bool> {
        let rd = |mem: &mut dyn Si22Memory, a: u32| mem.read(a & ADDR_MASK);
        match c.format {
            Format::Pcm8 => {
                if c.addr >= c.end {
                    return Err(false);
                }
                let b = rd(mem, c.addr);
                c.addr += 1;
                Ok(((i32::from(b) - 128) << 8) as i16)
            }
            Format::Pcm16 => {
                if c.addr + 2 > c.end {
                    return Err(false);
                }
                let s = i16::from_be_bytes([rd(mem, c.addr), rd(mem, c.addr + 1)]);
                c.addr += 2;
                Ok(s)
            }
            Format::Sqv4(_) => {
                let base = c.addr;
                if c.header_pending {
                    c.header_pending = false;
                    if base + SQV4_HEADER > c.end {
                        return Err(true);
                    }
                    let mut h = [0u8; 16];
                    for (i, byte) in h.iter_mut().enumerate() {
                        *byte = rd(mem, base + i as u32);
                    }
                    let variant = Sqv4Variant::try_from(h[5]).ok();
                    if &h[..4] != b"SQV4"
                        || h[4] != sqv4::VERSION
                        || variant.is_none()
                        || h[6] != 1
                        || h[7] != 0
                    {
                        return Err(true);
                    }
                    c.sqv4 = PlaySqv4 {
                        variant,
                        total: u32::from_le_bytes([h[12], h[13], h[14], h[15]]),
                        ..PlaySqv4::default()
                    };
                }
                let s = &mut c.sqv4;
                let variant = s.variant.unwrap();
                let fs = frame_size(variant);
                let frame = s.index / SQV4_FRAME_SAMPLES;
                let within = s.index % SQV4_FRAME_SAMPLES;
                let frame_addr = base + SQV4_HEADER + frame * fs;
                if s.index >= s.total || frame_addr + fs > c.end {
                    return Err(false);
                }
                if within == 0 {
                    // Each fragment restarts the decoder from its own step index.
                    s.frame_volume = i32::from(rd(mem, frame_addr));
                    s.state = Sqv4State::fragment_start(rd(mem, frame_addr + 1));
                }
                let bits = variant.bits() as u32;
                let bit = within * bits;
                let payload = frame_addr + sqv4::FRAGMENT_PREFIX as u32;
                let lo = rd(mem, payload + bit / 8);
                let hi = if (bit % 8) + bits > 8 {
                    rd(mem, payload + bit / 8 + 1)
                } else {
                    0
                };
                let code =
                    (((u16::from(hi) << 8 | u16::from(lo)) >> (bit % 8)) & ((1 << bits) - 1)) as u8;
                let reconstructed = sqv4::decode_code(sqv4::VERSION, variant, &mut s.state, code);
                s.index += 1;
                Ok(sqv4::apply_volume(reconstructed, s.frame_volume) as i16)
            }
        }
    }

    fn rec_step(&mut self, ch: usize, x: i16, mem: &mut dyn Si22Memory) {
        let agc_enable = self.agc_enable[ch];
        let threshold = u32::from(self.vox_threshold) << 7;
        let hold = u32::from(self.vox_hold) * VOX_HOLD_UNIT;
        let c = &mut self.rec[ch];
        if c.status != ST_RUNNING && c.status != ST_WAITING {
            return;
        }
        if c.cnt > 0 {
            c.cnt -= 1;
            return;
        }
        c.cnt = c.div;
        let period = u32::from(c.div) + 1;
        let g = if agc_enable {
            c.agc.min(c.gain)
        } else {
            c.gain
        };
        let y = ((i64::from(x) * i64::from(GAIN_LUT[g as usize])) >> 8).clamp(-32768, 32767) as i16;
        let level = magnitude(y);
        if agc_enable {
            c.agc = c.agc.min(c.gain);
            if level >= AGC_HIGH {
                c.agc = c.agc.saturating_sub(1);
                c.release = 0;
            } else if level < AGC_LOW {
                c.release += period;
                if c.release >= AGC_RELEASE_TICKS {
                    c.release = 0;
                    if c.agc < c.gain {
                        c.agc += 1;
                    }
                }
            } else {
                c.release = 0;
            }
        }
        if c.status == ST_WAITING {
            if level < threshold {
                return;
            }
            c.status = ST_RUNNING;
            c.quiet = 0;
        }
        if !Self::record_sample(c, y, mem) {
            self.finish_rec(ch, mem);
            return;
        }
        let c = &mut self.rec[ch];
        if c.auto && hold != 0 {
            if level >= threshold {
                c.quiet = 0;
            } else {
                c.quiet += period;
            }
            if c.quiet >= hold {
                self.finish_rec(ch, mem);
            }
        }
    }

    /// Store one sample; false when it does not fit before STOP.
    fn record_sample(c: &mut RecChannel, y: i16, mem: &mut dyn Si22Memory) -> bool {
        match c.format {
            Format::Pcm8 => {
                if c.addr + 1 > c.end {
                    return false;
                }
                mem.write(c.addr & ADDR_MASK, ((y >> 8) + 128) as u8);
                c.addr += 1;
            }
            Format::Pcm16 => {
                if c.addr + 2 > c.end {
                    return false;
                }
                let [hi, lo] = y.to_be_bytes();
                mem.write(c.addr & ADDR_MASK, hi);
                mem.write((c.addr + 1) & ADDR_MASK, lo);
                c.addr += 2;
            }
            Format::Sqv4(variant) => {
                // A fragment must fit before its first sample is taken; it is written
                // whole after its last sample (doc/spec_34SI22.md 6.3).
                let within = (c.count % SQV4_FRAME_SAMPLES) as usize;
                if within == 0 && c.addr + frame_size(variant) > c.end {
                    return false;
                }
                if within == 0 {
                    c.fragment.clear();
                }
                c.fragment.push(y);
                c.count += 1;
                if within + 1 == sqv4::FRAGMENT_SAMPLES {
                    Self::write_fragment(c, variant, sqv4::FRAGMENT_SAMPLES, mem);
                }
            }
        }
        true
    }

    fn finish_rec(&mut self, ch: usize, mem: &mut dyn Si22Memory) {
        self.rec_close(ch, mem);
        self.rec[ch].status = ST_FINISHED;
        self.flags |= 0x40 << ch;
    }

    /// Encode the buffered `count` samples as one fragment at `addr` and advance.
    fn write_fragment(
        c: &mut RecChannel,
        variant: Sqv4Variant,
        count: usize,
        mem: &mut dyn Si22Memory,
    ) {
        let mut bytes = Vec::with_capacity(variant.fragment_size());
        sqv4::encode_fragment(variant, &c.fragment[..count], &mut bytes);
        for (i, byte) in bytes.iter().enumerate() {
            mem.write((c.addr + i as u32) & ADDR_MASK, *byte);
        }
        c.addr += bytes.len() as u32;
    }

    /// End of a recording: write the partial SQV4 fragment and the header.
    fn rec_close(&mut self, ch: usize, mem: &mut dyn Si22Memory) {
        let c = &mut self.rec[ch];
        let Format::Sqv4(variant) = c.format else {
            return;
        };
        let partial = (c.count % SQV4_FRAME_SAMPLES) as usize;
        if partial != 0 {
            // unused code slots and padding bits are zero
            Self::write_fragment(c, variant, partial, mem);
        }
        if c.end < c.base + SQV4_HEADER {
            return;
        }
        let rate = BASE_RATE / (u32::from(c.div) + 1);
        let header = sqv4::header_bytes(variant, 1, rate, c.count);
        for (i, byte) in header.iter().enumerate() {
            mem.write((c.base + i as u32) & ADDR_MASK, *byte);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ram(Vec<u8>);
    impl Si22Memory for Ram {
        fn read(&mut self, a: u32) -> u8 {
            self.0[a as usize % self.0.len()]
        }
        fn write(&mut self, a: u32, v: u8) {
            let n = self.0.len();
            self.0[a as usize % n] = v;
        }
    }

    fn set_addr(chip: &mut Si22, reg: u32, value: u32, ram: &mut Ram) {
        for i in 0..3 {
            chip.write_register(reg + i, addr_byte(value, i), ram);
        }
    }

    #[test]
    fn pcm8_and_pcm16_play_once_with_divider_and_stop() {
        let mut ram = Ram(vec![0; 0x1000]);
        ram.0[0x100..0x103].copy_from_slice(&[0x80, 0xC0, 0x40]);
        ram.0[0x200..0x204].copy_from_slice(&[0x12, 0x34, 0xFF, 0xFE]);
        let mut chip = Si22::new();
        set_addr(&mut chip, 0x00, 0x100, &mut ram);
        set_addr(&mut chip, 0x03, 0x103, &mut ram);
        set_addr(&mut chip, 0x06, 0x200, &mut ram);
        set_addr(&mut chip, 0x09, 0x204, &mut ram);
        chip.write_register(0x21, 0x10, &mut ram);
        chip.write_register(0x0D, 0xFF, &mut ram);
        chip.write_register(0x0E, 1, &mut ram);
        // A: L only (R disabled); B: R only (L disabled).
        chip.write_register(0x0C, 0x20 | 0x40 | 0x01 | 0x04, &mut ram);
        let out: Vec<_> = (0..8).map(|_| chip.tick((0, 0), &mut ram)).collect();
        assert_eq!(
            out,
            [
                (0, 0x1234),
                (0, -2),
                (0x4000, 0),
                (0x4000, 0),
                (-0x4000, 0),
                (-0x4000, 0),
                (0, 0),
                (0, 0)
            ]
        );
        assert_eq!(chip.read_register(0x0C) & 0x0F, 0x0F);
        assert_eq!(chip.read_register(0x20), 0x30);
        // Writing back the read value keeps the finished state; W1C clears DONE.
        chip.write_register(0x0C, chip.read_register(0x0C), &mut ram);
        assert_eq!(chip.read_register(0x0C) & 0x0F, 0x0F);
        chip.write_register(0x20, 0x10, &mut ram);
        assert_eq!(chip.read_register(0x20), 0x20);
    }

    #[test]
    fn volume_lut_scales_held_sample() {
        let mut ram = Ram(vec![0x80; 0x100]);
        ram.0[0] = 0xFF;
        let mut chip = Si22::new();
        set_addr(&mut chip, 0x03, 0x10, &mut ram);
        chip.write_register(0x0D, 0x0E, &mut ram);
        chip.write_register(0x0C, 0x01, &mut ram);
        let (l, r) = chip.tick((0, 0), &mut ram);
        let y = ((0x7F00 * 20675) >> 15) as i16;
        assert_eq!((l, r), (y, y));
        chip.write_register(0x0D, 0, &mut ram);
        assert_eq!(chip.tick((0, 0), &mut ram), (0, 0));
    }

    #[test]
    fn sqv4_recording_matches_the_file_encoder_and_plays_back() {
        for (field, variant) in [
            (2, Sqv4Variant::H),
            (3, Sqv4Variant::L),
            (4, Sqv4Variant::LPlus),
        ] {
            let input: Vec<i16> = (0..150)
                .map(|i| ((i * 977) % 20000 - 10000) as i16)
                .collect();
            let mut ram = Ram(vec![0; 0x2000]);
            let mut chip = Si22::new();
            set_addr(&mut chip, 0x10, 0x100, &mut ram);
            set_addr(&mut chip, 0x13, 0x1000, &mut ram);
            chip.write_register(0x22, field, &mut ram);
            chip.write_register(0x1C, 0x01, &mut ram);
            for &x in &input {
                chip.tick((x, 0), &mut ram);
            }
            chip.write_register(0x1C, 0x00, &mut ram);
            let expected = sqv4::encode_interleaved(&input, 1, 48_000, variant, 255).unwrap();
            assert_eq!(&ram.0[0x100..0x100 + expected.len()], &expected[..]);
            assert!(sqv4::inspect(&ram.0[0x100..0x100 + expected.len()]).is_ok());

            // Play it back on channel B at 0 dB: the decoder output, sample for sample.
            let decoded = sqv4::decode(&expected).unwrap().samples;
            set_addr(&mut chip, 0x06, 0x100, &mut ram);
            set_addr(&mut chip, 0x09, 0x100 + expected.len() as u32, &mut ram);
            chip.write_register(0x21, 0x20, &mut ram);
            chip.write_register(0x0D, 0xF0, &mut ram);
            chip.write_register(0x0C, 0x04, &mut ram);
            let played: Vec<i16> = (0..decoded.len())
                .map(|_| chip.tick((0, 0), &mut ram).0)
                .collect();
            assert_eq!(played, decoded);
            chip.tick((0, 0), &mut ram);
            assert_eq!(chip.read_register(0x0C) >> 2 & 3, ST_FINISHED);
        }
    }

    #[test]
    fn sqv4_fragment_is_written_after_its_last_sample() {
        let mut ram = Ram(vec![0xAA; 0x200]);
        let mut chip = Si22::new();
        set_addr(&mut chip, 0x13, 0x100, &mut ram);
        chip.write_register(0x22, 3, &mut ram); // L: 26-byte fragments
        chip.write_register(0x1C, 0x01, &mut ram);
        let input: Vec<i16> = (1..=63).map(|i| i * 300).collect();
        for &x in &input[..62] {
            chip.tick((x, 0), &mut ram);
        }
        assert!(ram.0[16..42].iter().all(|&b| b == 0xAA)); // still buffered
        chip.tick((input[62], 0), &mut ram);
        let mut expected = Vec::new();
        sqv4::encode_fragment(Sqv4Variant::L, &input, &mut expected);
        assert_eq!(&ram.0[16..42], &expected[..]);
        assert_eq!(ram.0[16], sqv4::fragment_volume(&input));
        assert_eq!(ram.0[42], 0xAA);
    }

    #[test]
    fn rec_adc_registers_show_the_raw_sample_without_recording() {
        let mut ram = Ram(vec![0; 0x100]);
        let mut chip = Si22::new();
        chip.write_register(0x1D, 0xFF, &mut ram); // gain does not apply
        chip.write_register(0x1E, 7, &mut ram); // nor the divider
        chip.tick((-2, 0x1234), &mut ram);
        let regs: Vec<u8> = (0x25..=0x29).map(|o| chip.read_register(o)).collect();
        assert_eq!(regs, [0xFF, 0xFE, 0x12, 0x34, 0]);
        chip.write_register(0x25, 0x55, &mut ram); // read only
        chip.tick((i16::MIN, i16::MAX), &mut ram);
        let regs: Vec<u8> = (0x25..=0x28).map(|o| chip.read_register(o)).collect();
        assert_eq!(regs, [0x80, 0x00, 0x7F, 0xFF]);
    }

    #[test]
    fn invalid_sqv4_header_sets_play_err() {
        let mut ram = Ram(vec![0; 0x100]);
        let mut chip = Si22::new();
        set_addr(&mut chip, 0x03, 0x80, &mut ram);
        chip.write_register(0x21, 0x02, &mut ram);
        chip.write_register(0x0C, 0x01, &mut ram);
        chip.tick((0, 0), &mut ram);
        assert_eq!(chip.read_register(0x0C) & 3, ST_FINISHED);
        assert_eq!(chip.read_register(0x20), 0x14);
    }

    #[test]
    fn pcm_recording_stops_at_stop_address_with_gain() {
        let mut ram = Ram(vec![0; 0x100]);
        let mut chip = Si22::new();
        set_addr(&mut chip, 0x16, 0x10, &mut ram);
        set_addr(&mut chip, 0x19, 0x15, &mut ram);
        chip.write_register(0x22, 0x10, &mut ram);
        chip.write_register(0x1D, 0x30, &mut ram);
        chip.write_register(0x1C, 0x04, &mut ram);
        for x in [100, -100, 20000] {
            chip.tick((0, x), &mut ram);
        }
        // 3 samples need 6 bytes; only two fit before 0x15.
        let y0 = ((100 * 1019) >> 8) as i16;
        let y1 = ((-100 * 1019) >> 8) as i16;
        assert_eq!(
            &ram.0[0x10..0x14],
            &[y0.to_be_bytes(), y1.to_be_bytes()].concat()[..]
        );
        assert_eq!(ram.0[0x14], 0);
        assert_eq!(chip.read_register(0x1C) >> 2, ST_FINISHED);
        assert_eq!(chip.read_register(0x20), 0x80);
    }

    #[test]
    fn vox_waits_for_threshold_and_stops_on_silence() {
        let mut ram = Ram(vec![0; 0x10000]);
        let mut chip = Si22::new();
        set_addr(&mut chip, 0x13, 0x8000, &mut ram);
        chip.write_register(0x23, 8, &mut ram); // 1024
        chip.write_register(0x24, 1, &mut ram); // 1024 ticks
        chip.write_register(0x1C, 0x02, &mut ram);
        for _ in 0..10 {
            chip.tick((1000, 0), &mut ram);
        }
        assert_eq!(chip.read_register(0x1C) & 3, ST_WAITING);
        chip.tick((2000, 0), &mut ram);
        assert_eq!(chip.read_register(0x1C) & 3, ST_RUNNING);
        for _ in 0..1023 {
            chip.tick((0, 0), &mut ram);
        }
        assert_eq!(chip.read_register(0x1C) & 3, ST_RUNNING);
        chip.tick((0, 0), &mut ram);
        assert_eq!(chip.read_register(0x1C) & 3, ST_FINISHED);
        assert_eq!(ram.0[0], 0x87); // first recorded sample is the trigger (2000 >> 8 + 128)
        assert_eq!(ram.0[1024], 0x80);
        assert_eq!(ram.0[1025], 0);
    }

    #[test]
    fn agc_attacks_and_releases_within_gain_limit() {
        let mut ram = Ram(vec![0; 0x20000]);
        let mut chip = Si22::new();
        set_addr(&mut chip, 0x13, 0x20000, &mut ram);
        chip.write_register(0x20, 0x01, &mut ram);
        chip.write_register(0x1D, 0x05, &mut ram); // +20 dB limit
        chip.write_register(0x1C, 0x01, &mut ram);
        for _ in 0..4 {
            chip.tick((8000, 0), &mut ram);
        }
        // 8000 * 10 saturates, then each sample drops one step until below 0x5000.
        assert_eq!(chip.rec[0].agc, 2);
        for _ in 0..9600 {
            chip.tick((100, 0), &mut ram);
        }
        assert_eq!(chip.rec[0].agc, 3);
        for _ in 0..9600 * 5 {
            chip.tick((100, 0), &mut ram);
        }
        assert_eq!(chip.rec[0].agc, 5);
    }
}
