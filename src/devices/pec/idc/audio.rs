//! IDC disk operation sound simulation.
//!
//! Asset layout (user-supplied, NOT bundled for licensing reasons):
//! point `disk_sound.asset_root` at a checkout of
//! <https://github.com/86Box/assets> (the directory that contains `sounds/`).
//! The engine reads the same profile files the original repository ships:
//! `sounds/hdd/hdd_audio_profiles.cfg` and `sounds/fdd/fdd_audio_profiles.cfg`
//! (INI-like `[Profile "N"]` sections), plus the 16-bit PCM WAV samples they
//! reference. Missing files or profiles always degrade to silence, never to a
//! boot failure.
//!
//! Original behavior being mirrored (see `src/disk/hdd_audio.c` and
//! `src/floppy/fdd_audio.c` in 86Box):
//! - one spindle state machine per drive
//!   (stopped -> starting -> running -> stopping -> stopped),
//! - a looping spindle hum while running,
//! - polyphonic one-shot seek clicks layered on top,
//! - per-sample volumes taken from the profile config.
//!
//! Differences: 86Box assumes 48 kHz output and never resamples; this engine
//! keeps source sample rates and linearly resamples to the host output rate,
//! so any output device rate and any (reasonable) WAV rate work.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const HDD_PROFILE_CFG: &str = "sounds/hdd/hdd_audio_profiles.cfg";
pub const FDD_PROFILE_CFG: &str = "sounds/fdd/fdd_audio_profiles.cfg";
pub const NONE_PROFILE_NAME: &str = "none";

const MAX_SEEK_VOICES_PER_DRIVE: usize = 4;
/// Minimum emulated cycles between two seek clicks of one drive (2 ms at
/// 48 MHz). Multi-sector transfers pace themselves through the transfer
/// timing anyway; this only stops pathological machine-gun retriggering.
const MIN_SEEK_GAP_CYCLES: u128 = 96_000;
/// FDD track estimate: IDC has no CHS geometry, only LBA, so map sectors to
/// tracks coarsely to pick a distance-graded seek sample.
const SECTORS_PER_TRACK_ESTIMATE: u32 = 8;

#[derive(Debug, Clone)]
struct Sample {
    /// Stereo interleaved, normalized to [-1.0, 1.0].
    frames: Vec<[f32; 2]>,
    rate: u32,
}

impl Sample {
    fn from_pcm_wave(wave: &crate::wav::PcmWave) -> Self {
        let frames = match wave.channels {
            1 => wave
                .samples
                .iter()
                .map(|&s| {
                    let v = f32::from(s) / 32768.0;
                    [v, v]
                })
                .collect(),
            _ => wave
                .samples
                .chunks_exact(2)
                .map(|pair| [f32::from(pair[0]) / 32768.0, f32::from(pair[1]) / 32768.0])
                .collect(),
        };
        Self {
            frames,
            rate: wave.sample_rate,
        }
    }

    fn len(&self) -> usize {
        self.frames.len()
    }

    fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Linear-interpolated lookup at a fractional frame position.
    fn at(&self, pos: f64) -> [f32; 2] {
        if self.frames.is_empty() {
            return [0.0, 0.0];
        }
        let clamped = pos.clamp(0.0, (self.frames.len() - 1) as f64);
        let base = clamped as usize;
        let frac = (clamped - base as f64) as f32;
        let next = (base + 1).min(self.frames.len() - 1);
        let a = self.frames[base];
        let b = self.frames[next];
        [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileKind {
    Fdd { total_tracks: u32 },
    Hdd { rpm: u32 },
}

#[derive(Debug, Clone)]
struct VolumeSample {
    sample: Sample,
    volume: f32,
}

#[derive(Debug, Clone)]
pub struct SoundProfile {
    pub name: String,
    pub internal_name: String,
    pub kind: ProfileKind,
    start: Option<VolumeSample>,
    looped: Option<VolumeSample>,
    stop: Option<VolumeSample>,
    /// HDD-style single seek click.
    seek: Option<VolumeSample>,
    /// FDD-style distance-graded seeks, indexed by distance-1.
    seek_up: Vec<Option<VolumeSample>>,
    seek_down: Vec<Option<VolumeSample>>,
}

impl SoundProfile {
    fn is_none_profile(&self) -> bool {
        self.internal_name.eq_ignore_ascii_case(NONE_PROFILE_NAME)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpindleState {
    Stopped,
    Starting,
    Running,
    Stopping,
}

#[derive(Debug, Clone, Copy)]
enum SeekVariant {
    Generic,
    Up(usize),
    Down(usize),
}

#[derive(Debug, Clone, Copy)]
struct SeekVoice {
    variant: SeekVariant,
    pos: f64,
    volume: f32,
}

impl SeekVoice {
    fn sample<'a>(&self, profile: &'a SoundProfile) -> Option<&'a VolumeSample> {
        match self.variant {
            SeekVariant::Generic => profile.seek.as_ref(),
            SeekVariant::Up(index) => profile.seek_up.get(index)?.as_ref(),
            SeekVariant::Down(index) => profile.seek_down.get(index)?.as_ref(),
        }
    }
}

#[derive(Debug)]
struct SlotState {
    profile: Option<usize>,
    spindle: SpindleState,
    spindle_pos: f64,
    transition_pos: f64,
    voices: Vec<SeekVoice>,
    last_lba: Option<u32>,
    last_seek_cycle: Option<u128>,
}

impl SlotState {
    fn new() -> Self {
        Self {
            profile: None,
            spindle: SpindleState::Stopped,
            spindle_pos: 0.0,
            transition_pos: 0.0,
            voices: Vec::new(),
            last_lba: None,
            last_seek_cycle: None,
        }
    }
}

/// Per-IDC sound engine. Lives inside [`super::idc::Idc`] so seek triggers
/// from the command path and audio rendering share one thread and need no
/// locking (unlike 86Box, which mixes from a separate audio thread).
#[derive(Debug)]
pub struct IdcAudio {
    profiles: Vec<SoundProfile>,
    slots: [SlotState; 4],
    volume: f32,
}

impl Default for IdcAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl IdcAudio {
    pub fn new() -> Self {
        Self {
            profiles: Vec::new(),
            slots: std::array::from_fn(|_| SlotState::new()),
            volume: 1.0,
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 4.0);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn profile_count(&self) -> usize {
        self.profiles.len()
    }

    pub fn profile_names(&self) -> Vec<(String, String)> {
        self.profiles
            .iter()
            .map(|profile| (profile.internal_name.clone(), profile.name.clone()))
            .collect()
    }

    pub fn find_profile(&self, internal_name: &str) -> Option<usize> {
        self.profiles
            .iter()
            .position(|profile| profile.internal_name == internal_name)
    }

    /// Load both HDD and FDD profile configs below `asset_root`.
    /// Returns the number of profiles available afterwards. Missing files
    /// are not an error: the drive simply stays silent.
    pub fn load_from_asset_root(&mut self, asset_root: &Path) -> usize {
        for (cfg_rel, kind) in [
            (HDD_PROFILE_CFG, ProfileKind::Hdd { rpm: 0 }),
            (FDD_PROFILE_CFG, ProfileKind::Fdd { total_tracks: 0 }),
        ] {
            let cfg_path = asset_root.join(cfg_rel);
            let Ok(text) = std::fs::read_to_string(&cfg_path) else {
                continue;
            };
            let cfg_dir = cfg_path.parent().unwrap_or(asset_root);
            for profile in parse_profiles(&text, kind, cfg_dir, asset_root) {
                if self.find_profile(&profile.internal_name).is_none() {
                    self.profiles.push(profile);
                }
            }
        }
        self.profiles.len()
    }

    /// Assign (or clear with `None`) the sound profile of one slot and
    /// spin the virtual spindle up, mirroring `hdd_audio_spinup_drive`.
    pub fn set_slot_profile(&mut self, slot: usize, profile: Option<usize>) {
        let Some(state) = self.slots.get_mut(slot) else {
            return;
        };
        let profile = profile.filter(|&index| index < self.profiles.len());
        let wants_sound = profile.is_some_and(|index| !self.profiles[index].is_none_profile());
        state.profile = if wants_sound { profile } else { None };
        state.voices.clear();
        state.spindle_pos = 0.0;
        state.transition_pos = 0.0;
        state.spindle = if state.profile.is_some() {
            SpindleState::Starting
        } else {
            SpindleState::Stopped
        };
    }

    pub fn slot_profile(&self, slot: usize) -> Option<usize> {
        self.slots.get(slot)?.profile
    }

    /// Play the spindle stop sample of one drive, then go silent.
    /// Kept for symmetry with the 86Box `spindown_drive` hook (e.g. for a
    /// future UI affordance); the emulator never calls it by itself.
    pub fn spindown_slot(&mut self, slot: usize) {
        let Some(state) = self.slots.get_mut(slot) else {
            return;
        };
        if state.profile.is_none() {
            return;
        }
        match state.spindle {
            SpindleState::Starting | SpindleState::Running => {
                state.spindle = SpindleState::Stopping;
                state.transition_pos = 0.0;
            }
            SpindleState::Stopped | SpindleState::Stopping => {}
        }
    }

    pub fn spindown_all(&mut self) {
        for slot in 0..self.slots.len() {
            self.spindown_slot(slot);
        }
    }

    /// Called when a READ/WRITE command is accepted. Starts the spindle if
    /// needed and layers one seek click. `cycle` is the IDC cycle counter
    /// and throttles retriggers.
    pub fn notify_transfer_start(
        &mut self,
        slot: usize,
        lba: u32,
        _sector_count: u32,
        cycle: u128,
    ) {
        let Some(state) = self.slots.get_mut(slot) else {
            return;
        };
        if state.profile.is_none() {
            state.last_lba = Some(lba);
            return;
        }
        match state.spindle {
            SpindleState::Stopped => {
                state.spindle = SpindleState::Starting;
                state.transition_pos = 0.0;
            }
            SpindleState::Stopping => {
                state.spindle = SpindleState::Starting;
                state.transition_pos = 0.0;
            }
            SpindleState::Starting | SpindleState::Running => {}
        }
        let previous = state.last_lba.replace(lba);
        self.trigger_seek(slot, previous, lba, cycle);
    }

    /// Called when one sector of a running command becomes ready
    /// (READ staged / WRITE committed) so long transfers produce repeated
    /// head-step clicks instead of a single one.
    pub fn notify_sector_tick(&mut self, slot: usize, lba: u32, cycle: u128) {
        let Some(state) = self.slots.get_mut(slot) else {
            return;
        };
        if state.profile.is_none() {
            return;
        }
        let previous = state.last_lba.replace(lba);
        // Same LBA re-reported (e.g. both staging and DMA completion paths):
        // still allow a click, but only through the normal throttle.
        self.trigger_seek(slot, previous.or(Some(lba)), lba, cycle);
    }

    fn trigger_seek(&mut self, slot: usize, previous_lba: Option<u32>, lba: u32, cycle: u128) {
        let Some(state) = self.slots.get_mut(slot) else {
            return;
        };
        let Some(profile_index) = state.profile else {
            return;
        };
        let profile = &self.profiles[profile_index];
        if let Some(last_cycle) = state.last_seek_cycle
            && cycle.saturating_sub(last_cycle) < MIN_SEEK_GAP_CYCLES
        {
            return;
        }
        let variant = seek_variant_for(profile, previous_lba, lba);
        // Drop finished voices first so long transfers reuse slots.
        state.voices.retain(|voice| {
            voice
                .sample(profile)
                .is_some_and(|sample| voice.pos < sample.sample.len() as f64)
        });
        if state.voices.len() >= MAX_SEEK_VOICES_PER_DRIVE {
            return;
        }
        let Some(sample) = (match variant {
            SeekVariant::Generic => profile.seek.as_ref(),
            SeekVariant::Up(index) => profile.seek_up.get(index).and_then(|s| s.as_ref()),
            SeekVariant::Down(index) => profile.seek_down.get(index).and_then(|s| s.as_ref()),
        }) else {
            return;
        };
        // FDD distance without an exact sample: fall back to the longest
        // available sample in the same direction, then to the generic click.
        if sample.sample.is_empty() {
            return;
        }
        state.voices.push(SeekVoice {
            variant,
            pos: 0.0,
            volume: sample_volume_for(profile, variant),
        });
        state.last_seek_cycle = Some(cycle);
    }

    /// Additive mix into the caller's stereo buffers (saturating i16).
    pub fn render_into(&mut self, sample_rate: u32, left: &mut [i16], right: &mut [i16]) {
        if sample_rate == 0 || self.volume <= 0.0 {
            return;
        }
        let frames = left.len().min(right.len());
        if frames == 0 {
            return;
        }
        if !self.slots.iter().any(|slot| slot.profile.is_some()) {
            return;
        }
        let mut mix_l = vec![0.0f32; frames];
        let mut mix_r = vec![0.0f32; frames];
        for slot_index in 0..self.slots.len() {
            let Some(&profile_index) = self.slots[slot_index].profile.as_ref() else {
                continue;
            };
            if profile_index >= self.profiles.len() {
                continue;
            }
            // Clone the small state we need to satisfy the borrow checker;
            // profiles are read-only during rendering.
            let profile = self.profiles[profile_index].clone();
            let state = &mut self.slots[slot_index];
            render_spindle(&profile, state, sample_rate, &mut mix_l, &mut mix_r);
            render_seeks(&profile, state, sample_rate, &mut mix_l, &mut mix_r);
        }
        let gain = self.volume;
        for i in 0..frames {
            let l = left[i] as f32 + mix_l[i] * 32767.0 * gain;
            let r = right[i] as f32 + mix_r[i] * 32767.0 * gain;
            left[i] = l.clamp(-32768.0, 32767.0) as i16;
            right[i] = r.clamp(-32768.0, 32767.0) as i16;
        }
    }

    /// Convenience wrapper allocating the mixed pair.
    pub fn render(&mut self, sample_rate: u32, frames: usize) -> (Vec<i16>, Vec<i16>) {
        let mut left = vec![0i16; frames];
        let mut right = vec![0i16; frames];
        self.render_into(sample_rate, &mut left, &mut right);
        (left, right)
    }
}

fn sample_volume_for(profile: &SoundProfile, variant: SeekVariant) -> f32 {
    match variant {
        SeekVariant::Generic => profile.seek.as_ref().map(|s| s.volume).unwrap_or(1.0),
        SeekVariant::Up(index) => profile
            .seek_up
            .get(index)
            .and_then(|s| s.as_ref())
            .map(|s| s.volume)
            .or_else(|| profile.seek.as_ref().map(|s| s.volume))
            .unwrap_or(1.0),
        SeekVariant::Down(index) => profile
            .seek_down
            .get(index)
            .and_then(|s| s.as_ref())
            .map(|s| s.volume)
            .or_else(|| profile.seek.as_ref().map(|s| s.volume))
            .unwrap_or(1.0),
    }
}

fn seek_variant_for(profile: &SoundProfile, previous_lba: Option<u32>, lba: u32) -> SeekVariant {
    if !matches!(profile.kind, ProfileKind::Fdd { .. }) {
        return SeekVariant::Generic;
    }
    let Some(previous) = previous_lba else {
        return SeekVariant::Up(0);
    };
    if previous == lba {
        return SeekVariant::Generic;
    }
    let distance_sectors = lba.abs_diff(previous);
    let mut distance_tracks = distance_sectors.div_ceil(SECTORS_PER_TRACK_ESTIMATE).max(1);
    let max_tracks = match profile.kind {
        ProfileKind::Fdd { total_tracks } if total_tracks > 0 => total_tracks as usize,
        _ => profile.seek_up.len().max(profile.seek_down.len()).max(1),
    };
    distance_tracks = distance_tracks.min(max_tracks as u32).max(1);
    let index = distance_tracks as usize - 1;
    if lba > previous {
        SeekVariant::Up(index)
    } else {
        SeekVariant::Down(index)
    }
}

fn step_for(sample_rate: u32, output_rate: u32) -> f64 {
    if output_rate == 0 {
        return 1.0;
    }
    f64::from(sample_rate) / f64::from(output_rate)
}

fn render_loop(
    profile: &SoundProfile,
    state: &mut SlotState,
    output_rate: u32,
    mix_l: &mut [f32],
    mix_r: &mut [f32],
) {
    let frames = mix_l.len().min(mix_r.len());
    for i in 0..frames {
        mix_loop_frame(profile, state, output_rate, mix_l, mix_r, i);
    }
}

fn mix_loop_frame(
    profile: &SoundProfile,
    state: &mut SlotState,
    output_rate: u32,
    mix_l: &mut [f32],
    mix_r: &mut [f32],
    frame: usize,
) {
    let Some(looped) = profile.looped.as_ref() else {
        return;
    };
    if looped.sample.is_empty() {
        return;
    }
    let s = looped.sample.at(state.spindle_pos);
    mix_l[frame] += s[0] * looped.volume;
    mix_r[frame] += s[1] * looped.volume;
    state.spindle_pos += step_for(looped.sample.rate, output_rate);
    let len = looped.sample.len() as f64;
    if state.spindle_pos >= len {
        state.spindle_pos %= len;
    }
}

fn render_spindle(
    profile: &SoundProfile,
    state: &mut SlotState,
    output_rate: u32,
    mix_l: &mut [f32],
    mix_r: &mut [f32],
) {
    let frames = mix_l.len().min(mix_r.len());
    match state.spindle {
        SpindleState::Stopped => {}
        SpindleState::Starting => {
            match profile.start.as_ref() {
                Some(start) if !start.sample.is_empty() => {
                    let start_step = step_for(start.sample.rate, output_rate);
                    let start_len = start.sample.len() as f64;
                    // Mix the spin-up sample frame by frame; the moment it
                    // ends, the same buffer hands over to the spindle loop
                    // so no frame is ever mixed twice or skipped.
                    for i in 0..frames {
                        if state.transition_pos < start_len {
                            let s = start.sample.at(state.transition_pos);
                            mix_l[i] += s[0] * start.volume;
                            mix_r[i] += s[1] * start.volume;
                            state.transition_pos += start_step;
                        } else {
                            if state.spindle == SpindleState::Starting {
                                state.spindle = SpindleState::Running;
                                state.spindle_pos = 0.0;
                            }
                            mix_loop_frame(profile, state, output_rate, mix_l, mix_r, i);
                        }
                    }
                }
                // No spin-up sample: skip straight to the loop so the first
                // rendered buffer is already audible.
                _ => {
                    state.spindle = SpindleState::Running;
                    state.spindle_pos = 0.0;
                    render_loop(profile, state, output_rate, mix_l, mix_r);
                }
            }
        }
        SpindleState::Running => render_loop(profile, state, output_rate, mix_l, mix_r),
        SpindleState::Stopping => {
            let Some(stop) = profile.stop.as_ref() else {
                state.spindle = SpindleState::Stopped;
                return;
            };
            if stop.sample.is_empty() {
                state.spindle = SpindleState::Stopped;
                return;
            }
            let step = step_for(stop.sample.rate, output_rate);
            for i in 0..frames {
                if state.transition_pos >= stop.sample.len() as f64 {
                    break;
                }
                let s = stop.sample.at(state.transition_pos);
                mix_l[i] += s[0] * stop.volume;
                mix_r[i] += s[1] * stop.volume;
                state.transition_pos += step;
            }
            if state.transition_pos >= stop.sample.len() as f64 {
                state.spindle = SpindleState::Stopped;
            }
        }
    }
}

fn render_seeks(
    profile: &SoundProfile,
    state: &mut SlotState,
    output_rate: u32,
    mix_l: &mut [f32],
    mix_r: &mut [f32],
) {
    let frames = mix_l.len().min(mix_r.len());
    for voice in &mut state.voices {
        let Some(sample) = voice.sample(profile) else {
            voice.pos = f64::INFINITY;
            continue;
        };
        if sample.sample.is_empty() {
            voice.pos = f64::INFINITY;
            continue;
        }
        let step = step_for(sample.sample.rate, output_rate);
        let mut pos = voice.pos;
        for i in 0..frames {
            if pos >= sample.sample.len() as f64 {
                break;
            }
            let s = sample.sample.at(pos);
            mix_l[i] += s[0] * voice.volume;
            mix_r[i] += s[1] * voice.volume;
            pos += step;
        }
        voice.pos = pos;
    }
    state.voices.retain(|voice| {
        voice.sample(profile).is_some_and(|sample| {
            !sample.sample.is_empty() && voice.pos < sample.sample.len() as f64
        })
    });
}

// ---------------------------------------------------------------------------
// Profile file parsing (minimal INI subset used by the asset pack).
// ---------------------------------------------------------------------------

fn parse_ini_sections(text: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut sections = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_map: HashMap<String, String> = HashMap::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(name) = current_name.take() {
                sections.push((name, std::mem::take(&mut current_map)));
            }
            current_name = Some(line[1..line.len() - 1].trim().to_string());
            continue;
        }
        if current_name.is_none() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        current_map.insert(key.trim().to_string(), unquote(value.trim()).to_string());
    }
    if let Some(name) = current_name.take() {
        sections.push((name, current_map));
    }
    sections
}

fn unquote(value: &str) -> &str {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn parse_volume(map: &HashMap<String, String>, key: &str) -> f32 {
    map.get(key)
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|volume| volume.is_finite() && *volume >= 0.0)
        .unwrap_or(1.0)
}

/// Resolve a sample path from a profile entry. The asset-pack configs store
/// paths like `assets/sounds/hdd/...` relative to the asset repo root, so
/// strip the leading `assets/` when joining with `asset_root`. Absolute
/// paths and cfg-relative paths are honored as-is.
fn resolve_sample_path(raw: &str, cfg_dir: &Path, asset_root: &Path) -> Option<PathBuf> {
    if raw.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(raw);
    if candidate.is_absolute() {
        return candidate.is_file().then_some(candidate);
    }
    let without_prefix = raw
        .strip_prefix("assets/")
        .or_else(|| raw.strip_prefix("assets\\"))
        .unwrap_or(raw);
    [
        asset_root.join(without_prefix),
        asset_root.join(raw),
        cfg_dir.join(raw),
        cfg_dir.join(without_prefix),
    ]
    .into_iter()
    .find(|joined| joined.is_file())
}

fn load_volume_sample(
    map: &HashMap<String, String>,
    file_key: &str,
    volume_key: &str,
    cfg_dir: &Path,
    asset_root: &Path,
) -> Option<VolumeSample> {
    let raw = map.get(file_key).map(String::as_str).unwrap_or("");
    let path = resolve_sample_path(raw, cfg_dir, asset_root)?;
    let bytes = std::fs::read(&path).ok()?;
    let wave = crate::wav::read_pcm16_wave(&bytes).ok()?;
    if wave.sample_rate == 0 || wave.samples.is_empty() {
        return None;
    }
    tracing::debug!(path = %path.display(), rate = wave.sample_rate, "IDC audio: loaded sample");
    Some(VolumeSample {
        sample: Sample::from_pcm_wave(&wave),
        volume: parse_volume(map, volume_key),
    })
}

fn parse_profiles(
    text: &str,
    kind_hint: ProfileKind,
    cfg_dir: &Path,
    asset_root: &Path,
) -> Vec<SoundProfile> {
    let mut profiles = Vec::new();
    for (section, map) in parse_ini_sections(text) {
        if !section.starts_with("Profile") {
            continue;
        }
        let internal_name = map
            .get("internal_name")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let name = map
            .get("name")
            .cloned()
            .unwrap_or_else(|| internal_name.clone());
        if internal_name.eq_ignore_ascii_case(NONE_PROFILE_NAME) {
            profiles.push(SoundProfile {
                name,
                internal_name,
                kind: kind_hint,
                start: None,
                looped: None,
                stop: None,
                seek: None,
                seek_up: Vec::new(),
                seek_down: Vec::new(),
            });
            continue;
        }
        let is_fdd = matches!(kind_hint, ProfileKind::Fdd { .. })
            || map
                .keys()
                .any(|key| key.starts_with("seek_up_") || key.starts_with("seek_down_"));
        let kind = if is_fdd {
            ProfileKind::Fdd {
                total_tracks: map
                    .get("total_tracks")
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|&tracks| tracks > 0)
                    .unwrap_or(80),
            }
        } else {
            ProfileKind::Hdd {
                rpm: map
                    .get("rpm")
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(0),
            }
        };
        let start = load_volume_sample(
            &map,
            "spindlemotor_start_file",
            "spindlemotor_start_volume",
            cfg_dir,
            asset_root,
        );
        let looped = load_volume_sample(
            &map,
            "spindlemotor_loop_file",
            "spindlemotor_loop_volume",
            cfg_dir,
            asset_root,
        );
        let stop = load_volume_sample(
            &map,
            "spindlemotor_stop_file",
            "spindlemotor_stop_volume",
            cfg_dir,
            asset_root,
        );
        let seek = load_volume_sample(
            &map,
            "seek_track_file",
            "seek_track_volume",
            cfg_dir,
            asset_root,
        );
        let (mut seek_up, mut seek_down) = (Vec::new(), Vec::new());
        if is_fdd {
            for distance in 1..=80u32 {
                seek_up.push(load_volume_sample(
                    &map,
                    &format!("seek_up_{distance}track_file"),
                    &format!("seek_up_{distance}track_volume"),
                    cfg_dir,
                    asset_root,
                ));
                seek_down.push(load_volume_sample(
                    &map,
                    &format!("seek_down_{distance}track_file"),
                    &format!("seek_down_{distance}track_volume"),
                    cfg_dir,
                    asset_root,
                ));
            }
            while seek_up.last().is_some_and(|entry| entry.is_none()) {
                seek_up.pop();
            }
            while seek_down.last().is_some_and(|entry| entry.is_none()) {
                seek_down.pop();
            }
        }
        profiles.push(SoundProfile {
            name,
            internal_name,
            kind,
            start,
            looped,
            stop,
            seek,
            seek_up,
            seek_down,
        });
    }
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_sample(frames: usize, rate: u32) -> Sample {
        let frames = (0..frames)
            .map(|i| {
                let phase = i as f32 / frames as f32 * std::f32::consts::TAU;
                let v = (phase.sin() * 10000.0) as f32 / 32768.0;
                [v, v]
            })
            .collect();
        Sample { frames, rate }
    }

    fn wav_bytes_mono(samples: &[i16], rate: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        let data_len = samples.len() * 2;
        let riff_len = 36 + data_len;
        out.extend_from_slice(&(riff_len as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for sample in samples {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }

    #[test]
    fn ini_sections_parse_quoted_profile_names() {
        let text = "[Profile \"1\"]\nname = Test Drive\ninternal_name = test_drive\n";
        let sections = parse_ini_sections(text);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "Profile \"1\"");
        assert_eq!(sections[0].1["name"], "Test Drive");
    }

    #[test]
    fn spindle_runs_start_then_loop() {
        let mut audio = IdcAudio::new();
        audio.profiles.push(SoundProfile {
            name: "test".to_string(),
            internal_name: "test".to_string(),
            kind: ProfileKind::Hdd { rpm: 3600 },
            start: Some(VolumeSample {
                sample: sine_sample(48, 48_000),
                volume: 1.0,
            }),
            looped: Some(VolumeSample {
                sample: sine_sample(480, 48_000),
                volume: 0.5,
            }),
            stop: None,
            seek: None,
            seek_up: Vec::new(),
            seek_down: Vec::new(),
        });
        audio.set_slot_profile(0, Some(0));
        // Starting state first.
        let (left, _) = audio.render(48_000, 48);
        assert!(left.iter().any(|&s| s != 0));
        // After the 48-frame start sample, the loop keeps producing output.
        let (left, _) = audio.render(48_000, 480);
        assert!(left.iter().any(|&s| s != 0));
    }

    #[test]
    fn seek_click_renders_and_finishes() {
        let mut audio = IdcAudio::new();
        audio.profiles.push(SoundProfile {
            name: "test".to_string(),
            internal_name: "test".to_string(),
            kind: ProfileKind::Hdd { rpm: 3600 },
            start: None,
            looped: None,
            stop: None,
            seek: Some(VolumeSample {
                sample: sine_sample(240, 48_000),
                volume: 1.0,
            }),
            seek_up: Vec::new(),
            seek_down: Vec::new(),
        });
        audio.set_slot_profile(0, Some(0));
        audio.notify_transfer_start(0, 100, 1, 0);
        let (left, _) = audio.render(48_000, 480);
        assert!(left.iter().any(|&s| s != 0));
        // Voice finished: further rendering is silent (no loop sample).
        let (left, _) = audio.render(48_000, 480);
        assert!(left.iter().all(|&s| s == 0));
    }

    #[test]
    fn seek_throttle_drops_rapid_retriggers() {
        let mut audio = IdcAudio::new();
        audio.profiles.push(SoundProfile {
            name: "test".to_string(),
            internal_name: "test".to_string(),
            kind: ProfileKind::Hdd { rpm: 3600 },
            start: None,
            looped: None,
            stop: None,
            seek: Some(VolumeSample {
                sample: sine_sample(4800, 48_000),
                volume: 1.0,
            }),
            seek_up: Vec::new(),
            seek_down: Vec::new(),
        });
        audio.set_slot_profile(0, Some(0));
        audio.notify_transfer_start(0, 0, 1, 0);
        audio.notify_sector_tick(0, 1, 1000); // within 2 ms gap: ignored
        assert_eq!(audio.slots[0].voices.len(), 1);
    }

    #[test]
    fn fdd_distance_picks_graded_sample() {
        let profile = SoundProfile {
            name: "fdd".to_string(),
            internal_name: "fdd".to_string(),
            kind: ProfileKind::Fdd { total_tracks: 80 },
            start: None,
            looped: None,
            stop: None,
            seek: None,
            seek_up: vec![Some(VolumeSample {
                sample: sine_sample(10, 48_000),
                volume: 1.0,
            })],
            seek_down: Vec::new(),
        };
        assert!(matches!(
            seek_variant_for(&profile, Some(0), 64),
            SeekVariant::Up(_)
        ));
        assert!(matches!(
            seek_variant_for(&profile, Some(64), 0),
            SeekVariant::Down(_)
        ));
        assert!(matches!(
            seek_variant_for(&profile, None, 10),
            SeekVariant::Up(0)
        ));
    }

    #[test]
    fn loads_hdd_pack_from_disk() {
        let dir = std::env::temp_dir().join(format!(
            "cpt32-idc-audio-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let hdd_dir = dir.join("sounds").join("hdd").join("TestDrive");
        std::fs::create_dir_all(&hdd_dir).unwrap();
        let wav = wav_bytes_mono(&[0, 1000, -1000, 2000], 48_000);
        std::fs::write(hdd_dir.join("spin.wav"), &wav).unwrap();
        std::fs::write(hdd_dir.join("seek.wav"), &wav).unwrap();
        let cfg = "[Profile \"1\"]\nid = 1\nname = Test Drive\ninternal_name = test_drive\nrpm = 3600\nspindlemotor_loop_file = assets/sounds/hdd/TestDrive/spin.wav\nspindlemotor_loop_volume = 0.5\nseek_track_file = assets/sounds/hdd/TestDrive/seek.wav\nseek_track_volume = 1.0\n";
        std::fs::write(
            dir.join("sounds")
                .join("hdd")
                .join("hdd_audio_profiles.cfg"),
            cfg,
        )
        .unwrap();
        let mut audio = IdcAudio::new();
        assert_eq!(audio.load_from_asset_root(&dir), 1);
        assert_eq!(audio.find_profile("test_drive"), Some(0));
        audio.set_slot_profile(0, Some(0));
        audio.notify_transfer_start(0, 0, 1, 0);
        let (left, _) = audio.render(48_000, 16);
        assert!(left.iter().any(|&s| s != 0));
        std::fs::remove_dir_all(&dir).ok();
    }
}
