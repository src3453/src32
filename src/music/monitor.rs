//! Headless SGUB playback engine and analysis used by the `sgub_player` monitor.
//!
//! The engine owns two SGU cores with shared PCMRAM, applies exactly one SGUB
//! frame per 60 Hz tick (the same contract as the guest driver and the music
//! editor) and renders each channel separately so the monitor can meter, mute
//! and scope individual voices. The unmuted per-channel sum is bit-identical to
//! `S3w2Sound::clock_mixed_into_i32` followed by `saturate_output`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use super::stream::{MusicError, StreamPlayer};
use crate::devices::sgu::s3w2::{
    ModulationType, NUM_CHANNELS, PCM_RAM_SIZE, PHASE_FRAC_BITS, S3w2Sound, SAMPLE_RATE,
    SguVersion, WaveformType, saturate_output,
};

/// Voices across both SGU cores.
pub const VOICES: usize = NUM_CHANNELS * 2;
/// Output samples per 60 Hz frame at the SGU's native 48 kHz.
pub const FRAME_SAMPLES: usize = SAMPLE_RATE / 60;
/// Length of the retained stereo history used by the scope and spectrum.
pub const HISTORY: usize = 2048;

/// Snapshot of one channel's visible register state for display.
#[derive(Clone, Debug)]
pub struct ChannelView {
    pub waveform: WaveformType,
    pub frequency: u16,
    pub volume: u8,
    pub panpot: u8,
    pub modulation: ModulationType,
    pub modulation_target: u8,
    pub modulation_relative: bool,
    pub param1: u16,
    pub param2: u16,
    pub pcm_start: u32,
    pub pcm_end: u32,
    pub pcm_loop: u32,
    pub pcm_control: u8,
    /// Sample offset from PCM_START (PCM/SQV4 only).
    pub pcm_position: u32,
    pub fifo_len: usize,
    pub wavetable: [u8; 256],
}

impl ChannelView {
    pub fn capture(core: &S3w2Sound, channel: usize) -> Self {
        let c = &core.channels[channel];
        Self {
            waveform: c.waveform_type,
            frequency: c.frequency,
            volume: c.volume,
            panpot: c.panpot,
            modulation: c.modulation_type,
            modulation_target: c.modulation_target & 7,
            modulation_relative: c.modulation_targeting_mode & 1 != 0,
            param1: c.modulation_param_1,
            param2: c.modulation_param_2,
            pcm_start: c.pcm_start_addr,
            pcm_end: c.pcm_end_addr,
            pcm_loop: c.pcm_loop_addr,
            pcm_control: c.pcm_control,
            pcm_position: (c.phase >> PHASE_FRAC_BITS) as u32,
            fifo_len: c.fifo.len(),
            wavetable: c.wavetable,
        }
    }

    /// True when the register state can produce sound.
    pub fn audible(&self) -> bool {
        match self.waveform {
            WaveformType::Wavetable | WaveformType::Noise => self.volume != 0,
            WaveformType::Pcm | WaveformType::Sqv4 => self.volume != 0 && self.pcm_control & 1 != 0,
            WaveformType::FifoDac => self.pcm_control & 1 != 0,
            WaveformType::DmaPcm => false,
        }
    }

    /// Pitch as a fractional MIDI note. Only Wavetable has a musical pitch:
    /// one FREQUENCY step is 1 Hz there (256 samples per cycle at freq/750
    /// samples per 192 kHz tick).
    pub fn midi_note(&self) -> Option<f32> {
        (self.waveform == WaveformType::Wavetable && self.frequency != 0)
            .then(|| midi_from_hz(f32::from(self.frequency)))
    }
}

pub fn midi_from_hz(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// "A-4" style name for the nearest note; octave 4 contains A440 (MIDI 69).
pub fn note_name(midi: f32) -> String {
    const NAMES: [&str; 12] = [
        "C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-",
    ];
    let note = midi.round() as i32;
    let octave = note.div_euclid(12) - 1;
    let name = NAMES[note.rem_euclid(12) as usize];
    if (0..=9).contains(&octave) {
        format!("{name}{octave}")
    } else {
        "---".into()
    }
}

pub fn waveform_tag(waveform: WaveformType) -> &'static str {
    match waveform {
        WaveformType::Wavetable => "WAVE",
        WaveformType::Pcm => "PCM ",
        WaveformType::Noise => "NOIS",
        WaveformType::DmaPcm => "DMA ",
        WaveformType::Sqv4 => "SQV4",
        WaveformType::FifoDac => "FIFO",
    }
}

pub fn modulation_tag(modulation: ModulationType) -> &'static str {
    match modulation {
        ModulationType::None => "---",
        ModulationType::Phase => "PHS",
        ModulationType::Ring => "RNG",
        ModulationType::HardSync => "SYN",
        ModulationType::Window => "WIN",
        ModulationType::SlewRate => "SLR",
        ModulationType::SlewStep => "SLS",
    }
}

/// Smoothed level meter with a falling peak hold, both in 0..=1.
#[derive(Clone, Copy, Debug, Default)]
pub struct Meter {
    pub level: f32,
    pub peak: f32,
    peak_hold: u8,
}

impl Meter {
    pub fn update(&mut self, peak: f32) {
        self.level = if peak > self.level {
            peak
        } else {
            (self.level * 0.82).max(peak)
        };
        if peak >= self.peak {
            self.peak = peak;
            self.peak_hold = 30;
        } else if self.peak_hold > 0 {
            self.peak_hold -= 1;
        } else {
            self.peak = (self.peak - 0.02).max(self.level);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Play,
    Pause,
    Stop,
}

/// Owns the stream, both SGU cores and the analysis buffers.
pub struct Engine {
    bytes: Vec<u8>,
    version: SguVersion,
    player: StreamPlayer,
    cores: [Rc<RefCell<S3w2Sound>>; 2],
    pub transport: Transport,
    /// Frames applied since the start, including repeated passes.
    pub elapsed_frames: u64,
    pub loops: u32,
    pub muted: [bool; VOICES],
    pub channel_meters: [Meter; VOICES],
    pub master_meters: [Meter; 2],
    /// Ordered v3 card writes seen (no expansion card is attached).
    pub card_writes: u64,
    pub error: Option<String>,
    history: [VecDeque<i16>; 2],
}

impl Engine {
    pub fn new(bytes: Vec<u8>, version: SguVersion) -> Result<Self, MusicError> {
        let player = StreamPlayer::from_bytes(&bytes)?;
        Ok(Self {
            cores: new_cores(version),
            player,
            bytes,
            version,
            transport: Transport::Play,
            elapsed_frames: 0,
            loops: 0,
            muted: [false; VOICES],
            channel_meters: [Meter::default(); VOICES],
            master_meters: [Meter::default(); 2],
            card_writes: 0,
            error: None,
            history: std::array::from_fn(|_| VecDeque::from(vec![0; HISTORY])),
        })
    }

    pub fn stream_version(&self) -> u8 {
        self.player.version()
    }
    pub fn frame_count(&self) -> u32 {
        self.player.frame_count()
    }
    pub fn frame_index(&self) -> u32 {
        self.player.frame_index()
    }
    pub fn repeats(&self) -> bool {
        self.player.repeats()
    }
    pub fn finished(&self) -> bool {
        self.player.is_finished()
    }
    pub fn stream_size(&self) -> usize {
        self.bytes.len()
    }
    pub fn core(&self, index: usize) -> std::cell::Ref<'_, S3w2Sound> {
        self.cores[index].borrow()
    }
    pub fn channel(&self, voice: usize) -> ChannelView {
        ChannelView::capture(
            &self.cores[voice / NUM_CHANNELS].borrow(),
            voice % NUM_CHANNELS,
        )
    }
    /// Most recent `HISTORY` output samples of one side (0 = left).
    pub fn history(&self, side: usize) -> &VecDeque<i16> {
        &self.history[side]
    }

    fn advance_stream(&mut self) {
        if self.error.is_some() || self.player.is_finished() {
            return;
        }
        let before = self.player.frame_index();
        if let Err(error) = self.player.advance_vsync(&self.cores) {
            self.error = Some(error.to_string());
            return;
        }
        self.card_writes += self.player.take_card_writes().len() as u64;
        if self.player.is_finished() {
            return;
        }
        if self.player.frame_index() < before {
            self.loops += 1;
        }
        self.elapsed_frames += 1;
    }

    /// Render one 60 Hz frame of `samples` output samples. Paused and stopped
    /// engines render silence without advancing the stream or the SGU state.
    pub fn tick(&mut self, samples: usize) -> (Vec<i16>, Vec<i16>) {
        let mut left = vec![0i32; samples];
        let mut right = vec![0i32; samples];
        let mut peaks = [0f32; VOICES];
        if self.transport == Transport::Play {
            // Register updates must precede synthesis on both shared-PCMRAM cores.
            self.advance_stream();
            for (core_index, core) in self.cores.iter().enumerate() {
                let output = core.borrow_mut().clock(samples);
                for (channel, sides) in output.iter().enumerate() {
                    let voice = core_index * NUM_CHANNELS + channel;
                    let mut peak = 0i32;
                    for i in 0..samples {
                        let (l, r) = (i32::from(sides[0][i]), i32::from(sides[1][i]));
                        peak = peak.max(l.abs()).max(r.abs());
                        if !self.muted[voice] {
                            left[i] += l;
                            right[i] += r;
                        }
                    }
                    // A single channel peaks near 1/4 of full scale (master gain).
                    peaks[voice] = (peak as f32 / 8192.0).min(1.0);
                }
            }
        }
        for (meter, peak) in self.channel_meters.iter_mut().zip(peaks) {
            meter.update(peak);
        }
        let left: Vec<i16> = left.into_iter().map(saturate_output).collect();
        let right: Vec<i16> = right.into_iter().map(saturate_output).collect();
        for (side, buffer) in [&left, &right].into_iter().enumerate() {
            let peak = buffer
                .iter()
                .map(|s| i32::from(*s).abs())
                .max()
                .unwrap_or(0);
            self.master_meters[side].update(peak as f32 / 32768.0);
            let history = &mut self.history[side];
            for &sample in buffer {
                history.pop_front();
                history.push_back(sample);
            }
        }
        (left, right)
    }

    /// Restart from silence and apply `frame` frames without synthesizing audio.
    /// PCM/FIFO voices therefore resume from register state, not from their
    /// exact sample position, which is acceptable for a monitor seek.
    pub fn seek(&mut self, frame: u64) {
        let transport = self.transport;
        let muted = self.muted;
        let Ok(player) = StreamPlayer::from_bytes(&self.bytes) else {
            return;
        };
        self.player = player;
        self.cores = new_cores(self.version);
        self.elapsed_frames = 0;
        self.loops = 0;
        self.card_writes = 0;
        self.error = None;
        // One advance per frame, exactly like `tick`, so a seek past the end of a
        // non-repeating stream also performs the final stop-all frame.
        for _ in 0..frame {
            self.advance_stream();
            if self.error.is_some() || self.player.is_finished() {
                break;
            }
        }
        self.transport = transport;
        self.muted = muted;
        for history in &mut self.history {
            history.iter_mut().for_each(|s| *s = 0);
        }
    }

    pub fn stop(&mut self) {
        self.seek(0);
        self.transport = Transport::Stop;
    }
}

fn new_cores(version: SguVersion) -> [Rc<RefCell<S3w2Sound>>; 2] {
    let pcm_ram = Rc::new(RefCell::new(vec![0; PCM_RAM_SIZE]));
    std::array::from_fn(|_| {
        Rc::new(RefCell::new(S3w2Sound::with_version(
            pcm_ram.clone(),
            version,
        )))
    })
}

/// Log-spaced spectrum of the newest `HISTORY` samples, in dB (about -96..0),
/// one value per band between `low_hz` and `high_hz`.
pub fn spectrum_db(samples: &VecDeque<i16>, bands: usize, low_hz: f32, high_hz: f32) -> Vec<f32> {
    let n = HISTORY;
    let mut re = vec![0f32; n];
    let mut im = vec![0f32; n];
    let start = samples.len().saturating_sub(n);
    for (i, sample) in samples.iter().skip(start).enumerate() {
        let window = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos();
        re[i] = f32::from(*sample) / 32768.0 * window;
    }
    fft(&mut re, &mut im);
    let bin_hz = SAMPLE_RATE as f32 / n as f32;
    // Hann coherent gain 0.5; a full-scale sine reads 0 dB.
    let scale = 4.0 / n as f32;
    let ratio = (high_hz / low_hz).powf(1.0 / bands as f32);
    (0..bands)
        .map(|band| {
            let lo = low_hz * ratio.powi(band as i32);
            let hi = lo * ratio;
            let first = ((lo / bin_hz).floor() as usize).clamp(1, n / 2 - 1);
            let last = ((hi / bin_hz).ceil() as usize).clamp(first + 1, n / 2);
            let magnitude = (first..last)
                .map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt())
                .fold(0f32, f32::max)
                * scale;
            (20.0 * magnitude.max(1e-5).log10()).max(-96.0)
        })
        .collect()
}

/// In-place iterative radix-2 FFT; `re.len()` must be a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -std::f32::consts::TAU / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (sin, cos) = (angle * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cos - im[b] * sin;
                let ti = re[b] * sin + im[b] * cos;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_follow_a440_octave_4() {
        assert_eq!(note_name(midi_from_hz(440.0)), "A-4");
        assert_eq!(note_name(midi_from_hz(261.63)), "C-4");
        assert_eq!(note_name(midi_from_hz(131.0)), "C-3");
        assert_eq!(note_name(midi_from_hz(466.16)), "A#4");
    }

    #[test]
    fn spectrum_peaks_at_the_band_containing_the_tone() {
        let tone: VecDeque<i16> = (0..HISTORY)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                ((std::f32::consts::TAU * 1000.0 * t).sin() * 32767.0) as i16
            })
            .collect();
        let bands = spectrum_db(&tone, 40, 40.0, 20000.0);
        let ratio = (20000.0f32 / 40.0).powf(1.0 / 40.0);
        let expected = ((1000.0f32 / 40.0).ln() / ratio.ln()) as usize;
        let loudest = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert!(loudest.abs_diff(expected) <= 1, "{loudest} vs {expected}");
        assert!(bands[loudest] > -3.0, "{}", bands[loudest]);
        assert!(bands[0] < -60.0);
    }

    #[test]
    fn meter_holds_peak_then_falls() {
        let mut meter = Meter::default();
        meter.update(0.8);
        for _ in 0..30 {
            meter.update(0.0);
        }
        assert_eq!(meter.peak, 0.8);
        meter.update(0.0);
        assert!(meter.peak < 0.8);
        assert!(meter.level < 0.01);
    }
}
