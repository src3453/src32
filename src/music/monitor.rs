//! Headless SGUB playback engine and analysis used by the `sgub_player` monitor.
//!
//! The engine owns two SGU cores with shared PCMRAM, the 34SI22 and the
//! expansion cards a stream requires. It applies one SGUB frame per frame
//! period (VSync = 800 samples, or the v4 `frame_period` in 48 MHz cycles) and
//! renders each SGU channel separately so the monitor can meter, mute and scope
//! individual voices. The unmuted SGU per-channel sum is bit-identical to
//! `S3w2Sound::clock_mixed_into_i32` followed by `saturate_output`.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use super::stream::{CardRequirement, MusicError, SGUB_TARGET_SI22, StreamPlayer};
use crate::devices::expansion::HostedCard;
use crate::devices::sgu::s3w2::{
    ModulationType, NUM_CHANNELS, PCM_RAM_SIZE, PHASE_FRAC_BITS, S3w2Sound, SAMPLE_RATE,
    SguVersion, WaveformType, saturate_output,
};
use crate::devices::sgu::sgu::Si22Unit;
use crate::devices::sgu::si22::Si22;

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
    /// Samples per wavetable cycle (3WS8PNB SAMPLE_CONTROL; 256 otherwise).
    pub wave_length: usize,
    /// False when the core holds each sample (pre-3WS8PNA, or SAMPLE_CONTROL bit 2).
    pub interpolate: bool,
    /// Wave pool entry the wave SRAM currently holds, when the stream copied
    /// one with COPY_POOL and nothing has changed the SRAM since.
    pub wave_pool: Option<u16>,
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
            wave_length: core.wavetable_length(channel),
            interpolate: core.interpolates(channel),
            wave_pool: None,
        }
    }

    /// One wavetable cycle as display points in a `width` x `height` box
    /// (origin top-left, value 255 at the top). Only the first `wave_length`
    /// SRAM bytes play. Interpolated tables draw straight segments ending
    /// back at sample 0, as the core wraps the next tap; held tables draw a
    /// step per sample. Either way sample `i` starts at `i * width / length`.
    pub fn wave_points(&self, width: f32, height: f32) -> Vec<[f32; 2]> {
        let length = self.wave_length;
        let x = |i: usize| i as f32 * width / length as f32;
        let y = |i: usize| height - f32::from(self.wavetable[i % length]) / 255.0 * height;
        let mut points = Vec::with_capacity(length * 2 + 1);
        for i in 0..length {
            if self.interpolate {
                points.push([x(i), y(i)]);
            } else {
                points.push([x(i), y(i)]);
                points.push([x(i + 1), y(i)]);
            }
        }
        if self.interpolate {
            points.push([width, y(length)]);
        }
        points
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
    /// one FREQUENCY step is 1 Hz for a 256-sample table (freq/750 samples
    /// per 192 kHz tick); a shorter 3WS8PNB table cycles 256/length times faster.
    pub fn midi_note(&self) -> Option<f32> {
        (self.waveform == WaveformType::Wavetable && self.frequency != 0)
            .then(|| midi_from_hz(f32::from(self.frequency) * 256.0 / self.wave_length as f32))
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

/// 48 MHz bus cycles per 48 kHz output sample.
pub const CYCLES_PER_SAMPLE: u32 = 1000;
/// A VSync frame (60 Hz) in 48 MHz cycles: exactly `FRAME_SAMPLES` samples.
pub const VSYNC_FRAME_CYCLES: u32 = 800_000;

/// Expansion card models the monitor knows how to display. The stream never
/// says which card it targets; the kind comes from the ID the card itself reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKind {
    /// 3HS816: 16 harmonic channels at slot-relative `0x000..0x4FF`.
    Hs816,
    /// 3HS88PWN4 card (ID `3WS88PWN4`): 8 HS + 4 PCM/WT channels at `0x400000..`.
    Hs88pwn4,
    Unknown,
}

const HS816_ID_OFFSET: u32 = 0x4F8;
const HS816_ID: &[u8] = b"3HS816";
const HS88_ID_OFFSET: u32 = 0x4003F0;
const HS88_ID: &[u8] = b"3WS88PWN4";

impl CardKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Hs816 => "3HS816",
            Self::Hs88pwn4 => "3HS88PWN4",
            Self::Unknown => "CARD",
        }
    }

    /// Package file name searched for when the user did not name one.
    pub fn package_file_name(self) -> Option<&'static str> {
        match self {
            Self::Hs816 => Some("3hs816.cpt32card"),
            Self::Hs88pwn4 => Some("3ws88pwn4.cpt32card"),
            Self::Unknown => None,
        }
    }

    /// The kind a requirement asks for, judged only from its ID bytes and offset.
    pub fn of_requirement(requirement: &CardRequirement) -> Self {
        if requirement.id_offset == HS816_ID_OFFSET && requirement.id.starts_with(HS816_ID) {
            Self::Hs816
        } else if requirement.id_offset == HS88_ID_OFFSET && requirement.id.starts_with(HS88_ID) {
            Self::Hs88pwn4
        } else {
            Self::Unknown
        }
    }

    fn detect(card: &mut HostedCard) -> Self {
        let matches = |card: &mut HostedCard, offset: u32, id: &[u8]| {
            id.iter()
                .enumerate()
                .all(|(i, byte)| card.read(offset + i as u32) == *byte)
        };
        if matches(card, HS816_ID_OFFSET, HS816_ID) {
            Self::Hs816
        } else if matches(card, HS88_ID_OFFSET, HS88_ID) {
            Self::Hs88pwn4
        } else {
            Self::Unknown
        }
    }

    /// Writes kept in the display shadow (registers, not sample RAM).
    fn shadows(self, address: u32) -> bool {
        match self {
            Self::Hs816 => address < 0x500,
            Self::Hs88pwn4 => (0x400000..0x400400).contains(&address),
            Self::Unknown => address < 0x10000,
        }
    }

    /// Number of harmonic (8-operator) channels.
    pub fn hs_channels(self) -> usize {
        match self {
            Self::Hs816 => 16,
            Self::Hs88pwn4 => 8,
            Self::Unknown => 0,
        }
    }

    fn hs_base(self, ch: usize) -> u32 {
        match self {
            Self::Hs88pwn4 => 0x400000 + 0x40 * ch as u32,
            _ => 0x40 * ch as u32,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardStatus {
    Ready,
    /// No package was given or found for a required slot.
    Missing,
    /// The package did not load.
    LoadError(String),
    /// The card does not report the ID the stream requires.
    IdMismatch,
    /// The card trapped or ran out of fuel while playing.
    Faulted(String),
}

/// One expansion slot used by the stream.
pub struct CardSlot {
    pub slot: u8,
    pub requirement: CardRequirement,
    pub path: Option<PathBuf>,
    pub kind: CardKind,
    pub status: CardStatus,
    card: Option<HostedCard>,
    /// Last value written to each displayed register (the card is not read back).
    shadow: BTreeMap<u32, u8>,
    /// Frames left on the key-on flash of each harmonic channel.
    pub key_flash: [u8; 16],
    pub muted: bool,
    pub meters: [Meter; 2],
    pub writes: u64,
    /// Writes not delivered because the card is missing, mismatched or faulted.
    pub dropped: u64,
    audio: [Vec<i16>; 2],
}

impl CardSlot {
    fn new(
        requirement: CardRequirement,
        path: Option<PathBuf>,
        resolve_error: Option<String>,
    ) -> Self {
        let mut slot = Self {
            slot: requirement.slot,
            kind: CardKind::of_requirement(&requirement),
            requirement,
            status: CardStatus::Missing,
            card: None,
            shadow: BTreeMap::new(),
            key_flash: [0; 16],
            muted: false,
            meters: [Meter::default(); 2],
            writes: 0,
            dropped: 0,
            audio: [Vec::new(), Vec::new()],
            path: None,
        };
        if let Some(error) = resolve_error {
            slot.status = CardStatus::LoadError(error);
        }
        let Some(path) = path else { return slot };
        match HostedCard::load(slot.slot, &path) {
            Ok(mut card) => {
                let requirement = &slot.requirement;
                let id_ok = requirement.id.iter().enumerate().all(|(i, byte)| {
                    card.read(requirement.id_offset.wrapping_add(i as u32)) == *byte
                });
                slot.kind = CardKind::detect(&mut card);
                if id_ok {
                    slot.status = CardStatus::Ready;
                    slot.card = Some(card);
                } else {
                    slot.status = CardStatus::IdMismatch;
                }
            }
            Err(error) => slot.status = CardStatus::LoadError(error),
        }
        slot.path = Some(path);
        slot
    }

    pub fn is_ready(&self) -> bool {
        self.status == CardStatus::Ready
    }

    /// ID the stream requires, printable.
    pub fn required_id(&self) -> String {
        self.requirement
            .id
            .iter()
            .map(|&byte| {
                if byte.is_ascii_graphic() {
                    char::from(byte).to_string()
                } else {
                    format!("\\x{byte:02X}")
                }
            })
            .collect()
    }

    /// Last written value of a displayed register (0 if never written).
    pub fn reg(&self, address: u32) -> u8 {
        self.shadow.get(&address).copied().unwrap_or(0)
    }

    pub fn reg16(&self, address: u32) -> u16 {
        u16::from_be_bytes([self.reg(address), self.reg(address + 1)])
    }

    fn reset(&mut self) {
        if let Some(card) = self.card.as_mut() {
            card.reset();
        }
        if matches!(self.status, CardStatus::Faulted(_)) {
            self.status = CardStatus::Ready;
        }
        self.shadow.clear();
        self.key_flash = [0; 16];
        self.writes = 0;
        self.dropped = 0;
    }

    fn write(&mut self, address: u32, value: u8) {
        let Some(card) = self
            .card
            .as_mut()
            .filter(|_| self.status == CardStatus::Ready)
        else {
            self.dropped += 1;
            return;
        };
        card.write(address, value);
        if card.is_faulted() {
            let reason = card
                .debug_info()
                .fault_reason
                .unwrap_or_else(|| "faulted".into());
            self.status = CardStatus::Faulted(reason);
        }
        self.writes += 1;
        if self.kind.shadows(address) {
            self.shadow.insert(address, value);
        }
        // Gate (+0x1E) written with 1 is a key-on (retrigger) on both models.
        for ch in 0..self.kind.hs_channels() {
            if address == self.kind.hs_base(ch) + 0x1E && value == 1 {
                self.key_flash[ch] = 8;
            }
        }
    }

    /// Render `n` samples at `offset` of this tick into the slot's own buffers.
    fn render(&mut self, offset: usize, n: usize) {
        for side in &mut self.audio {
            side[offset..offset + n].fill(0);
        }
        let Some(card) = self
            .card
            .as_mut()
            .filter(|_| self.status == CardStatus::Ready)
        else {
            return;
        };
        card.advance_cycles(n as u64 * u64::from(CYCLES_PER_SAMPLE));
        let [left, right] = &mut self.audio;
        card.render_audio(
            SAMPLE_RATE as u32,
            &mut left[offset..offset + n],
            &mut right[offset..offset + n],
        );
        if card.is_faulted() {
            let reason = card
                .debug_info()
                .fault_reason
                .unwrap_or_else(|| "faulted".into());
            self.status = CardStatus::Faulted(reason);
        }
    }

    /// Harmonic channel `ch` as last written.
    pub fn hs(&self, ch: usize) -> HsView {
        let base = self.kind.hs_base(ch);
        let (mute, fb_target) = match self.kind {
            CardKind::Hs88pwn4 => (0x4002D0 + ch as u32, 0x4003C0 + ch as u32),
            _ => (0x480 + ch as u32, 0x490 + ch as u32),
        };
        HsView {
            frequency: self.reg16(base),
            ratios: std::array::from_fn(|op| {
                if op == 0 {
                    4096
                } else {
                    self.reg16(base + 2 * op as u32)
                }
            }),
            volumes: std::array::from_fn(|op| self.reg(base + 0x10 + op as u32)),
            waves: std::array::from_fn(|op| {
                let byte = self.reg(base + 0x18 + op as u32 / 2);
                if op % 2 == 0 { byte >> 4 } else { byte & 15 }
            }),
            algorithm: self.reg(base + 0x1C),
            pan: self.reg(base + 0x1D),
            gate: self.reg(base + 0x1E) == 1,
            feedback: self.reg(base + 0x1F),
            feedback_target: self.reg(fb_target),
            muted: self.reg(mute) != 0,
            key_flash: self.key_flash[ch],
        }
    }

    /// 3HS88PWN4 PCM/WT channel `w` (0..3) as last written.
    pub fn wt(&self, w: usize) -> WtView {
        let base = 0x400200 + 48 * w as u32;
        WtView {
            frequency: self.reg16(base),
            volume: self.reg(base + 2),
            mode: self.reg(base + 3),
            filter: self.reg(base + 4),
            cutoff: self.reg(base + 5),
            resonance: self.reg(base + 6),
            pan: self.reg(base + 7),
            table: std::array::from_fn(|i| self.reg(base + 0x10 + i as u32)),
            muted: self.reg(0x4002D8 + w as u32) != 0,
        }
    }
}

/// Register view of one 8-operator harmonic channel.
#[derive(Clone, Debug)]
pub struct HsView {
    /// OP1 frequency in Hz.
    pub frequency: u16,
    /// Q12 ratios (OP1 is 1.0).
    pub ratios: [u16; 8],
    pub volumes: [u8; 8],
    pub waves: [u8; 8],
    pub algorithm: u8,
    pub pan: u8,
    pub gate: bool,
    pub feedback: u8,
    pub feedback_target: u8,
    pub muted: bool,
    pub key_flash: u8,
}

impl HsView {
    /// L/R pan 0..15; both nibbles 0 mean centre at full level.
    pub fn pan_lr(&self) -> (u8, u8) {
        if self.pan == 0 {
            (15, 15)
        } else {
            (self.pan >> 4, self.pan & 15)
        }
    }
}

/// Register view of one 3HS88PWN4 PCM/WT channel.
#[derive(Clone, Debug)]
pub struct WtView {
    pub frequency: u16,
    pub volume: u8,
    pub mode: u8,
    pub filter: u8,
    pub cutoff: u8,
    pub resonance: u8,
    pub pan: u8,
    pub table: [u8; 32],
    pub muted: bool,
}

impl WtView {
    pub fn mode_name(&self) -> &'static str {
        match self.mode {
            0 => "PCM ",
            1 => "WT  ",
            2 => "NOIS",
            3 => "N64 ",
            4 => "WTNN",
            5 => "FIFO",
            _ => "OFF ",
        }
    }

    pub fn filter_name(&self) -> &'static str {
        if self.cutoff == 0 {
            return "--";
        }
        match self.filter {
            0 => "LP",
            1 => "HP",
            2 => "BP",
            3 => "NT",
            _ => "L2",
        }
    }

    /// PCM mode: start, end (inclusive) and loop start, from the table head.
    pub fn pcm_range(&self) -> (u32, u32, u32) {
        let u24 = |i: usize| {
            u32::from(self.table[i]) << 16
                | u32::from(self.table[i + 1]) << 8
                | u32::from(self.table[i + 2])
        };
        (u24(0), u24(3), u24(6))
    }
}

/// Owns the stream, both SGU cores, the 34SI22, the expansion cards and the
/// analysis buffers.
pub struct Engine {
    bytes: Vec<u8>,
    version: SguVersion,
    player: StreamPlayer,
    cores: [Rc<RefCell<S3w2Sound>>; 2],
    si22: Si22Unit,
    pub cards: Vec<CardSlot>,
    pub transport: Transport,
    /// Frames applied since the start, including repeated passes.
    pub elapsed_frames: u64,
    pub loops: u32,
    pub muted: [bool; VOICES],
    pub channel_meters: [Meter; VOICES],
    pub si22_muted: bool,
    pub si22_meters: [Meter; 2],
    pub master_meters: [Meter; 2],
    /// Writes to slots the stream does not declare (should not happen).
    pub stray_writes: u64,
    pub error: Option<String>,
    /// Cycles until the next frame; <= 0 means a frame is due now.
    until_frame: i64,
    history: [VecDeque<i16>; 2],
}

impl Engine {
    /// Open a stream without expansion cards.
    pub fn new(bytes: Vec<u8>, version: SguVersion) -> Result<Self, MusicError> {
        Self::with_cards(bytes, version, &mut |_, _| Ok(None))
    }

    /// Open a stream; `resolve` names the package for each slot the stream
    /// requires (v4 card table, or slot 0 for v3). `Ok(None)` leaves it missing.
    pub fn with_cards(
        bytes: Vec<u8>,
        version: SguVersion,
        resolve: &mut dyn FnMut(&CardRequirement, CardKind) -> Result<Option<PathBuf>, String>,
    ) -> Result<Self, MusicError> {
        let player = StreamPlayer::from_bytes(&bytes)?;
        let mut requirements = player.card_requirements().to_vec();
        if player.version() == 3 {
            // v3 has no card table: target 3 is a 3WS88PWN4 card in slot 0.
            requirements.push(CardRequirement {
                slot: 0,
                id_offset: HS88_ID_OFFSET,
                id: HS88_ID.to_vec(),
                bulk_start: 0,
                bulk_length: 0x400000,
            });
        }
        let cards = requirements
            .into_iter()
            .map(|requirement| {
                let kind = CardKind::of_requirement(&requirement);
                match resolve(&requirement, kind) {
                    Ok(path) => CardSlot::new(requirement, path, None),
                    Err(error) => CardSlot::new(requirement, None, Some(error)),
                }
            })
            .collect();
        let (cores, pcm_ram) = new_cores(version);
        Ok(Self {
            cores,
            si22: Si22Unit::new(pcm_ram),
            cards,
            player,
            bytes,
            version,
            transport: Transport::Play,
            elapsed_frames: 0,
            loops: 0,
            muted: [false; VOICES],
            channel_meters: [Meter::default(); VOICES],
            si22_muted: false,
            si22_meters: [Meter::default(); 2],
            master_meters: [Meter::default(); 2],
            stray_writes: 0,
            error: None,
            until_frame: 0,
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
    /// Header frame period: 0 = VSync, otherwise 48 MHz cycles (v4).
    pub fn frame_period(&self) -> u32 {
        self.player.frame_period()
    }
    /// Length of one frame in 48 MHz cycles.
    pub fn frame_cycles(&self) -> u32 {
        match self.player.frame_period() {
            0 => VSYNC_FRAME_CYCLES,
            period => period,
        }
    }
    pub fn frames_per_second(&self) -> f64 {
        48_000_000.0 / f64::from(self.frame_cycles())
    }
    pub fn uses_si22(&self) -> bool {
        self.player.uses_si22()
    }
    pub fn si22(&self) -> &Si22 {
        &self.si22.chip
    }
    pub fn core(&self, index: usize) -> std::cell::Ref<'_, S3w2Sound> {
        self.cores[index].borrow()
    }
    pub fn channel(&self, voice: usize) -> ChannelView {
        let (core, channel) = (voice / NUM_CHANNELS, voice % NUM_CHANNELS);
        let mut view = ChannelView::capture(&self.cores[core].borrow(), channel);
        // Trust the last COPY_POOL only while the SRAM still matches it
        // (a later WRITE/FILL or a channel reset changes the bytes).
        view.wave_pool = self.player.wave_source(core, channel).filter(|&id| {
            self.player
                .pool_payload(id)
                .is_some_and(|(_, data)| data == view.wavetable.as_slice())
        });
        view
    }
    /// Most recent `HISTORY` output samples of one side (0 = left).
    pub fn history(&self, side: usize) -> &VecDeque<i16> {
        &self.history[side]
    }

    fn advance_stream(&mut self) {
        if self.error.is_some() || self.player.is_finished() {
            return;
        }
        for card in &mut self.cards {
            for flash in &mut card.key_flash {
                *flash = flash.saturating_sub(1);
            }
        }
        let before = self.player.frame_index();
        let result = self.player.advance_vsync(&self.cores);
        // Deliver device writes even when the frame failed part-way.
        for (target, address, value) in self.player.take_card_writes() {
            if target == SGUB_TARGET_SI22 {
                self.si22.write_register(address, value);
            } else if let Some(card) = self
                .cards
                .iter_mut()
                .find(|card| u32::from(card.slot) + 3 == u32::from(target))
            {
                card.write(address, value);
            } else {
                self.stray_writes += 1;
            }
        }
        if let Err(error) = result {
            self.error = Some(error.to_string());
            return;
        }
        if self.player.is_finished() {
            return;
        }
        if self.player.frame_index() < before {
            self.loops += 1;
        }
        self.elapsed_frames += 1;
    }

    /// Render `samples` output samples (48 kHz). Frames are applied on the
    /// stream's frame period: every 800 samples for VSync streams, every
    /// `frame_period / 1000` samples (with the remainder carried) for v4
    /// timer streams. Paused and stopped engines render silence without
    /// advancing the stream or the device state.
    pub fn tick(&mut self, samples: usize) -> (Vec<i16>, Vec<i16>) {
        let mut left = vec![0i32; samples];
        let mut right = vec![0i32; samples];
        let mut peaks = [0f32; VOICES];
        let mut si22_peaks = [0f32; 2];
        for card in &mut self.cards {
            card.audio = [vec![0; samples], vec![0; samples]];
        }
        if self.transport == Transport::Play {
            let mut done = 0;
            while done < samples {
                if self.until_frame <= 0 {
                    // Register updates must precede synthesis on every device.
                    self.advance_stream();
                    self.until_frame += i64::from(self.frame_cycles());
                }
                let cycles = i64::from(CYCLES_PER_SAMPLE);
                let n = ((self.until_frame + cycles - 1) / cycles).max(1) as usize;
                let n = n.min(samples - done);
                self.render_segment(done, n, &mut left, &mut right, &mut peaks, &mut si22_peaks);
                self.until_frame -= n as i64 * cycles;
                done += n;
            }
        }
        for (meter, peak) in self.channel_meters.iter_mut().zip(peaks) {
            meter.update(peak);
        }
        for (meter, peak) in self.si22_meters.iter_mut().zip(si22_peaks) {
            meter.update(peak);
        }
        let mut left: Vec<i16> = left.into_iter().map(saturate_output).collect();
        let mut right: Vec<i16> = right.into_iter().map(saturate_output).collect();
        // Cards add to the saturated SGU mix, as the emulator's host mixer does.
        for card in &mut self.cards {
            for side in 0..2 {
                let peak = card.audio[side]
                    .iter()
                    .map(|s| i32::from(*s).abs())
                    .max()
                    .unwrap_or(0);
                card.meters[side].update(peak as f32 / 32768.0);
            }
            if !card.muted {
                for i in 0..samples {
                    left[i] = left[i].saturating_add(card.audio[0][i]);
                    right[i] = right[i].saturating_add(card.audio[1][i]);
                }
            }
        }
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

    fn render_segment(
        &mut self,
        offset: usize,
        n: usize,
        left: &mut [i32],
        right: &mut [i32],
        peaks: &mut [f32; VOICES],
        si22_peaks: &mut [f32; 2],
    ) {
        for (core_index, core) in self.cores.iter().enumerate() {
            let output = core.borrow_mut().clock(n);
            for (channel, sides) in output.iter().enumerate() {
                let voice = core_index * NUM_CHANNELS + channel;
                let mut peak = 0i32;
                for i in 0..n {
                    let (l, r) = (i32::from(sides[0][i]), i32::from(sides[1][i]));
                    peak = peak.max(l.abs()).max(r.abs());
                    if !self.muted[voice] {
                        left[offset + i] += l;
                        right[offset + i] += r;
                    }
                }
                // A single channel peaks near 1/4 of full scale (master gain).
                peaks[voice] = peaks[voice].max((peak as f32 / 8192.0).min(1.0));
            }
        }
        // The 34SI22 DAC joins the SGU master mix before the single saturation.
        let mut si22_l = vec![0i32; n];
        let mut si22_r = vec![0i32; n];
        self.si22.clock_mixed_into_i32(&mut si22_l, &mut si22_r);
        for (side, buffer) in [&si22_l, &si22_r].into_iter().enumerate() {
            let peak = buffer.iter().map(|s| s.abs()).max().unwrap_or(0);
            si22_peaks[side] = si22_peaks[side].max((peak as f32 / 32768.0).min(1.0));
        }
        if !self.si22_muted {
            for i in 0..n {
                left[offset + i] += si22_l[i];
                right[offset + i] += si22_r[i];
            }
        }
        for card in &mut self.cards {
            card.render(offset, n);
        }
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
        let (cores, pcm_ram) = new_cores(self.version);
        self.cores = cores;
        self.si22 = Si22Unit::new(pcm_ram);
        for card in &mut self.cards {
            card.reset();
        }
        self.elapsed_frames = 0;
        self.loops = 0;
        self.stray_writes = 0;
        self.error = None;
        self.until_frame = 0;
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

fn new_cores(version: SguVersion) -> ([Rc<RefCell<S3w2Sound>>; 2], Rc<RefCell<Vec<u8>>>) {
    let pcm_ram = Rc::new(RefCell::new(vec![0; PCM_RAM_SIZE]));
    let cores = std::array::from_fn(|_| {
        Rc::new(RefCell::new(S3w2Sound::with_version(
            pcm_ram.clone(),
            version,
        )))
    });
    (cores, pcm_ram)
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
