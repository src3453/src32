// 3WS8PN (S3W2) Sound Generator Core Implementation
//
// 3WS8PN is a wavetable/PCM/noise sound generator chip with 8 channels.
// Master Output: 16-bit Stereo Linear PCM, 48kHz
// Sound clock: 192kHz (48MHz / 250)

pub const NUM_CHANNELS: usize = 8;
pub const WAVETABLE_SIZE: usize = 256;
pub const PCM_RAM_SIZE: usize = 1024 * 1024; // 1MB
pub const SAMPLE_RATE: usize = 48000;
pub const SOUND_CLOCK: u32 = 192_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveformType {
    Wavetable = 0,
    Pcm = 1,
    Noise = 2,
    DmaPcm = 3,
}

impl From<u8> for WaveformType {
    fn from(value: u8) -> Self {
        match value {
            0 => WaveformType::Wavetable,
            1 => WaveformType::Pcm,
            2 => WaveformType::Noise,
            3 => WaveformType::DmaPcm,
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
}

impl From<u8> for ModulationType {
    fn from(value: u8) -> Self {
        match value {
            1 => ModulationType::Phase,
            2 => ModulationType::Ring,
            3 => ModulationType::HardSync,
            4 => ModulationType::Window,
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

    // Internal state
    pub old_phase: u64,
    pub lfsr_state: u32,
    pub phase: f64,
    pub last_sample: i16,
    pub active: bool,

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
            old_phase: 0,
            lfsr_state: 0x12D4_803C,
            phase: 0.0,
            last_sample: 0,
            active: true,
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
        self.old_phase = 0;
        self.phase = 0.0;
        self.last_sample = 0;
        self.lfsr_state = 0x12D4_803C;
        self.active = true;
        self.wavetable.fill(0x80);
    }
}

pub struct S3w2Sound {
    pub channels: [Channel; NUM_CHANNELS],
    pub pcm_ram: Vec<u8>,
}

impl Default for S3w2Sound {
    fn default() -> Self {
        Self::new()
    }
}

impl S3w2Sound {
    pub fn new() -> Self {
        let mut sound = Self {
            channels: std::array::from_fn(|_| Channel::default()),
            pcm_ram: vec![0; PCM_RAM_SIZE],
        };
        sound.reset();
        sound
    }

    pub fn reset(&mut self) {
        self.pcm_ram.fill(0);
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

    pub fn write_pcm_ram(&mut self, address: u32, value: u8) {
        let addr = address as usize;
        if addr < self.pcm_ram.len() {
            self.pcm_ram[addr] = value;
        }
    }

    pub fn read_pcm_ram(&self, address: u32) -> u8 {
        let addr = address as usize;
        if addr < self.pcm_ram.len() {
            self.pcm_ram[addr]
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
        let c = &mut self.channels[ch];
        match offset {
            0 => {
                c.frequency = (c.frequency & 0x00FF) | ((value as u16) << 8);
            }
            1 => {
                c.frequency = (c.frequency & 0xFF00) | (value as u16);
            }
            2 => {
                c.waveform_type = WaveformType::from(value);
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
                c.lfsr_state = 0x12D4_803C;
            }
            0x0B => {
                c.modulation_targeting_mode = value & 0x01;
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
                0
            }
            0x0B => c.modulation_targeting_mode & 0x01,
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

    pub fn generate_sample(&mut self, ch: usize) -> i16 {
        if ch >= NUM_CHANNELS {
            return 0;
        }
        if !self.channels[ch].active {
            return 0;
        }

        let sample = match self.channels[ch].waveform_type {
            WaveformType::Wavetable => self.generate_wavetable_sample(ch),
            WaveformType::Pcm => self.generate_pcm_sample(ch),
            WaveformType::Noise => self.generate_noise_sample(ch),
            WaveformType::DmaPcm => self.generate_dma_pcm_sample(ch),
        };

        self.channels[ch].last_sample = sample;
        ((sample as i32) * (self.channels[ch].volume as i32) / 4) as i16
    }

    fn generate_wavetable_sample(&mut self, ch: usize) -> i16 {
        let abs_target_ch = Self::convert_to_absolute_channel_address(
            ch as u8,
            self.channels[ch].modulation_targeting_mode,
            self.channels[ch].modulation_target,
        );
        let target_last_sample = self.channels[abs_target_ch].last_sample;
        let target_phase = self.channels[abs_target_ch].phase;

        let c = &mut self.channels[ch];
        c.phase += (c.frequency as f64) / (SOUND_CLOCK as f64);
        let phase = (c.phase * 256.0) as u64;

        let index = match c.modulation_type {
            ModulationType::None => (phase & 0xFF) as usize,
            ModulationType::Phase => {
                let phase_offset =
                    ((c.modulation_param_1 as i32 * target_last_sample as i32) >> 12) as i64;
                ((phase.wrapping_add(phase_offset as u64)) & 0xFF) as usize
            }
            _ => (phase & 0xFF) as usize,
        };

        let mut sample8 = c.wavetable[index];

        if c.modulation_type == ModulationType::Ring {
            let mod_sample = target_last_sample as i32;
            let org_sample8 = sample8 as i32;
            let calculated =
                (((org_sample8 - 128) * mod_sample * (c.modulation_param_1 as i32)) / 65536) + 128;
            sample8 = calculated.clamp(0, 255) as u8;
        } else if c.modulation_type == ModulationType::HardSync {
            if target_phase < 1.0 {
                c.phase = 0.0;
                sample8 = c.wavetable[0];
            }
        } else if c.modulation_type == ModulationType::Window {
            let mod_sample = target_last_sample;
            if mod_sample < 0 {
                sample8 = 128;
            }
        }

        (sample8 as i32 - 128) as i16
    }

    fn generate_pcm_sample(&mut self, ch: usize) -> i16 {
        let abs_target_ch = Self::convert_to_absolute_channel_address(
            ch as u8,
            self.channels[ch].modulation_targeting_mode,
            self.channels[ch].modulation_target,
        );
        let target_last_sample = self.channels[abs_target_ch].last_sample;

        let c = &mut self.channels[ch];
        let mut phase = (c.phase * 256.0) as u64;

        if c.modulation_type == ModulationType::Phase {
            let phase_offset =
                ((c.modulation_param_1 as i32 * target_last_sample as i32) >> 12) as i64;
            phase = phase.wrapping_add(phase_offset as u64);
        }

        let addr = ((phase + c.pcm_start_addr as u64) as usize) & (PCM_RAM_SIZE - 1);
        let sample8 = self.pcm_ram[addr];
        let mut out = (sample8 as i32 - 128) as i16;

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

        if (c.lfsr_state & 1) != 0 { 127 } else { -128 }
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
                    let sample = self.generate_sample(ch) / 2; // Reduce volume
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

    /// Mix all channels to stereo 16-bit PCM output: `(left_samples, right_samples)`
    pub fn clock_mixed(&mut self, samples: usize) -> (Vec<i16>, Vec<i16>) {
        let mut left_out = vec![0i16; samples];
        let mut right_out = vec![0i16; samples];
        for i in 0..samples {
            let mut mix_l = 0i32;
            let mut mix_r = 0i32;
            for ch in 0..NUM_CHANNELS {
                for _ in 0..4 {
                    let sample = self.generate_sample(ch) / 2; // Reduce volume 
                    let pan_l = (self.channels[ch].panpot >> 4) as i32;
                    let pan_r = (self.channels[ch].panpot & 0x0F) as i32;
                    mix_l += (sample as i32) * pan_l / 15;
                    mix_r += (sample as i32) * pan_r / 15;
                }
            }
            left_out[i] = mix_l.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            right_out[i] = mix_r.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
        (left_out, right_out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialization() {
        let mut sound = S3w2Sound::new();
        assert_eq!(sound.channels.len(), NUM_CHANNELS);
        assert_eq!(sound.pcm_ram.len(), PCM_RAM_SIZE);
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
    }
}
