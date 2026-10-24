// 3WS8PN / 3WS8PNA (S3W2) Sound Generator Core Implementation
//
// 3WS8PN is a wavetable/PCM/noise sound generator chip with 8 channels.
// SGU v2.1 (3WS8PNA) adds 2-tap linear interpolation and a 16-bit FIFO DAC;
// `SguVersion` selects v1 / v2 / v2.1 behavior (doc/spec_SGU.md section 8).
// Master Output: 16-bit Stereo Linear PCM, 48kHz
// Sound clock: 192kHz (48MHz / 250)

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use super::sqv4::{self, State as Sqv4State, Variant as Sqv4Variant};

pub const NUM_CHANNELS: usize = 8;
const SGU_MASTER_GAIN_DIVISOR: i32 = 4;
pub const WAVETABLE_SIZE: usize = 256;
pub const PCM_RAM_SIZE: usize = 1024 * 1024; // 1MB
pub const SAMPLE_RATE: usize = 48000;
pub const SOUND_CLOCK: u32 = 192_000;
pub const SGU_CLOCK_DIVIDER: u32 = 250; // 48MHz / 250 = 192kHz
/// Per-channel FIFO DAC capacity in 16-bit samples (v2.1).
pub const FIFO_CAPACITY: usize = 2048;

/// SGU hardware generation. `V2` is the pre-3WS8PNA behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SguVersion {
    /// A single SGU core (SGU 1 is absent); no interpolation.
    V1,
    /// Two SGU cores; Wavetable/PCM are not interpolated.
    V2,
    /// 3WS8PNA: 2-tap linear interpolation and the FIFO DAC.
    #[default]
    V2_1,
}

impl SguVersion {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().trim_start_matches(['v', 'V']) {
            "1" | "1.0" => Some(Self::V1),
            "2" | "2.0" => Some(Self::V2),
            "2.1" => Some(Self::V2_1),
            _ => None,
        }
    }

    /// Number of SGU cores mapped on the bus.
    pub fn core_count(self) -> usize {
        if self == Self::V1 { 1 } else { 2 }
    }

    fn has_3ws8pna(self) -> bool {
        self == Self::V2_1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveformType {
    Wavetable = 0,
    Pcm = 1,
    Noise = 2,
    DmaPcm = 3,
    Sqv4 = 4,
    /// 16-bit 48 kHz FIFO DAC (SGU v2.1).
    FifoDac = 5,
}

impl From<u8> for WaveformType {
    fn from(value: u8) -> Self {
        match value {
            0 => WaveformType::Wavetable,
            1 => WaveformType::Pcm,
            2 => WaveformType::Noise,
            3 => WaveformType::DmaPcm,
            4 => WaveformType::Sqv4,
            5 => WaveformType::FifoDac,
            _ => WaveformType::Wavetable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModulationType {
    None = 0,
    Phase = 1,
    Ring = 2,
    HardSync = 3,
    Window = 4,
    SlewRate = 5,
    SlewStep = 6,
}

impl From<u8> for ModulationType {
    fn from(value: u8) -> Self {
        match value {
            1 => ModulationType::Phase,
            2 => ModulationType::Ring,
            3 => ModulationType::HardSync,
            4 => ModulationType::Window,
            5 => ModulationType::SlewRate,
            6 => ModulationType::SlewStep,
            _ => ModulationType::None,
        }
    }
}

#[derive(Clone)]
pub struct Channel {
    // Register data
    pub frequency: u16,
    pub waveform_type: WaveformType,
    pub volume: u8,
    pub panpot: u8,
    pub modulation_type: ModulationType,
    pub modulation_target: u8,
    pub modulation_targeting_mode: u8,
    pub modulation_param_1: u16,
    pub modulation_param_2: u16,

    // PCM registers
    pub pcm_start_addr: u32,
    pub pcm_end_addr: u32,
    pub pcm_loop_addr: u32,
    pub pcm_control: u8,

    // Incremental SQV4 decode state; only one 64-sample frame is cached.
    sqv4_validated: bool,
    sqv4_version: u8,
    sqv4_variant: Option<Sqv4Variant>,
    sqv4_sample_count: usize,
    sqv4_frame_size: usize,
    sqv4_loop_sample: usize,
    sqv4_cached_frame: Option<usize>,
    sqv4_next_frame: usize,
    sqv4_decoder: Sqv4State,
    sqv4_frame: [i16; 64],

    // Internal state
    pub old_phase: u64,
    pub lfsr_state: u32,
    pub phase: f64,
    pub last_sample: i16,
    pub active: bool,
    slew_current: i32,

    // FIFO DAC (v2.1): queued samples, last output, remaining hold ticks and
    // the 192 kHz sub-tick (the DAC advances once per 48 kHz output sample).
    pub fifo: VecDeque<i16>,
    fifo_held: i16,
    fifo_hold: u32,
    fifo_sub: u8,
    fifo_latch: u8,

    // Wavetable data (256 bytes)
    pub wavetable: [u8; WAVETABLE_SIZE],
}

impl Default for Channel {
    fn default() -> Self {
        let mut ch = Self {
            frequency: 0,
            waveform_type: WaveformType::Wavetable,
            volume: 0,
            panpot: 0xFF,
            modulation_type: ModulationType::None,
            modulation_target: 0,
            modulation_targeting_mode: 0,
            modulation_param_1: 0,
            modulation_param_2: 0,
            pcm_start_addr: 0,
            pcm_end_addr: 0,
            pcm_loop_addr: 0,
            pcm_control: 0,
            sqv4_validated: false,
            sqv4_version: 0,
            sqv4_variant: None,
            sqv4_sample_count: 0,
            sqv4_frame_size: 0,
            sqv4_loop_sample: 0,
            sqv4_cached_frame: None,
            sqv4_next_frame: 0,
            sqv4_decoder: Sqv4State::default(),
            sqv4_frame: [0; 64],
            old_phase: 0,
            lfsr_state: 0x12D4_803C,
            phase: 0.0,
            last_sample: 0,
            active: true,
            slew_current: 0,
            fifo: VecDeque::new(),
            fifo_held: 0,
            fifo_hold: 0,
            fifo_sub: 0,
            fifo_latch: 0,
            wavetable: [0x80; WAVETABLE_SIZE],
        };
        ch.reset();
        ch
    }
}

impl Channel {
    pub fn reset(&mut self) {
        self.frequency = 0;
        self.waveform_type = WaveformType::Wavetable;
        self.volume = 0;
        self.panpot = 0xFF;
        self.modulation_type = ModulationType::None;
        self.modulation_target = 0;
        self.modulation_targeting_mode = 0;
        self.modulation_param_1 = 0;
        self.modulation_param_2 = 0;
        self.pcm_start_addr = 0;
        self.pcm_end_addr = 0;
        self.pcm_loop_addr = 0;
        self.pcm_control = 0;
        self.invalidate_sqv4();
        self.old_phase = 0;
        self.phase = 0.0;
        self.last_sample = 0;
        self.lfsr_state = 0x12D4_803C;
        self.active = true;
        self.slew_current = 0;
        self.reset_fifo();
        self.wavetable.fill(0x80);
    }

    fn reset_fifo(&mut self) {
        self.fifo.clear();
        self.fifo_held = 0;
        self.fifo_hold = 0;
        self.fifo_sub = 0;
        self.fifo_latch = 0;
    }
    // Unsigned Q8.8 accumulator, updated at the 192 kHz internal clock.
    fn apply_slew(&mut self, sample: u8) -> u8 {
        (self.apply_slew_q8(i32::from(sample) << 8) >> 8) as u8
    }

    /// Slew on a Q8.8 input; returns the new Q8.8 state (or the input when
    /// no Slew mode is selected).
    fn apply_slew_q8(&mut self, sample_q8: i32) -> i32 {
        let difference = sample_q8 - self.slew_current;
        let step = i32::from(self.modulation_param_1);
        self.slew_current = match self.modulation_type {
            ModulationType::SlewRate => self.slew_current + difference.clamp(-step, step),
            ModulationType::SlewStep => {
                self.slew_current + if difference > 0 { step } else { -step }
            }
            _ => return sample_q8,
        }
        .clamp(0, 255 << 8);
        self.slew_current
    }

    fn invalidate_sqv4(&mut self) {
        self.sqv4_validated = false;
        self.sqv4_version = 0;
        self.sqv4_variant = None;
        self.sqv4_sample_count = 0;
        self.sqv4_frame_size = 0;
        self.sqv4_loop_sample = 0;
        self.sqv4_cached_frame = None;
        self.sqv4_next_frame = 0;
        self.sqv4_decoder = Sqv4State::default();
        self.sqv4_frame.fill(0);
    }
}

pub struct S3w2Sound {
    pub version: SguVersion,
    pub channels: [Channel; NUM_CHANNELS],
    pub pcm_ram: Rc<RefCell<Vec<u8>>>,
}

fn apply_master_gain(sample: i16) -> i16 {
    (i32::from(sample) / 2 / SGU_MASTER_GAIN_DIVISOR) as i16
}

impl Default for S3w2Sound {
    fn default() -> Self {
        Self::new()
    }
}

impl S3w2Sound {
    pub fn new() -> Self {
        Self::with_pcm_ram(Rc::new(RefCell::new(vec![0; PCM_RAM_SIZE])))
    }

    pub fn with_pcm_ram(pcm_ram: Rc<RefCell<Vec<u8>>>) -> Self {
        Self::with_version(pcm_ram, SguVersion::default())
    }

    pub fn with_version(pcm_ram: Rc<RefCell<Vec<u8>>>, version: SguVersion) -> Self {
        let mut sound = Self {
            version,
            channels: std::array::from_fn(|_| Channel::default()),
            pcm_ram,
        };
        sound.reset();
        sound
    }

    /// Select the hardware generation. Registers keep their values, but
    /// features the new version lacks stop taking effect on the next write.
    pub fn set_version(&mut self, version: SguVersion) {
        self.version = version;
        for c in &mut self.channels {
            if !version.has_3ws8pna() && c.waveform_type == WaveformType::FifoDac {
                c.waveform_type = WaveformType::Wavetable;
            }
            c.reset_fifo();
        }
    }

    pub fn reset(&mut self) {
        self.pcm_ram.borrow_mut().fill(0);
        for ch in 0..NUM_CHANNELS {
            self.reset_channel(ch);
        }
    }

    pub fn reset_channel(&mut self, ch: usize) {
        if ch < NUM_CHANNELS {
            self.channels[ch].reset();
        }
    }

    pub fn write_register(&mut self, address: u32, value: u8) {
        if address <= 0x7FF {
            let ch = ((address >> 8) & 0x7) as usize;
            let offset = (address & 0xFF) as u8;
            self.write_channel_wavetable(ch, offset, value);
        } else if (0x800..=0x8FF).contains(&address) {
            let ch = (((address - 0x800) >> 5) & 0x7) as usize;
            let offset = ((address - 0x800) & 0x1F) as u8;
            if offset < 0x10 {
                self.write_channel_control(ch, offset, value);
            } else {
                self.write_channel_pcm_reg(ch, offset - 0x10, value);
            }
        }
    }

    pub fn read_register(&mut self, address: u32) -> u8 {
        if address <= 0x7FF {
            let ch = ((address >> 8) & 0x7) as usize;
            let offset = (address & 0xFF) as u8;
            self.read_channel_wavetable(ch, offset)
        } else if (0x800..=0x8FF).contains(&address) {
            let ch = (((address - 0x800) >> 5) & 0x7) as usize;
            let offset = ((address - 0x800) & 0x1F) as u8;
            self.read_channel_control(ch, offset)
        } else {
            0
        }
    }

    /// Return the visible register value without applying read side effects.
    pub fn peek_register(&self, address: u32) -> u8 {
        if address <= 0x7FF {
            let ch = ((address >> 8) & 0x7) as usize;
            let offset = (address & 0xFF) as u8;
            self.read_channel_wavetable(ch, offset)
        } else if (0x800..=0x8FF).contains(&address) {
            let ch = (((address - 0x800) >> 5) & 0x7) as usize;
            let offset = ((address - 0x800) & 0x1F) as u8;
            match offset {
                0..=9 => {
                    let c = &self.channels[ch];
                    match offset {
                        0 => (c.frequency >> 8) as u8,
                        1 => c.frequency as u8,
                        2 => c.waveform_type as u8,
                        3 => c.volume,
                        4 => c.panpot,
                        5 => ((c.modulation_type as u8) << 3) | (c.modulation_target & 7),
                        6..=7 => (c.modulation_param_1 >> ((7 - offset) * 8)) as u8,
                        8..=9 => (c.modulation_param_2 >> ((9 - offset) * 8)) as u8,
                        _ => 0,
                    }
                }
                0x0B => self.channels[ch].modulation_targeting_mode & 1,
                0x0E | 0x0F if self.version.has_3ws8pna() => {
                    (self.channels[ch].fifo.len() as u16 >> ((0x0F - offset) * 8)) as u8
                }
                0x10..=0x19 => self.read_channel_pcm_reg(ch, offset - 0x10),
                _ => 0,
            }
        } else {
            0
        }
    }

    pub fn write_pcm_ram(&mut self, address: u32, value: u8) {
        let addr = address as usize;
        let mut pcm_ram = self.pcm_ram.borrow_mut();
        if addr < pcm_ram.len() {
            pcm_ram[addr] = value;
            for channel in &mut self.channels {
                if channel.waveform_type == WaveformType::Sqv4
                    && (addr as u32) >= channel.pcm_start_addr
                    && (addr as u32) < channel.pcm_end_addr
                {
                    channel.invalidate_sqv4();
                }
            }
        }
    }

    pub fn read_pcm_ram(&self, address: u32) -> u8 {
        let addr = address as usize;
        let pcm_ram = self.pcm_ram.borrow();
        if addr < pcm_ram.len() {
            pcm_ram[addr]
        } else {
            0
        }
    }

    pub fn write_channel_wavetable(&mut self, ch: usize, offset: u8, value: u8) {
        if ch < NUM_CHANNELS {
            let offset = offset as usize;
            if offset < WAVETABLE_SIZE {
                self.channels[ch].wavetable[offset] = value;
            }
        }
    }

    pub fn read_channel_wavetable(&self, ch: usize, offset: u8) -> u8 {
        if ch < NUM_CHANNELS {
            let offset = offset as usize;
            if offset < WAVETABLE_SIZE {
                return self.channels[ch].wavetable[offset];
            }
        }
        0
    }

    pub fn write_channel_control(&mut self, ch: usize, offset: u8, value: u8) {
        if ch >= NUM_CHANNELS {
            return;
        }
        let version = self.version;
        let c = &mut self.channels[ch];
        match offset {
            0 => {
                c.frequency = (c.frequency & 0x00FF) | ((value as u16) << 8);
            }
            1 => {
                c.frequency = (c.frequency & 0xFF00) | (value as u16);
            }
            2 => {
                c.waveform_type = match WaveformType::from(value) {
                    WaveformType::FifoDac if !version.has_3ws8pna() => WaveformType::Wavetable,
                    other => other,
                };
                c.invalidate_sqv4();
            }
            3 => {
                c.volume = value;
            }
            4 => {
                c.panpot = value;
            }
            5 => {
                c.modulation_type = ModulationType::from(value >> 3);
                c.modulation_target = value & 0x07;
            }
            6..=9 => {
                let shift_1 = (7 - (offset as i32)) * 8;
                let shift_2 = (9 - (offset as i32)) * 8;
                if (6..=7).contains(&offset) {
                    c.modulation_param_1 = (c.modulation_param_1 & !(0xFFu16 << shift_1))
                        | ((value as u16) << shift_1);
                }
                if (8..=9).contains(&offset) {
                    c.modulation_param_2 = (c.modulation_param_2 & !(0xFFu16 << shift_2))
                        | ((value as u16) << shift_2);
                }
            }
            0x0A => {
                c.phase = 0.0;
                c.slew_current = 0;
                c.invalidate_sqv4();
                c.lfsr_state = 0x12D4_803C;
                c.reset_fifo();
            }
            0x0B => {
                c.modulation_targeting_mode = value & 0x01;
            }
            // FIFO_DATA: high byte is latched, the low byte pushes the sample.
            // A write to a full FIFO is dropped.
            0x0C if version.has_3ws8pna() => c.fifo_latch = value,
            0x0D if version.has_3ws8pna() => {
                if c.fifo.len() < FIFO_CAPACITY {
                    c.fifo
                        .push_back(i16::from_be_bytes([c.fifo_latch, value]));
                }
            }
            _ => {}
        }
    }

    pub fn read_channel_control(&mut self, ch: usize, offset: u8) -> u8 {
        if ch >= NUM_CHANNELS {
            return 0;
        }
        let c = &mut self.channels[ch];
        match offset {
            0 => ((c.frequency >> 8) & 0xFF) as u8,
            1 => (c.frequency & 0xFF) as u8,
            2 => c.waveform_type as u8,
            3 => c.volume,
            4 => c.panpot,
            5 => ((c.modulation_type as u8) << 3) | (c.modulation_target & 0x07),
            6..=9 => {
                if offset <= 7 {
                    let shift = (7 - (offset as i32)) * 8;
                    ((c.modulation_param_1 >> shift) & 0xFF) as u8
                } else {
                    let shift = (9 - (offset as i32)) * 8;
                    ((c.modulation_param_2 >> shift) & 0xFF) as u8
                }
            }
            0x0A => {
                c.phase = 0.0;
                c.invalidate_sqv4();
                0
            }
            0x0B => c.modulation_targeting_mode & 0x01,
            0x0E | 0x0F if self.version.has_3ws8pna() => {
                (c.fifo.len() as u16 >> ((0x0F - offset) * 8)) as u8
            }
            0x10..=0x19 => {
                // PCM registers are at offset 0x10-0x19
                let pcm_offset = offset - 0x10;
                self.read_channel_pcm_reg(ch, pcm_offset)
            }
            _ => 0,
        }
    }

    pub fn write_channel_pcm_reg(&mut self, ch: usize, offset: u8, value: u8) {
        if ch >= NUM_CHANNELS {
            return;
        }
        if offset <= 8 || offset == 9 {
            self.channels[ch].invalidate_sqv4();
        }
        let c = &mut self.channels[ch];
        match offset {
            0..=2 => {
                let shift = (2 - (offset as i32)) * 8;
                let mask = 0xFFu32 << shift;
                let v = (value as u32) << shift;
                c.pcm_start_addr = (c.pcm_start_addr & !mask) | v;
            }
            3..=5 => {
                let shift = (5 - (offset as i32)) * 8;
                let mask = 0xFFu32 << shift;
                let v = (value as u32) << shift;
                c.pcm_end_addr = (c.pcm_end_addr & !mask) | v;
            }
            6..=8 => {
                let shift = (8 - (offset as i32)) * 8;
                let mask = 0xFFu32 << shift;
                let v = (value as u32) << shift;
                c.pcm_loop_addr = (c.pcm_loop_addr & !mask) | v;
            }
            9 => {
                c.pcm_control = value;
                if (value & 0x01) != 0 {
                    c.active = true;
                } else {
                    c.active = false;
                }
            }
            _ => {}
        }
    }

    pub fn read_channel_pcm_reg(&self, ch: usize, offset: u8) -> u8 {
        if ch >= NUM_CHANNELS {
            return 0;
        }
        let c = &self.channels[ch];
        match offset {
            0..=2 => {
                let shift = (2 - (offset as i32)) * 8;
                ((c.pcm_start_addr >> shift) & 0xFF) as u8
            }
            3..=5 => {
                let shift = (5 - (offset as i32)) * 8;
                ((c.pcm_end_addr >> shift) & 0xFF) as u8
            }
            6..=8 => {
                let shift = (8 - (offset as i32)) * 8;
                ((c.pcm_loop_addr >> shift) & 0xFF) as u8
            }
            9 => c.pcm_control,
            _ => 0,
        }
    }

    pub fn convert_to_absolute_channel_address(
        carrier_channel: u8,
        modulation_targeting_mode: u8,
        modulation_target: u8,
    ) -> usize {
        if modulation_targeting_mode == 0 {
            (modulation_target & 0x07) as usize
        } else {
            let mut relative_offset = (modulation_target & 0x07) as i8;
            if relative_offset >= 4 {
                relative_offset -= 8;
            }
            let mut absolute_channel = (carrier_channel as i32) + (relative_offset as i32);
            let num_ch = NUM_CHANNELS as i32;
            if absolute_channel < 0 {
                absolute_channel += num_ch;
            } else if absolute_channel >= num_ch {
                absolute_channel -= num_ch;
            }
            (absolute_channel & 0x07) as usize
        }
    }

    /// One 192 kHz internal-clock sample. Wavetable/PCM/Noise/Slew are
    /// computed in signed Q8.8 (centered on the 0x80 silence level) so v2.1
    /// interpolation keeps its fraction until the volume stage.
    pub fn generate_sample(&mut self, ch: usize) -> i16 {
        if ch >= NUM_CHANNELS {
            return 0;
        }
        if !self.channels[ch].active {
            return 0;
        }

        let waveform = self.channels[ch].waveform_type;
        let q8 = match waveform {
            WaveformType::Wavetable => self.generate_wavetable_sample(ch),
            WaveformType::Pcm => self.generate_pcm_sample(ch),
            WaveformType::Noise => i32::from(self.generate_noise_sample(ch)) << 8,
            WaveformType::DmaPcm => i32::from(self.generate_dma_pcm_sample(ch)) << 8,
            WaveformType::Sqv4 => i32::from(self.generate_sqv4_sample(ch)) << 8,
            WaveformType::FifoDac => {
                // 16-bit sample, fixed gain: VOLUME does not apply.
                let sample = self.generate_fifo_sample(ch);
                self.channels[ch].last_sample = sample >> 8;
                return sample;
            }
        };

        self.channels[ch].last_sample = (q8 >> 8) as i16;
        let volume = self.channels[ch].volume as i32;
        if waveform == WaveformType::Sqv4 {
            ((q8 >> 8) * volume / 255) as i16
        } else {
            (q8 * volume / (4 * 256)) as i16
        }
    }

    /// Fraction of `phase * 256` in 1/256 steps and its integer part.
    fn split_phase(phase: f64) -> (u64, i32) {
        let scaled = phase * 256.0;
        let whole = scaled as u64;
        (whole, (((scaled - whole as f64) * 256.0) as i32).clamp(0, 255))
    }

    fn generate_wavetable_sample(&mut self, ch: usize) -> i32 {
        let interpolate = self.version.has_3ws8pna();
        let abs_target_ch = Self::convert_to_absolute_channel_address(
            ch as u8,
            self.channels[ch].modulation_targeting_mode,
            self.channels[ch].modulation_target,
        );
        let target_last_sample = self.channels[abs_target_ch].last_sample;
        let target_phase = self.channels[abs_target_ch].phase;

        let c = &mut self.channels[ch];
        c.phase += (c.frequency as f64) / (SOUND_CLOCK as f64);
        let (phase, frac) = Self::split_phase(c.phase);

        let index = match c.modulation_type {
            ModulationType::Phase => {
                let phase_offset =
                    ((c.modulation_param_1 as i32 * target_last_sample as i32) >> 12) as i64;
                ((phase.wrapping_add(phase_offset as u64)) & 0xFF) as usize
            }
            _ => (phase & 0xFF) as usize,
        };

        let current = i32::from(c.wavetable[index]);
        let mut q8 = if interpolate {
            let next = i32::from(c.wavetable[(index + 1) & 0xFF]);
            current * (256 - frac) + next * frac
        } else {
            current << 8
        };

        if c.modulation_type == ModulationType::Ring {
            q8 = Self::ring(q8, target_last_sample, c.modulation_param_1, interpolate);
        } else if c.modulation_type == ModulationType::HardSync {
            if target_phase < 1.0 {
                c.phase = 0.0;
                q8 = i32::from(c.wavetable[0]) << 8;
            }
        } else if c.modulation_type == ModulationType::Window {
            if target_last_sample < 0 {
                q8 = 128 << 8;
            }
        }

        Self::finish_unsigned(c, q8, interpolate)
    }

    fn ring(q8: i32, modulator: i16, param: u16, interpolate: bool) -> i32 {
        let scale = i64::from(modulator) * i64::from(param);
        if interpolate {
            ((i64::from(q8 - (128 << 8)) * scale) / 65536 + (128 << 8)).clamp(0, 255 << 8) as i32
        } else {
            // Integer-sample arithmetic of v1/v2.
            let original = i64::from(q8 >> 8) - 128;
            (((original * scale) / 65536 + 128).clamp(0, 255) as i32) << 8
        }
    }

    /// Slew, then re-center the unsigned Q8.8 sample. v1/v2 truncate to whole
    /// sample steps after Slew.
    fn finish_unsigned(c: &mut Channel, q8: i32, interpolate: bool) -> i32 {
        let mut out = c.apply_slew_q8(q8);
        if !interpolate {
            out &= !0xFF;
        }
        out - (128 << 8)
    }

    fn generate_pcm_sample(&mut self, ch: usize) -> i32 {
        let interpolate = self.version.has_3ws8pna();
        let abs_target_ch = Self::convert_to_absolute_channel_address(
            ch as u8,
            self.channels[ch].modulation_targeting_mode,
            self.channels[ch].modulation_target,
        );
        let target_last_sample = self.channels[abs_target_ch].last_sample;

        let c = &mut self.channels[ch];
        let (mut phase, frac) = Self::split_phase(c.phase);

        if c.modulation_type == ModulationType::Phase {
            let phase_offset =
                ((c.modulation_param_1 as i32 * target_last_sample as i32) >> 12) as i64;
            phase = phase.wrapping_add(phase_offset as u64);
        }

        let addr = ((phase + c.pcm_start_addr as u64) as usize) & (PCM_RAM_SIZE - 1);
        let current = i32::from(self.pcm_ram.borrow()[addr]);
        let q8 = if interpolate {
            let c = &self.channels[ch];
            // The tap after the last sample is the loop start (looping) or
            // the last sample itself (one-shot hold).
            let next_addr = addr + 1;
            let next = if (next_addr as u32) < c.pcm_end_addr {
                next_addr & (PCM_RAM_SIZE - 1)
            } else if (c.pcm_control & 0x02) != 0 {
                (c.pcm_loop_addr as usize) & (PCM_RAM_SIZE - 1)
            } else {
                addr
            };
            let next = i32::from(self.pcm_ram.borrow()[next]);
            current * (256 - frac) + next * frac
        } else {
            current << 8
        };
        let mut out = Self::finish_unsigned(&mut self.channels[ch], q8, interpolate);

        let c = &mut self.channels[ch];
        if (addr as u32) >= c.pcm_end_addr {
            if (c.pcm_control & 0x02) != 0 {
                // Loop on
                c.phase = (c.pcm_loop_addr.saturating_sub(c.pcm_start_addr) as f64) / 256.0;
            } else {
                c.phase = (c.pcm_end_addr.saturating_sub(c.pcm_start_addr) as f64) / 256.0;
                out = 0;
            }
        } else {
            c.phase += (c.frequency as f64) / (SOUND_CLOCK as f64) / 8.0;
        }

        out
    }

    /// FIFO DAC: one 48 kHz output step per four internal ticks. A new sample
    /// is taken every FREQUENCY + 1 output steps; an empty FIFO holds the last
    /// value and retries on the next step.
    fn generate_fifo_sample(&mut self, ch: usize) -> i16 {
        let c = &mut self.channels[ch];
        if c.fifo_sub == 0 {
            if c.fifo_hold == 0 {
                if let Some(sample) = c.fifo.pop_front() {
                    c.fifo_held = sample;
                    c.fifo_hold = u32::from(c.frequency);
                }
            } else {
                c.fifo_hold -= 1;
            }
        }
        c.fifo_sub = (c.fifo_sub + 1) & 3;
        c.fifo_held
    }

    fn initialize_sqv4(&mut self, ch: usize) -> bool {
        let start = self.channels[ch].pcm_start_addr as usize;
        let end = self.channels[ch].pcm_end_addr as usize;
        let pcm_ram = self.pcm_ram.borrow();
        if start >= end || end > pcm_ram.len() {
            self.channels[ch].active = false;
            self.channels[ch].invalidate_sqv4();
            return false;
        }
        let header = match sqv4::inspect(&pcm_ram[start..end]) {
            Ok(header) if header.channels == 1 => header,
            _ => {
                self.channels[ch].active = false;
                self.channels[ch].invalidate_sqv4();
                return false;
            }
        };
        let loop_address = self.channels[ch].pcm_loop_addr as usize;
        let loop_sample = if loop_address == 0 || loop_address == start {
            0
        } else {
            let first_fragment = match start.checked_add(16) {
                Some(address) => address,
                None => {
                    self.channels[ch].active = false;
                    self.channels[ch].invalidate_sqv4();
                    return false;
                }
            };
            if loop_address < first_fragment {
                self.channels[ch].active = false;
                self.channels[ch].invalidate_sqv4();
                return false;
            }
            let fragment_offset = loop_address - first_fragment;
            if fragment_offset % header.frame_size != 0 {
                self.channels[ch].active = false;
                self.channels[ch].invalidate_sqv4();
                return false;
            }
            let frame = fragment_offset / header.frame_size;
            if frame >= header.frame_count {
                self.channels[ch].active = false;
                self.channels[ch].invalidate_sqv4();
                return false;
            }
            frame * 64
        };
        let channel = &mut self.channels[ch];
        channel.sqv4_validated = true;
        channel.sqv4_version = header.version;
        channel.sqv4_variant = Some(header.variant);
        channel.sqv4_sample_count = header.samples_per_channel;
        channel.sqv4_frame_size = header.frame_size;
        channel.sqv4_loop_sample = loop_sample;
        channel.sqv4_cached_frame = None;
        channel.sqv4_next_frame = 0;
        channel.sqv4_decoder = Sqv4State::default();
        true
    }

    fn generate_sqv4_sample(&mut self, ch: usize) -> i16 {
        if !self.channels[ch].sqv4_validated && !self.initialize_sqv4(ch) {
            return 0;
        }
        let start = self.channels[ch].pcm_start_addr as usize;
        let mut sample_index = (self.channels[ch].phase * 256.0) as usize;
        let sample_count = self.channels[ch].sqv4_sample_count;
        if sample_count == 0 {
            self.channels[ch].active = false;
            return 0;
        }
        if sample_index >= sample_count {
            if self.channels[ch].pcm_control & 0x02 == 0 {
                self.channels[ch].active = false;
                return 0;
            }
            sample_index = self.channels[ch].sqv4_loop_sample;
            self.channels[ch].phase = sample_index as f64 / 256.0;
        }
        let frame_index = sample_index / 64;
        let within_frame = sample_index % 64;
        let frame_size = self.channels[ch].sqv4_frame_size;
        let variant = self.channels[ch].sqv4_variant.unwrap();
        let version = self.channels[ch].sqv4_version;
        let pcm_ram = self.pcm_ram.borrow();
        let channels = &mut self.channels;
        let channel = &mut channels[ch];
        if channel.sqv4_cached_frame != Some(frame_index) {
            if frame_index < channel.sqv4_next_frame {
                channel.sqv4_decoder = Sqv4State::default();
                channel.sqv4_next_frame = 0;
            }
            while channel.sqv4_next_frame <= frame_index {
                let decode_frame = channel.sqv4_next_frame;
                let fragment_offset = start + 16 + decode_frame * frame_size;
                let volume = pcm_ram[fragment_offset] as i32;
                let payload = &pcm_ram[fragment_offset + 1..fragment_offset + frame_size];
                let count = (sample_count - decode_frame * 64).min(64);
                for within in 0..count {
                    let code = sqv4::unpack_code(payload, within, variant.bits());
                    let reconstructed =
                        sqv4::decode_code(version, variant, &mut channel.sqv4_decoder, code);
                    channel.sqv4_frame[within] = (reconstructed * volume / 255) as i16;
                }
                channel.sqv4_cached_frame = Some(decode_frame);
                channel.sqv4_next_frame += 1;
            }
        }
        let sample = channel.sqv4_frame[within_frame];
        channel.phase += (channel.frequency as f64) / (SOUND_CLOCK as f64) / 8.0;
        sample
    }

    fn generate_noise_sample(&mut self, ch: usize) -> i16 {
        let c = &mut self.channels[ch];
        c.phase += (c.frequency as f64) / (SOUND_CLOCK as f64);
        let phase = (c.phase * 256.0) as u64;

        if c.old_phase / 8 != phase / 8 {
            c.old_phase = (c.phase * 256.0) as u64;
            let bit = match c.modulation_type {
                ModulationType::None => ((c.lfsr_state >> 0) ^ (c.lfsr_state >> 1)) & 1,
                ModulationType::Phase => {
                    // Custom tap mode in C++ (MOD_NOISE_CUSTOM_TAP = MOD_PHASE)
                    let taps =
                        ((c.modulation_param_1 as u32) << 16) | (c.modulation_param_2 as u32);
                    let mut b = 0u32;
                    for i in 0..23 {
                        if (taps & (1 << i)) != 0 {
                            b ^= (c.lfsr_state >> i) & 1;
                        }
                    }
                    b
                }
                _ => ((c.lfsr_state >> 0) ^ (c.lfsr_state >> 1)) & 1,
            };
            c.lfsr_state = (c.lfsr_state >> 1) | (bit << 22);
        }

        let sample = if (c.lfsr_state & 1) != 0 { 255 } else { 0 };
        (i32::from(c.apply_slew(sample)) - 128) as i16
    }

    fn generate_dma_pcm_sample(&mut self, _ch: usize) -> i16 {
        // DMA PCM not implemented yet (matches C++ implementation)
        0
    }

    /// Clock the sound chip and generate audio samples.
    /// Returns 3D vector: [channel][left/right (0=L, 1=R)][sample_index]
    /// Over-samples 4x from 192kHz to 48kHz.
    pub fn clock(&mut self, samples: usize) -> Vec<Vec<Vec<i16>>> {
        let mut output = vec![vec![vec![0i16; samples]; 2]; NUM_CHANNELS];
        for i in 0..samples {
            for ch in 0..NUM_CHANNELS {
                for _ in 0..4 {
                    let sample = apply_master_gain(self.generate_sample(ch));
                    let pan_l = (self.channels[ch].panpot >> 4) as i32;
                    let pan_r = (self.channels[ch].panpot & 0x0F) as i32;
                    output[ch][0][i] =
                        output[ch][0][i].saturating_add(((sample as i32) * pan_l / 15) as i16);
                    output[ch][1][i] =
                        output[ch][1][i].saturating_add(((sample as i32) * pan_r / 15) as i16);
                }
            }
        }
        output
    }

    /// Mix all channels to stereo 16-bit PCM output: `(left_samples, right_samples)`.
    pub fn clock_mixed(&mut self, samples: usize) -> (Vec<i16>, Vec<i16>) {
        let mut left_out = vec![0i16; samples];
        let mut right_out = vec![0i16; samples];
        self.clock_mixed_into(&mut left_out, &mut right_out);
        (left_out, right_out)
    }

    /// Add this chip's stereo output to existing buffers using saturating mix.
    pub fn clock_mixed_into(&mut self, left_out: &mut [i16], right_out: &mut [i16]) {
        let samples = left_out.len().min(right_out.len());
        for i in 0..samples {
            let mut mix_l = 0i32;
            let mut mix_r = 0i32;
            for ch in 0..NUM_CHANNELS {
                for _ in 0..4 {
                    let sample = apply_master_gain(self.generate_sample(ch));
                    let pan_l = (self.channels[ch].panpot >> 4) as i32;
                    let pan_r = (self.channels[ch].panpot & 0x0F) as i32;
                    mix_l += (sample as i32) * pan_l / 15;
                    mix_r += (sample as i32) * pan_r / 15;
                }
            }
            left_out[i] =
                left_out[i].saturating_add(mix_l.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
            right_out[i] =
                right_out[i].saturating_add(mix_r.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slew_modes_keep_existing_ids_and_limit_internal_samples() {
        assert_eq!(ModulationType::from(3), ModulationType::HardSync);
        assert_eq!(ModulationType::from(4), ModulationType::Window);
        for waveform in [
            WaveformType::Wavetable,
            WaveformType::Pcm,
            WaveformType::Noise,
        ] {
            for mode in [ModulationType::SlewRate, ModulationType::SlewStep] {
                let mut sound = S3w2Sound::new();
                sound.write_pcm_ram(0, 255);
                let c = &mut sound.channels[0];
                c.waveform_type = waveform;
                c.wavetable.fill(255);
                c.lfsr_state = 1;
                c.volume = 4;
                c.pcm_end_addr = 16;
                c.modulation_type = mode;
                c.modulation_param_1 = 256;
                assert_eq!(sound.generate_sample(0), -127);
                assert_eq!(sound.generate_sample(0), -126);
                sound.write_register(0x80a, 0);
                assert_eq!(sound.channels[0].slew_current, 0);
                sound.channels[0].lfsr_state = 1;
                assert_eq!(sound.generate_sample(0), -127);
                sound.reset();
                assert_eq!(sound.channels[0].slew_current, 0);
            }
        }
    }

    #[test]
    fn slew_rate_settles_and_step_oscillates_with_saturation() {
        let mut c = Channel::default();
        c.modulation_type = ModulationType::SlewRate;
        c.modulation_param_1 = 256;
        assert_eq!(c.apply_slew(1), 1);
        assert_eq!(c.apply_slew(1), 1);
        c.modulation_type = ModulationType::SlewStep;
        // Like the Furnace stub, equality takes a negative step.
        assert_eq!(c.apply_slew(1), 0);
        assert_eq!(c.apply_slew(1), 1);
        c.modulation_param_1 = 65535;
        assert_eq!(c.apply_slew(255), 255);
        assert_eq!(c.apply_slew(0), 0);
        c.modulation_param_1 = 0;
        assert_eq!(c.apply_slew(255), 0);
    }

    fn v21_sound() -> S3w2Sound {
        S3w2Sound::new()
    }

    fn v2_sound() -> S3w2Sound {
        S3w2Sound::with_version(Rc::new(RefCell::new(vec![0; PCM_RAM_SIZE])), SguVersion::V2)
    }

    #[test]
    fn wavetable_two_tap_interpolation_vs_v2() {
        for (mut sound, interpolated) in [(v21_sound(), true), (v2_sound(), false)] {
            sound.channels[0].wavetable[0] = 0;
            sound.channels[0].wavetable[1] = 128;
            // 24 => 24/750 = 0.032 entry per internal tick.
            sound.channels[0].frequency = 24;
            sound.channels[0].volume = 4;
            let mut outputs = Vec::new();
            for _ in 0..32 {
                outputs.push(sound.generate_sample(0));
            }
            // After 16 ticks the position is 0.512 entry.
            if interpolated {
                assert!((-64..=-60).contains(&outputs[15]), "{}", outputs[15]);
                assert_ne!(outputs[15], outputs[0]);
            } else {
                assert_eq!(outputs[15], -128);
            }
        }
    }

    #[test]
    fn wavetable_interpolation_wraps_last_to_first_entry() {
        let mut sound = v21_sound();
        sound.channels[0].wavetable[255] = 0;
        sound.channels[0].wavetable[0] = 200;
        sound.channels[0].volume = 4;
        sound.channels[0].phase = 255.5 / 256.0;
        // The advance (frequency 0) is nil, so index 255 + 0.5.
        assert_eq!(sound.generate_sample(0), 100 - 128);
    }

    #[test]
    fn pcm_two_tap_interpolation_loop_and_hold() {
        let mut sound = v21_sound();
        sound.write_pcm_ram(0, 0);
        sound.write_pcm_ram(1, 100);
        sound.write_pcm_ram(2, 200);
        let c = &mut sound.channels[0];
        c.waveform_type = WaveformType::Pcm;
        c.volume = 4;
        c.pcm_end_addr = 2;
        c.pcm_control = 0x01;
        c.phase = 1.5 / 256.0;
        // between ram[1]=100 and ram[2]... end is exclusive: one-shot holds ram[1].
        assert_eq!(sound.generate_sample(0), 100 - 128);
        sound.channels[0].phase = 0.5 / 256.0;
        assert_eq!(sound.generate_sample(0), 50 - 128);
        // Looping: the tap past the end reads the loop start.
        sound.channels[0].pcm_control = 0x03;
        sound.channels[0].pcm_loop_addr = 0;
        sound.channels[0].phase = 1.5 / 256.0;
        assert_eq!(sound.generate_sample(0), 50 - 128);
    }

    #[test]
    fn fifo_dac_is_16_bit_fixed_gain_with_decimation_and_pan() {
        let mut sound = v21_sound();
        sound.write_register(0x802, 5);
        assert_eq!(sound.read_register(0x802), 5);
        sound.write_register(0x803, 0); // VOLUME does not apply
        sound.write_register(0x801, 1); // hold each sample for 2 output steps
        for sample in [1000i16, -2000, 3000] {
            let [hi, lo] = sample.to_be_bytes();
            sound.write_register(0x80C, hi);
            sound.write_register(0x80D, lo);
        }
        assert_eq!(sound.read_register(0x80E), 0);
        assert_eq!(sound.read_register(0x80F), 3);
        let mut steps = Vec::new();
        for _ in 0..8 {
            let tick = sound.generate_sample(0);
            for _ in 0..3 {
                assert_eq!(sound.generate_sample(0), tick);
            }
            steps.push(tick);
        }
        assert_eq!(steps, [1000, 1000, -2000, -2000, 3000, 3000, 3000, 3000]);
        assert_eq!(sound.read_register(0x80F), 0);
        // Writing PHASE_RESET flushes the FIFO and the held value.
        sound.write_register(0x80C, 0x7F);
        sound.write_register(0x80D, 0xFF);
        sound.write_register(0x80A, 0);
        assert_eq!(sound.read_register(0x80F), 0);
        for _ in 0..4 {
            assert_eq!(sound.generate_sample(0), 0);
        }

        // Panpot: left only.
        sound.write_register(0x801, 0);
        sound.write_register(0x804, 0xF0);
        sound.write_register(0x80C, 0x40);
        sound.write_register(0x80D, 0x00);
        let (left, right) = sound.clock_mixed(1);
        assert_eq!((left[0], right[0]), (0x4000 / 8 * 4, 0));
    }

    #[test]
    fn fifo_capacity_drops_excess_writes() {
        let mut sound = v21_sound();
        for _ in 0..FIFO_CAPACITY + 10 {
            sound.write_register(0x80D, 1);
        }
        let count = u16::from_be_bytes([sound.read_register(0x80E), sound.read_register(0x80F)]);
        assert_eq!(count as usize, FIFO_CAPACITY);
    }

    #[test]
    fn older_versions_ignore_new_features() {
        let mut v2 = v2_sound();
        v2.write_register(0x802, 5);
        assert_eq!(v2.read_register(0x802), 0);
        v2.write_register(0x80D, 1);
        assert_eq!(v2.read_register(0x80F), 0);
        v2.write_register(0x805, 5 << 3);
        assert_eq!(v2.channels[0].modulation_type, ModulationType::SlewRate);
        assert_eq!(SguVersion::V1.core_count(), 1);
        assert_eq!(SguVersion::V2.core_count(), 2);
    }

    #[test]
    fn version_parses_config_values() {
        assert_eq!(SguVersion::parse("1"), Some(SguVersion::V1));
        assert_eq!(SguVersion::parse("v2"), Some(SguVersion::V2));
        assert_eq!(SguVersion::parse("2.1"), Some(SguVersion::V2_1));
        assert_eq!(SguVersion::parse("3"), None);
        assert_eq!(SguVersion::default(), SguVersion::V2_1);
    }

    #[test]
    fn test_initialization() {
        let mut sound = S3w2Sound::new();
        assert_eq!(sound.channels.len(), NUM_CHANNELS);
        assert_eq!(sound.pcm_ram.borrow().len(), PCM_RAM_SIZE);
        assert_eq!(sound.read_register(0x000), 0x80);
    }

    #[test]
    fn test_wavetable_registers() {
        let mut sound = S3w2Sound::new();
        // Write wavetable byte to ch 2, offset 0x42
        let addr = (2 << 8) | 0x42;
        sound.write_register(addr, 0xAB);
        assert_eq!(sound.read_register(addr), 0xAB);
        assert_eq!(sound.channels[2].wavetable[0x42], 0xAB);
    }

    #[test]
    fn test_channel_control_registers() {
        let mut sound = S3w2Sound::new();
        // Control base for ch 1 is 0x800 + 32 * 1 = 0x820
        sound.write_register(0x820, 0x04); // Frequency high
        sound.write_register(0x821, 0x40); // Frequency low (1088 Hz)
        sound.write_register(0x822, 1); // PCM mode
        sound.write_register(0x823, 200); // Volume
        sound.write_register(0x824, 0x84); // Panpot
        sound.write_register(0x825, (1 << 3) | 3); // Mod type Phase, target ch 3

        assert_eq!(sound.channels[1].frequency, 1088);
        assert_eq!(sound.channels[1].waveform_type, WaveformType::Pcm);
        assert_eq!(sound.channels[1].volume, 200);
        assert_eq!(sound.channels[1].panpot, 0x84);
        assert_eq!(sound.channels[1].modulation_type, ModulationType::Phase);
        assert_eq!(sound.channels[1].modulation_target, 3);

        assert_eq!(sound.read_register(0x820), 0x04);
        assert_eq!(sound.read_register(0x821), 0x40);
        assert_eq!(sound.read_register(0x822), 1);
        assert_eq!(sound.read_register(0x823), 200);
        assert_eq!(sound.read_register(0x824), 0x84);
        assert_eq!(sound.read_register(0x825), (1 << 3) | 3);
    }

    #[test]
    fn test_pcm_registers_and_ram() {
        let mut sound = S3w2Sound::new();
        sound.write_pcm_ram(0x1000, 0xFE);
        assert_eq!(sound.read_pcm_ram(0x1000), 0xFE);

        // PCM regs for ch 0 at 0x800 + 0x10
        sound.write_register(0x810, 0x01); // Start addr byte 2 (MSB of 20bit)
        sound.write_register(0x811, 0x23); // Start addr byte 1
        sound.write_register(0x812, 0x45); // Start addr byte 0
        sound.write_register(0x819, 0x03); // Play + Loop

        assert_eq!(sound.channels[0].pcm_start_addr, 0x012345);
        assert_eq!(sound.channels[0].pcm_control, 0x03);
        assert!(sound.channels[0].active);
    }

    #[test]
    fn master_output_gain_is_one_quarter() {
        assert_eq!(apply_master_gain(4000), 500);
        assert_eq!(apply_master_gain(-4000), -500);
    }

    #[test]
    fn test_audio_clock() {
        let mut sound = S3w2Sound::new();
        // Setup channel 0 with a square-ish wavetable
        for i in 0..128 {
            sound.write_channel_wavetable(0, i as u8, 255);
        }
        for i in 128..256 {
            sound.write_channel_wavetable(0, i as u8, 0);
        }
        sound.channels[0].frequency = 440;
        sound.channels[0].volume = 128;
        sound.channels[0].panpot = 0xFF;

        let out = sound.clock(64);
        assert_eq!(out.len(), NUM_CHANNELS);
        assert_eq!(out[0][0].len(), 64);
        assert_eq!(out[0][1].len(), 64);
        let mut left = [100; 64];
        let mut right = [100; 64];
        sound.clock_mixed_into(&mut left, &mut right);
        assert_eq!(left, right);
        assert!(left.iter().any(|&sample| sample != 100));
    }

    // Disabled with the tests that depend on the external s3w2_core golden fixtures.
    #[cfg(any())]
    fn sqv4_golden_bytes() -> Vec<u8> {
        include_str!("../../../../s3w2_core/tests/sqv4_golden.inc")
            .split(',')
            .filter_map(|byte| {
                let byte = byte.trim().trim_start_matches("0x");
                (!byte.is_empty()).then(|| u8::from_str_radix(byte, 16).unwrap())
            })
            .collect()
    }

    #[cfg(any())]
    fn configure_sqv4(sound: &mut S3w2Sound, start: u32, end: u32, loop_addr: u32, control: u8) {
        sound.write_register(0x800, 0x17);
        sound.write_register(0x801, 0x70); // 6000 Hz => one decoded sample per call
        sound.write_register(0x802, 4);
        sound.write_register(0x803, 255);
        for (base, address) in [(0x810, start), (0x813, end), (0x816, loop_addr)] {
            sound.write_register(base, (address >> 16) as u8);
            sound.write_register(base + 1, (address >> 8) as u8);
            sound.write_register(base + 2, address as u8);
        }
        sound.write_register(0x819, control);
    }

    // Disabled until the external s3w2_core golden fixture is bundled in this repo.
    // #[ignore] would still expand include_str! and require the missing file.
    #[cfg(any())]
    #[test]
    fn sqv4_playback_matches_codec_across_frames_loop_reset_and_errors() {
        let bytes = sqv4_golden_bytes();
        let decoded = super::sqv4::decode(&bytes).unwrap();
        assert_eq!(decoded.samples, (1..=65).collect::<Vec<i16>>());
        let start = 128u32;
        let end = start + bytes.len() as u32;
        let mut sound = S3w2Sound::new();
        for (offset, byte) in bytes.iter().copied().enumerate() {
            sound.write_pcm_ram(start + offset as u32, byte);
        }
        configure_sqv4(&mut sound, start, end, start, 1);
        for expected in 1..=65 {
            assert_eq!(sound.generate_sample(0), expected);
        }
        assert_eq!(sound.generate_sample(0), 0);
        assert!(!sound.channels[0].active);

        configure_sqv4(&mut sound, start, end, start + 49, 3);
        sound.write_register(0x80A, 0);
        for expected in 1..=65 {
            assert_eq!(sound.generate_sample(0), expected);
        }
        assert_eq!(sound.generate_sample(0), 65); // loop to the second fragment boundary
        sound.write_register(0x80A, 0); // phase reset must also reset predictor replay
        assert_eq!(sound.generate_sample(0), 1);
        sound.write_register(0x803, 128);
        sound.write_register(0x80A, 0);
        for expected in 1..=65 {
            assert_eq!(sound.generate_sample(0), expected * 128 / 255);
        }

        sound.write_pcm_ram(start, 0);
        configure_sqv4(&mut sound, start, end, start, 1);
        assert_eq!(sound.generate_sample(0), 0);
        assert!(!sound.channels[0].active);

        configure_sqv4(
            &mut sound,
            (PCM_RAM_SIZE - 8) as u32,
            (PCM_RAM_SIZE + 8) as u32,
            0,
            1,
        );
        assert_eq!(sound.generate_sample(0), 0);
        assert!(!sound.channels[0].active);

        for (offset, byte) in bytes.iter().copied().enumerate() {
            sound.write_pcm_ram(start + offset as u32, byte);
        }
        configure_sqv4(&mut sound, start, end, start + 50, 1);
        assert_eq!(sound.generate_sample(0), 0);
        assert!(!sound.channels[0].active);

        let stereo =
            super::sqv4::encode_interleaved(&[1, 1], 2, 8_000, Sqv4Variant::H, 255).unwrap();
        for (offset, byte) in stereo.iter().copied().enumerate() {
            sound.write_pcm_ram(start + offset as u32, byte);
        }
        configure_sqv4(&mut sound, start, start + stereo.len() as u32, start, 1);
        assert_eq!(sound.generate_sample(0), 0);
        assert!(!sound.channels[0].active);
    }

    // Disabled until the external s3w2_core golden fixture is bundled in this repo.
    #[cfg(any())]
    #[test]
    fn sqv4_lplus_v2_hardware_matches_codec_fixture() {
        let bytes: Vec<u8> = include_str!("../../../../s3w2_core/tests/sqv4_lplus_golden.inc")
            .split(',')
            .filter_map(|byte| {
                let byte = byte.trim().trim_start_matches("0x");
                (!byte.is_empty()).then(|| u8::from_str_radix(byte, 16).unwrap())
            })
            .collect();
        let decoded = super::sqv4::decode(&bytes).unwrap();
        assert_eq!(decoded.variant, Sqv4Variant::LPlus);
        assert_eq!(decoded.samples, [12, 35, 75, 147, 275]);
        let start = 256u32;
        let mut sound = S3w2Sound::new();
        for (offset, byte) in bytes.iter().copied().enumerate() {
            sound.write_pcm_ram(start + offset as u32, byte);
        }
        configure_sqv4(&mut sound, start, start + bytes.len() as u32, start, 1);
        for expected in decoded.samples {
            assert_eq!(sound.generate_sample(0), expected);
        }
    }
}
