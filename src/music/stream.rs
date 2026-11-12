use super::project::{Effect, Instrument, MacroStep, Note, ProjectError, Song};
use crate::devices::sgu::s3w2::{PCM_RAM_SIZE, S3w2Sound};
use crate::wav::{WaveError, read_pcm16_wave};
use std::cell::RefCell;
use std::fmt;
use std::fs;
use std::path::Path;
use std::rc::Rc;

const HEADER_SIZE: usize = 20;
const RECORD_SIZE: usize = 5;
const STREAM_VERSION: u8 = 1;
const STREAM_VERSION_2: u8 = 2;
const STREAM_VERSION_3: u8 = 3;
const STREAM_VERSION_4: u8 = 4;
const V2_HEADER_SIZE: usize = 40;
const V4_HEADER_SIZE: usize = 48;
const V4_CARD_ENTRY_SIZE: usize = 32;
const V2_POOL_ENTRY_SIZE: usize = 16;
const SLOT_APERTURE: u32 = 0x0200_0000;
/// SGUB v4 target for the 34SI22 registers (`0x00..0x3F`).
pub const SGUB_TARGET_SI22: u8 = 11;
/// v4 frame period limits in 48 MHz cycles (1 kHz..1 Hz); 0 selects VSync.
const MIN_FRAME_PERIOD: u32 = 48_000;
const MAX_FRAME_PERIOD: u32 = 48_000_000;
const VOICE_COUNT: u8 = 16;
const MAX_STREAM_SIZE: usize = 0x00E0_0000;
const REGISTER_SPACE: usize = 0x900;

#[derive(Debug)]
pub struct MusicError(String);

impl fmt::Display for MusicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for MusicError {}
impl MusicError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl From<ProjectError> for MusicError {
    fn from(error: ProjectError) -> Self {
        Self(error.to_string())
    }
}

impl From<WaveError> for MusicError {
    fn from(error: WaveError) -> Self {
        Self(error.to_string())
    }
}

impl From<std::io::Error> for MusicError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

fn error(message: impl Into<String>) -> MusicError {
    MusicError(message.into())
}

#[derive(Clone, Copy, Debug)]
struct Record {
    target: u8,
    address: u32,
    value: u8,
}

impl Record {
    fn append(self, bytes: &mut Vec<u8>) {
        bytes.push(self.target);
        let address = self.address.to_be_bytes();
        bytes.extend_from_slice(&address[1..]);
        bytes.push(self.value);
    }
}

struct PcmAsset {
    start: u32,
    end: u32,
    loop_address: u32,
    loop_enabled: bool,
    sample_rate: u32,
    samples: Vec<u8>,
}

fn decode_pcm_asset(
    instrument: &Instrument,
    project_dir: &Path,
) -> Result<Option<PcmAsset>, MusicError> {
    let Instrument::Pcm {
        path, loop_start, ..
    } = instrument
    else {
        return Ok(None);
    };
    let asset_path = if path.is_absolute() {
        path.clone()
    } else {
        project_dir.join(path)
    };
    let bytes = fs::read(&asset_path)
        .map_err(|failure| error(format!("PCM asset {}: {failure}", asset_path.display())))?;
    let wave = read_pcm16_wave(&bytes)
        .map_err(|failure| error(format!("PCM asset {}: {failure}", asset_path.display())))?;
    if !(1..=2_097_120).contains(&wave.sample_rate) {
        return Err(error(format!(
            "PCM asset {} sample rate must be 1..=2097120 Hz",
            asset_path.display()
        )));
    }
    let samples: Vec<u8> = if wave.channels == 1 {
        wave.samples
            .iter()
            .map(|sample| ((sample >> 8) + 128).clamp(0, 255) as u8)
            .collect()
    } else {
        wave.samples
            .chunks_exact(2)
            .map(|pair| {
                let average = (i32::from(pair[0]) + i32::from(pair[1])) / 2;
                ((average >> 8) + 128).clamp(0, 255) as u8
            })
            .collect()
    };
    if samples.is_empty() {
        return Err(error(format!(
            "PCM asset {} has no samples",
            asset_path.display()
        )));
    }
    if loop_start.is_some_and(|index| index >= samples.len()) {
        return Err(error(format!(
            "PCM asset {} loop_start must address an existing sample",
            asset_path.display()
        )));
    }
    Ok(Some(PcmAsset {
        start: 0,
        end: 0,
        loop_address: 0,
        loop_enabled: loop_start.is_some(),
        sample_rate: wave.sample_rate,
        samples,
    }))
}

fn load_pcm_assets(song: &Song, project_dir: &Path) -> Result<Vec<Option<PcmAsset>>, MusicError> {
    let mut assets = Vec::with_capacity(song.instruments.len());
    let mut next_address = 0usize;
    for instrument in &song.instruments {
        let Some(mut asset) = decode_pcm_asset(instrument, project_dir)? else {
            assets.push(None);
            continue;
        };
        let end = next_address
            .checked_add(asset.samples.len())
            .ok_or_else(|| error("PCM asset size overflow"))?;
        if end > PCM_RAM_SIZE {
            return Err(error("PCM assets exceed the 1 MiB PCMRAM capacity"));
        }
        asset.start = next_address as u32;
        asset.end = end as u32;
        asset.loop_address = asset.start
            + u32::try_from(match instrument {
                Instrument::Pcm { loop_start, .. } => loop_start.unwrap_or(0),
                _ => unreachable!(),
            })
            .map_err(|_| error("PCM loop address overflow"))?;
        next_address = end;
        assets.push(Some(asset));
    }
    Ok(assets)
}

#[derive(Default)]
struct RegisterCache {
    values: Vec<Option<u8>>,
}

impl RegisterCache {
    fn new() -> Self {
        Self {
            values: vec![None; 2 * REGISTER_SPACE],
        }
    }

    fn index(target: u8, address: u32) -> usize {
        usize::from(target) * REGISTER_SPACE + address as usize
    }

    fn set(&mut self, target: u8, address: u32, value: u8, records: &mut Vec<Record>) {
        let slot = &mut self.values[Self::index(target, address)];
        if *slot != Some(value) {
            *slot = Some(value);
            records.push(Record {
                target,
                address,
                value,
            });
        }
    }
}

#[derive(Default)]
struct Voice {
    selected_instrument: Option<usize>,
    active_instrument: Option<usize>,
    note: u8,
    active: bool,
    effect_volume: Option<u8>,
    effect_pan: Option<u8>,
    macro_step: usize,
    macro_volume: Option<u8>,
    macro_pitch: Option<i8>,
    macro_pan: Option<u8>,
}

impl Voice {
    fn start(&mut self, instrument: usize, note: u8, definition: &Instrument) {
        self.active = true;
        self.active_instrument = Some(instrument);
        self.note = note;
        self.macro_step = 0;
        let (volume, pan) = definition.volume_pan();
        self.macro_volume = Some(volume);
        self.macro_pitch = Some(0);
        self.macro_pan = Some(pan);
        if let Some(step) = definition.macro_data().steps.first() {
            apply_macro_step(self, step);
        }
    }
}

fn apply_macro_step(voice: &mut Voice, step: &MacroStep) {
    if step.volume.is_some() {
        voice.macro_volume = step.volume;
    }
    if step.pitch_semitones.is_some() {
        voice.macro_pitch = step.pitch_semitones;
    }
    if step.pan.is_some() {
        voice.macro_pan = step.pan;
    }
}

fn advance_macro(voice: &mut Voice, song: &Song) {
    let Some(instrument_id) = voice.active_instrument else {
        return;
    };
    let steps = &song.instruments[instrument_id].macro_data().steps;
    if steps.is_empty() {
        return;
    }
    let next = if voice.macro_step + 1 < steps.len() {
        voice.macro_step + 1
    } else {
        song.instruments[instrument_id]
            .macro_data()
            .loop_start
            .unwrap_or(voice.macro_step)
    };
    voice.macro_step = next;
    apply_macro_step(voice, &steps[next]);
}

fn channel_register_base(voice: usize) -> (u8, u32) {
    let chip = (voice / 8) as u8;
    let channel = (voice % 8) as u32;
    (chip, 0x800 + channel * 0x20)
}

fn register(
    cache: &mut RegisterCache,
    records: &mut Vec<Record>,
    target: u8,
    base: u32,
    offset: u32,
    value: u8,
) {
    cache.set(target, base + offset, value, records);
}

fn frequency(note: u8, semitones: i8, base_note: Option<u8>, sample_rate: Option<u32>) -> u16 {
    let pitch = match (base_note, sample_rate) {
        (Some(base), Some(rate)) => {
            f64::from(rate) / 32.0
                * 2.0f64.powf((f64::from(note) - f64::from(base) + f64::from(semitones)) / 12.0)
        }
        _ => 440.0 * 2.0f64.powf((f64::from(note) + f64::from(semitones) - 69.0) / 12.0),
    };
    pitch.round().clamp(0.0, f64::from(u16::MAX)) as u16
}

fn emit_voice(
    voice_id: usize,
    voice: &Voice,
    song: &Song,
    assets: &[Option<PcmAsset>],
    cache: &mut RegisterCache,
    records: &mut Vec<Record>,
    note_started: bool,
) {
    if !voice.active {
        return;
    }
    let instrument_id = voice
        .active_instrument
        .expect("active voice has an instrument");
    let instrument = &song.instruments[instrument_id];
    let (base_volume, base_pan) = instrument.volume_pan();
    let macro_data = instrument.macro_data();
    let macro_has_volume = macro_data.steps.iter().any(|step| step.volume.is_some());
    let macro_has_pitch = macro_data
        .steps
        .iter()
        .any(|step| step.pitch_semitones.is_some());
    let macro_has_pan = macro_data.steps.iter().any(|step| step.pan.is_some());
    let volume = if macro_has_volume && voice.macro_volume.is_some() {
        voice.macro_volume.unwrap()
    } else {
        voice.effect_volume.unwrap_or(base_volume)
    };
    let pan = if macro_has_pan && voice.macro_pan.is_some() {
        voice.macro_pan.unwrap()
    } else {
        voice.effect_pan.unwrap_or(base_pan)
    };
    let semitones = if macro_has_pitch {
        voice.macro_pitch.unwrap_or(0)
    } else {
        0
    };
    let (target, base) = channel_register_base(voice_id);
    if note_started {
        for offset in [0, 1, 2, 3, 4, 0x19] {
            cache.values[RegisterCache::index(target, base + offset)] = None;
        }
    }
    let (waveform, freq, pcm_asset) = match instrument {
        Instrument::Wavetable { samples, .. } => {
            if note_started {
                let first = (voice_id % 8) as u32 * 0x100;
                for (index, sample) in samples.iter().enumerate() {
                    records.push(Record {
                        target,
                        address: first + index as u32,
                        value: *sample,
                    });
                }
            }
            (0, frequency(voice.note, semitones, None, None), None)
        }
        Instrument::Pcm { .. } => {
            let asset = assets[instrument_id]
                .as_ref()
                .expect("validated PCM instrument has an asset");
            (
                1,
                frequency(
                    voice.note,
                    semitones,
                    match instrument {
                        Instrument::Pcm { base_note, .. } => Some(*base_note),
                        _ => None,
                    },
                    Some(asset.sample_rate),
                ),
                Some(asset),
            )
        }
        Instrument::Noise { .. } => (2, frequency(voice.note, semitones, None, None), None),
    };
    register(cache, records, target, base, 0, (freq >> 8) as u8);
    register(cache, records, target, base, 1, freq as u8);
    register(cache, records, target, base, 2, waveform);
    register(cache, records, target, base, 3, volume);
    register(cache, records, target, base, 4, pan);
    if note_started {
        records.push(Record {
            target,
            address: base + 0x0a,
            value: 0,
        });
    }
    if let Some(asset) = pcm_asset {
        for (offset, address) in [asset.start, asset.end, asset.loop_address]
            .into_iter()
            .enumerate()
        {
            let offset = 0x10 + offset as u32 * 3;
            for byte in 0..3 {
                register(
                    cache,
                    records,
                    target,
                    base,
                    offset + byte,
                    (address >> ((2 - byte) * 8)) as u8,
                );
            }
        }
        let control = 1 | if asset.loop_enabled { 2 } else { 0 };
        register(cache, records, target, base, 0x19, control);
    } else {
        register(cache, records, target, base, 0x19, 1);
    }
}

fn stop_voice(voice_id: usize, cache: &mut RegisterCache, records: &mut Vec<Record>) {
    let (target, base) = channel_register_base(voice_id);
    register(cache, records, target, base, 3, 0);
    register(cache, records, target, base, 0x19, 0);
}

fn append_initial_registers(bytes: &mut Vec<u8>, cache: &mut RegisterCache) {
    for voice in 0..usize::from(VOICE_COUNT) {
        let (target, base) = channel_register_base(voice);
        for (offset, value) in [(0, 0), (1, 0), (2, 0), (3, 0), (4, 0xff), (0x19, 0)] {
            Record {
                target,
                address: base + offset,
                value,
            }
            .append(bytes);
            cache.values[RegisterCache::index(target, base + offset)] = Some(value);
        }
    }
}

/// Compile a project-relative song and its PCM assets into an SGUB v1 stream.
pub fn compile_song(song: &Song, project_dir: &Path) -> Result<Vec<u8>, MusicError> {
    song.validate()?;
    if song.orders.is_empty() {
        return Err(error("song has no orders"));
    }
    let assets = load_pcm_assets(song, project_dir)?;
    let pcm_bytes = assets
        .iter()
        .flatten()
        .try_fold(0usize, |sum, asset| sum.checked_add(asset.samples.len()))
        .ok_or_else(|| error("initial record count overflow"))?;
    let initial_count = pcm_bytes
        .checked_add(usize::from(VOICE_COUNT) * 6)
        .ok_or_else(|| error("initial record count overflow"))?;
    let initial_count_u32 =
        u32::try_from(initial_count).map_err(|_| error("initial record count exceeds u32"))?;
    let initial_size = HEADER_SIZE
        .checked_add(
            initial_count
                .checked_mul(RECORD_SIZE)
                .ok_or_else(|| error("stream size overflow"))?,
        )
        .ok_or_else(|| error("stream size overflow"))?;
    if initial_size > MAX_STREAM_SIZE {
        return Err(error("SGUB stream exceeds the 0x00E00000-byte limit"));
    }
    let mut bytes = Vec::with_capacity(initial_size.min(MAX_STREAM_SIZE));
    bytes.extend_from_slice(b"SGUB");
    bytes.extend_from_slice(&[STREAM_VERSION, VOICE_COUNT, u8::from(song.repeat), 0]);
    bytes.extend_from_slice(&0u32.to_be_bytes());
    bytes.extend_from_slice(&initial_count_u32.to_be_bytes());
    bytes.extend_from_slice(&0u32.to_be_bytes());
    for asset in assets.iter().flatten() {
        for (offset, sample) in asset.samples.iter().enumerate() {
            Record {
                target: 2,
                address: asset.start + offset as u32,
                value: *sample,
            }
            .append(&mut bytes);
        }
    }
    let mut cache = RegisterCache::new();
    append_initial_registers(&mut bytes, &mut cache);

    let mut voices: [Voice; 16] = std::array::from_fn(|_| Voice::default());
    let mut current_tempo = song.tempo_bpm;
    let mut tempo_accumulator = 0u16;
    let mut order_index = 0usize;
    let mut row_index = 0usize;
    let mut ticks_into_row = 0u8;
    let mut enter_row = true;
    let mut frame_count = 0u32;

    loop {
        let mut frame_records = Vec::new();
        let mut note_started = [false; 16];
        if enter_row {
            let pattern = &song.patterns[usize::from(song.orders[order_index])];
            let row = &pattern.rows[row_index];
            if let Some(tempo) = row.iter().find_map(|cell| match cell.effect {
                Some(Effect::SetTempo(tempo)) => Some(tempo),
                _ => None,
            }) {
                current_tempo = tempo;
            }
            for (voice_id, cell) in row.iter().enumerate() {
                let voice = &mut voices[voice_id];
                if let Some(instrument_id) = cell.instrument {
                    voice.selected_instrument = Some(usize::from(instrument_id));
                }
                match cell.effect {
                    Some(Effect::SetVolume(value)) => voice.effect_volume = Some(value),
                    Some(Effect::SetPan(value)) => voice.effect_pan = Some(value),
                    Some(Effect::SetTempo(_)) | None => {}
                }
                if let Some(volume) = cell.volume {
                    voice.effect_volume = Some(volume);
                }
                match cell.note {
                    Some(Note::Off) => {
                        voice.active = false;
                        voice.active_instrument = None;
                        stop_voice(voice_id, &mut cache, &mut frame_records);
                    }
                    Some(Note::On(note)) => {
                        if let Some(instrument_id) = voice.selected_instrument {
                            voice.start(instrument_id, note, &song.instruments[instrument_id]);
                            note_started[voice_id] = true;
                        } else {
                            voice.active = false;
                            voice.active_instrument = None;
                            stop_voice(voice_id, &mut cache, &mut frame_records);
                        }
                    }
                    None => {}
                }
            }
            enter_row = false;
        }

        for voice_id in 0..16 {
            if voices[voice_id].active && !note_started[voice_id] {
                advance_macro(&mut voices[voice_id], song);
            }
            emit_voice(
                voice_id,
                &voices[voice_id],
                song,
                &assets,
                &mut cache,
                &mut frame_records,
                note_started[voice_id],
            );
        }
        if frame_records.len() > usize::from(u16::MAX) {
            return Err(error("one frame contains more than 65535 register records"));
        }
        let added = 2usize
            .checked_add(
                frame_records
                    .len()
                    .checked_mul(RECORD_SIZE)
                    .ok_or_else(|| error("stream size overflow"))?,
            )
            .ok_or_else(|| error("stream size overflow"))?;
        if bytes
            .len()
            .checked_add(added)
            .is_none_or(|size| size > MAX_STREAM_SIZE)
        {
            return Err(error("SGUB stream exceeds the 0x00E00000-byte limit"));
        }
        bytes.extend_from_slice(&(frame_records.len() as u16).to_be_bytes());
        for record in frame_records {
            record.append(&mut bytes);
        }
        frame_count = frame_count
            .checked_add(1)
            .ok_or_else(|| error("frame count exceeds u32"))?;

        tempo_accumulator += current_tempo;
        if tempo_accumulator >= 150 {
            tempo_accumulator -= 150;
            ticks_into_row += 1;
            if ticks_into_row == song.ticks_per_row {
                ticks_into_row = 0;
                row_index += 1;
                if row_index
                    == song.patterns[usize::from(song.orders[order_index])]
                        .rows
                        .len()
                {
                    row_index = 0;
                    order_index += 1;
                }
                if order_index == song.orders.len() {
                    break;
                }
                enter_row = true;
            }
        }
    }
    bytes[8..12].copy_from_slice(&frame_count.to_be_bytes());
    bytes[12..16].copy_from_slice(&initial_count_u32.to_be_bytes());
    let total_size = u32::try_from(bytes.len()).map_err(|_| error("stream size exceeds u32"))?;
    bytes[16..20].copy_from_slice(&total_size.to_be_bytes());
    Ok(bytes)
}

/// Compile a song using the SGUB v2 pool/operation format. The v1 compiler is
/// used as the semantic reference; its PCM and full wavetable uploads are
/// compacted into typed pool entries without changing register-write order.
pub fn compile_song_v2(song: &Song, project_dir: &Path) -> Result<Vec<u8>, MusicError> {
    let v1 = compile_song(song, project_dir)?;
    let frame_count = read_u32(&v1, 8).ok_or_else(|| error("invalid internal SGUB v1 output"))?;
    let initial_count =
        read_u32(&v1, 12).ok_or_else(|| error("invalid internal SGUB v1 output"))? as usize;
    let repeat = v1[6];
    let mut cursor = HEADER_SIZE;
    let mut initial = Vec::with_capacity(initial_count);
    let mut pcm = Vec::new();
    for _ in 0..initial_count {
        let rec = decode_v1_record(&v1, cursor)?;
        if rec.target == 2 {
            if rec.address as usize != pcm.len() {
                return Err(error("v2 compiler requires contiguous PCM initialization"));
            }
            pcm.push(rec.value);
        } else {
            initial.push(vec![
                0,
                rec.target,
                (rec.address >> 16) as u8,
                (rec.address >> 8) as u8,
                rec.address as u8,
                rec.value,
            ]);
        }
        cursor += RECORD_SIZE;
    }
    let frames_start = cursor;
    let mut frames: Vec<Vec<Vec<u8>>> = Vec::with_capacity(frame_count as usize);
    let mut pools: Vec<(u8, Vec<u8>)> = Vec::new();
    let mut wave_ids: std::collections::HashMap<Vec<u8>, u16> = std::collections::HashMap::new();
    if !pcm.is_empty() {
        pools.push((2, pcm.clone()));
    }
    if !pcm.is_empty() {
        let tail = PCM_RAM_SIZE - pcm.len();
        if tail > 0 {
            let mut fill = vec![1, 2];
            fill.extend_from_slice(&(pcm.len() as u32).to_be_bytes()[1..]);
            fill.extend_from_slice(&(tail as u32).to_be_bytes());
            fill.push(0x80);
            initial.insert(0, fill);
        }
        initial.push(encode_v2_copy(2, 0, 0, 0, pcm.len() as u32));
    }
    let mut read_cursor = frames_start;
    for _ in 0..frame_count {
        let count = read_u16(&v1, read_cursor)
            .ok_or_else(|| error("invalid internal SGUB frame"))? as usize;
        read_cursor += 2;
        let mut records = Vec::with_capacity(count);
        for _ in 0..count {
            records.push(decode_v1_record(&v1, read_cursor)?);
            read_cursor += RECORD_SIZE;
        }
        let mut frame = Vec::new();
        let mut i = 0;
        while i < records.len() {
            let first = records[i];
            if first.target < 2
                && first.address < 0x800
                && first.address % 256 == 0
                && i + 256 <= records.len()
                && records[i..i + 256].iter().enumerate().all(|(n, record)| {
                    record.target == first.target && record.address == first.address + n as u32
                })
            {
                let wave: Vec<u8> = records[i..i + 256]
                    .iter()
                    .map(|record| record.value)
                    .collect();
                let pool_id = if let Some(id) = wave_ids.get(&wave) {
                    *id
                } else {
                    let id = u16::try_from(pools.len())
                        .map_err(|_| error("too many SGUB v2 pool entries"))?;
                    wave_ids.insert(wave.clone(), id);
                    pools.push((1, wave));
                    id
                };
                frame.push(encode_v2_copy(first.target, first.address, pool_id, 0, 256));
                i += 256;
            } else {
                frame.push(encode_v2_write(first));
                i += 1;
            }
        }
        if frame.len() > u16::MAX as usize {
            return Err(error("one frame contains more than 65535 SGUB operations"));
        }
        frames.push(frame);
    }
    if read_cursor != v1.len() {
        return Err(error("invalid internal SGUB trailing data"));
    }

    let directory_offset = V2_HEADER_SIZE;
    let mut output = vec![0u8; V2_HEADER_SIZE + pools.len() * V2_POOL_ENTRY_SIZE];
    let mut entries = Vec::with_capacity(pools.len());
    for (id, (kind, payload)) in pools.iter().enumerate() {
        let offset = output.len();
        output.extend_from_slice(payload);
        entries.push((id as u16, *kind, payload.len() as u32, offset as u32));
    }
    let initial_offset = output.len();
    for op in &initial {
        output.extend_from_slice(op);
    }
    let frames_offset = output.len();
    for frame in &frames {
        output.extend_from_slice(&(frame.len() as u16).to_be_bytes());
        for op in frame {
            output.extend_from_slice(op);
        }
        if output.len() > MAX_STREAM_SIZE {
            return Err(error("SGUB v2 stream exceeds the 0x00E00000-byte limit"));
        }
    }
    if output.len() > MAX_STREAM_SIZE {
        return Err(error("SGUB v2 stream exceeds the 0x00E00000-byte limit"));
    }
    output[..8].copy_from_slice(&[
        b'S',
        b'G',
        b'U',
        b'B',
        STREAM_VERSION_2,
        VOICE_COUNT,
        repeat,
        0,
    ]);
    output[8..12].copy_from_slice(&frame_count.to_be_bytes());
    output[12..16].copy_from_slice(&(pools.len() as u32).to_be_bytes());
    output[16..20].copy_from_slice(&(initial.len() as u32).to_be_bytes());
    output[20..24].copy_from_slice(&(directory_offset as u32).to_be_bytes());
    output[24..28].copy_from_slice(&(initial_offset as u32).to_be_bytes());
    output[28..32].copy_from_slice(&(frames_offset as u32).to_be_bytes());
    let total_size = output.len() as u32;
    output[32..36].copy_from_slice(&total_size.to_be_bytes());
    output[36..38].copy_from_slice(&(V2_HEADER_SIZE as u16).to_be_bytes());
    for (id, kind, len, offset) in entries {
        let entry = directory_offset + usize::from(id) * V2_POOL_ENTRY_SIZE;
        output[entry..entry + 2].copy_from_slice(&id.to_be_bytes());
        output[entry + 2] = kind;
        output[entry + 3] = 0;
        output[entry + 4..entry + 8].copy_from_slice(&len.to_be_bytes());
        output[entry + 8..entry + 12].copy_from_slice(&len.to_be_bytes());
        output[entry + 12..entry + 16].copy_from_slice(&offset.to_be_bytes());
    }
    Ok(output)
}

fn decode_v1_record(bytes: &[u8], offset: usize) -> Result<Record, MusicError> {
    let record = bytes
        .get(offset..offset + RECORD_SIZE)
        .ok_or_else(|| error("truncated internal SGUB record"))?;
    Ok(Record {
        target: record[0],
        address: (u32::from(record[1]) << 16) | (u32::from(record[2]) << 8) | u32::from(record[3]),
        value: record[4],
    })
}

fn encode_v2_write(record: Record) -> Vec<u8> {
    let mut out = vec![0, record.target];
    out.extend_from_slice(&record.address.to_be_bytes()[1..]);
    out.push(record.value);
    out
}

fn encode_v2_copy(target: u8, address: u32, id: u16, source: u32, length: u32) -> Vec<u8> {
    let mut out = vec![2, target];
    out.extend_from_slice(&address.to_be_bytes()[1..]);
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&source.to_be_bytes());
    out.extend_from_slice(&length.to_be_bytes());
    out
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// One v4 card requirement: the ID a slot must expose and its bulk window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardRequirement {
    pub slot: u8,
    pub id_offset: u32,
    pub id: Vec<u8>,
    pub bulk_start: u32,
    pub bulk_length: u32,
}

fn validate_v4_operation(
    bytes: &[u8],
    offset: usize,
    end: usize,
    pools: &[(u8, usize, usize)],
    cards: &[CardRequirement],
) -> Result<usize, MusicError> {
    let opcode = *bytes
        .get(offset)
        .filter(|_| offset < end)
        .ok_or_else(|| error("truncated SGUB v4 operation"))?;
    let size = match opcode {
        0 => 7,
        1 => 11,
        2 => 16,
        _ => return Err(error("unknown SGUB v4 operation")),
    };
    let op_end = offset
        .checked_add(size)
        .filter(|value| *value <= end)
        .ok_or_else(|| error("truncated SGUB v4 operation"))?;
    let target = bytes[offset + 1];
    let address = read_u32(bytes, offset + 2).unwrap();
    // (inclusive WRITE limit, bulk destination range) per target.
    let (limit, bulk): (u32, Option<(u32, u32)>) = match target {
        0 | 1 => (0x8ff, Some((0, 0x800))),
        2 => (0x0f_ffff, Some((0, 0x10_0000))),
        3..=10 => {
            let slot = target - 3;
            let card = cards
                .iter()
                .find(|card| card.slot == slot)
                .ok_or_else(|| error(format!("SGUB v4 slot {slot} has no card requirement")))?;
            let bulk = (card.bulk_length != 0)
                .then_some((card.bulk_start, card.bulk_start + card.bulk_length));
            (SLOT_APERTURE - 1, bulk)
        }
        SGUB_TARGET_SI22 => (0x3f, None),
        _ => return Err(error("SGUB v4 operation has invalid target")),
    };
    if opcode == 0 {
        if address > limit {
            return Err(error("SGUB v4 WRITE address is out of range"));
        }
        return Ok(op_end);
    }
    let length = read_u32(bytes, offset + if opcode == 1 { 6 } else { 12 }).unwrap();
    let (bulk_start, bulk_end) =
        bulk.ok_or_else(|| error("SGUB v4 target does not accept FILL or COPY_POOL"))?;
    let dest_end = u64::from(address) + u64::from(length);
    if length == 0 || address < bulk_start || dest_end > u64::from(bulk_end) {
        return Err(error("SGUB v4 bulk destination is out of range"));
    }
    if opcode == 2 {
        let id = read_u16(bytes, offset + 6).unwrap() as usize;
        let source = read_u32(bytes, offset + 8).unwrap();
        let (kind, _, pool_len) = pools
            .get(id)
            .copied()
            .ok_or_else(|| error("SGUB v4 COPY_POOL references an invalid pool ID"))?;
        let type_ok = match kind {
            1 => target < 2 && address % 0x100 == 0 && source % 256 == 0 && length % 256 == 0,
            2 => target == 2,
            3 => (3..=10).contains(&target),
            _ => false,
        };
        if !type_ok || u64::from(source) + u64::from(length) > pool_len as u64 {
            return Err(error("SGUB v4 COPY_POOL range or type is invalid"));
        }
    }
    Ok(op_end)
}

fn parse_v4_cards(bytes: &[u8], count: usize) -> Result<Vec<CardRequirement>, MusicError> {
    let mut cards: Vec<CardRequirement> = Vec::with_capacity(count);
    for index in 0..count {
        let entry = bytes
            .get(V4_HEADER_SIZE + index * V4_CARD_ENTRY_SIZE..)
            .and_then(|rest| rest.get(..V4_CARD_ENTRY_SIZE))
            .ok_or_else(|| error("SGUB v4 card table is truncated"))?;
        let slot = entry[0];
        let id_length = entry[1] as usize;
        let id_offset = read_u32(entry, 4).unwrap();
        let bulk_start = read_u32(entry, 8).unwrap();
        let bulk_length = read_u32(entry, 12).unwrap();
        let aperture = u64::from(SLOT_APERTURE);
        if slot > 7
            || cards.last().is_some_and(|last| last.slot >= slot)
            || !(1..=16).contains(&id_length)
            || entry[2] != 0
            || entry[3] != 0
            || entry[16 + id_length..].iter().any(|byte| *byte != 0)
            || u64::from(id_offset) + id_length as u64 > aperture
            || u64::from(bulk_start) + u64::from(bulk_length) > aperture
        {
            return Err(error("SGUB v4 card requirement is invalid"));
        }
        cards.push(CardRequirement {
            slot,
            id_offset,
            id: entry[16..16 + id_length].to_vec(),
            bulk_start,
            bulk_length,
        });
    }
    Ok(cards)
}

fn validate_v2_operation(
    bytes: &[u8],
    offset: usize,
    end: usize,
    pools: &[(u8, usize, usize)],
) -> Result<usize, MusicError> {
    let version3 = bytes.get(4) == Some(&STREAM_VERSION_3);
    let address_size = if version3 { 4 } else { 3 };
    let opcode = *bytes
        .get(offset)
        .filter(|_| offset < end)
        .ok_or_else(|| error("truncated SGUB v2 operation"))?;
    let size = match opcode {
        0 => 3 + address_size,
        1 => 2 + address_size + 5,
        2 => 2 + address_size + 10,
        _ => return Err(error("unknown SGUB v2 operation")),
    };
    let op_end = offset
        .checked_add(size)
        .filter(|value| *value <= end)
        .ok_or_else(|| error("truncated SGUB v2 operation"))?;
    let target = bytes[offset + 1];
    let address = if version3 {
        read_u32(bytes, offset + 2).ok_or_else(|| error("truncated SGUB v3 address"))?
    } else {
        (u32::from(bytes[offset + 2]) << 16)
            | (u32::from(bytes[offset + 3]) << 8)
            | u32::from(bytes[offset + 4])
    };
    let limit = match target {
        0 | 1 => 0x8ffu32,
        2 => 0x0f_ffff,
        3 if version3 => 0x01ff_ffff,
        _ => return Err(error("SGUB operation has invalid target")),
    };
    match opcode {
        0 if address <= limit => {}
        0 => return Err(error("SGUB WRITE address is out of range")),
        1 => {
            let length = read_u32(bytes, offset + 2 + address_size)
                .ok_or_else(|| error("truncated SGUB FILL"))?;
            let dest_limit = if target == 3 { 0x3f_ffff } else { limit };
            if length == 0
                || address
                    .checked_add(length - 1)
                    .is_none_or(|last| last > dest_limit)
                || (target < 2
                    && address
                        .checked_add(length - 1)
                        .is_none_or(|last| last > 0x7ff))
            {
                return Err(error("SGUB v2 FILL range is invalid"));
            }
        }
        2 => {
            let id = read_u16(bytes, offset + 2 + address_size)
                .ok_or_else(|| error("truncated SGUB v2 COPY_POOL"))? as usize;
            let source = read_u32(bytes, offset + 4 + address_size)
                .ok_or_else(|| error("truncated SGUB v2 COPY_POOL"))?
                as usize;
            let length = read_u32(bytes, offset + 8 + address_size)
                .ok_or_else(|| error("truncated SGUB v2 COPY_POOL"))?
                as usize;
            let (kind, _, pool_len) = pools
                .get(id)
                .copied()
                .ok_or_else(|| error("SGUB v2 COPY_POOL references an invalid pool ID"))?;
            let source_end = source
                .checked_add(length)
                .ok_or_else(|| error("SGUB v2 pool source range overflow"))?;
            let dest_end = address
                .checked_add(
                    u32::try_from(length).map_err(|_| error("SGUB v2 copy length overflow"))?,
                )
                .ok_or_else(|| error("SGUB v2 destination range overflow"))?;
            if length == 0
                || source_end > pool_len
                || dest_end == 0
                || dest_end - 1 > if target == 3 { 0x3f_ffff } else { limit }
                || (kind == 1
                    && (target > 1
                        || address > 0x7ff
                        || dest_end - 1 > 0x7ff
                        || address % 0x100 != 0
                        || source % 256 != 0
                        || length % 256 != 0))
                || (kind == 2 && target != 2)
                || (kind == 3 && (!version3 || target != 3 || pool_len > 0x400000))
                || (version3 && target == 3 && kind != 3)
            {
                return Err(error("SGUB v2 COPY_POOL range or type is invalid"));
            }
        }
        _ => unreachable!(),
    }
    Ok(op_end)
}

fn validate_record(bytes: &[u8], offset: usize) -> Result<(), MusicError> {
    let record = bytes
        .get(
            offset
                ..offset
                    .checked_add(RECORD_SIZE)
                    .ok_or_else(|| error("record offset overflow"))?,
        )
        .ok_or_else(|| error("truncated SGUB record"))?;
    let target = record[0];
    let address = (u32::from(record[1]) << 16) | (u32::from(record[2]) << 8) | u32::from(record[3]);
    match target {
        0 | 1 if address <= 0x8ff => Ok(()),
        2 if address <= 0x0f_ffff => Ok(()),
        0 | 1 => Err(error(format!(
            "SGUB SGU address 0x{address:06X} is out of range"
        ))),
        2 => Err(error(format!(
            "SGUB PCMRAM address 0x{address:06X} is out of range"
        ))),
        _ => Err(error(format!("SGUB record has invalid target {target}"))),
    }
}

/// A prevalidated stream cursor. `from_bytes` rejects malformed data before playback.
pub struct StreamPlayer {
    bytes: Vec<u8>,
    version: u8,
    frame_count: u32,
    initial_count: u32,
    pools: Vec<(u8, usize, usize)>,
    initial_start: usize,
    frame_start: usize,
    cursor: usize,
    frame_index: u32,
    repeat: bool,
    initialized: bool,
    finished: bool,
    frame_period: u32,
    cards: Vec<CardRequirement>,
    uses_si22: bool,
    device_writes: Vec<(u8, u32, u8)>,
    /// Last wave pool ID copied to each SGU channel's wave SRAM (core, channel).
    wave_sources: [[Option<u16>; 8]; 2],
}

impl StreamPlayer {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MusicError> {
        if bytes
            .get(4)
            .is_some_and(|version| (STREAM_VERSION_2..=STREAM_VERSION_4).contains(version))
        {
            return Self::from_v2_bytes(bytes);
        }
        if bytes.len() < HEADER_SIZE {
            return Err(error("SGUB header is truncated"));
        }
        if &bytes[..4] != b"SGUB" {
            return Err(error("invalid SGUB magic"));
        }
        if bytes[4] != STREAM_VERSION {
            return Err(error(format!("unsupported SGUB version {}", bytes[4])));
        }
        if bytes[5] != VOICE_COUNT {
            return Err(error(format!("SGUB voice_count must be {VOICE_COUNT}")));
        }
        if bytes[6] & !1 != 0 || bytes[7] != 0 {
            return Err(error("SGUB header contains reserved bits"));
        }
        let frame_count =
            read_u32(bytes, 8).ok_or_else(|| error("SGUB frame_count is truncated"))?;
        let initial_count =
            read_u32(bytes, 12).ok_or_else(|| error("SGUB initial_record_count is truncated"))?;
        let total_size =
            read_u32(bytes, 16).ok_or_else(|| error("SGUB total_size is truncated"))? as usize;
        if frame_count == 0 {
            return Err(error("SGUB stream has no frames"));
        }
        if total_size > MAX_STREAM_SIZE {
            return Err(error("SGUB stream exceeds the 0x00E00000-byte limit"));
        }
        if total_size != bytes.len() {
            return Err(error(format!(
                "SGUB total_size is {total_size}, but file contains {} bytes",
                bytes.len()
            )));
        }
        let initial_records_size = usize::try_from(initial_count)
            .ok()
            .and_then(|count| count.checked_mul(RECORD_SIZE))
            .ok_or_else(|| error("SGUB initial record count overflow"))?;
        let initial_end = HEADER_SIZE
            .checked_add(initial_records_size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| error("SGUB initial records are truncated"))?;
        let mut cursor = HEADER_SIZE;
        for _ in 0..initial_count {
            validate_record(bytes, cursor)?;
            cursor += RECORD_SIZE;
        }
        let frame_start = initial_end;
        for _ in 0..frame_count {
            let count = read_u16(bytes, cursor)
                .ok_or_else(|| error("SGUB frame record count is truncated"))?;
            cursor = cursor
                .checked_add(2)
                .ok_or_else(|| error("SGUB frame offset overflow"))?;
            for _ in 0..count {
                validate_record(bytes, cursor)?;
                cursor = cursor
                    .checked_add(RECORD_SIZE)
                    .ok_or_else(|| error("SGUB frame offset overflow"))?;
            }
            if cursor > bytes.len() {
                return Err(error("SGUB frame records are truncated"));
            }
        }
        if cursor != bytes.len() {
            return Err(error("SGUB stream has extra bytes after the final frame"));
        }
        Ok(Self {
            bytes: bytes.to_vec(),
            version: STREAM_VERSION,
            frame_count,
            initial_count,
            pools: Vec::new(),
            initial_start: HEADER_SIZE,
            frame_start,
            cursor: frame_start,
            frame_index: 0,
            repeat: bytes[6] & 1 != 0,
            initialized: false,
            finished: false,
            frame_period: 0,
            cards: Vec::new(),
            uses_si22: false,
            device_writes: Vec::new(),
            wave_sources: [[None; 8]; 2],
        })
    }

    fn from_v2_bytes(bytes: &[u8]) -> Result<Self, MusicError> {
        let fail = || error("invalid SGUB v2 stream");
        let version = bytes.get(4).copied().unwrap_or(0);
        let version4 = version == STREAM_VERSION_4;
        let header_size = if version4 {
            V4_HEADER_SIZE
        } else {
            V2_HEADER_SIZE
        };
        if bytes.len() < header_size || &bytes[..4] != b"SGUB" {
            return Err(error("SGUB pooled header is truncated"));
        }
        if !(STREAM_VERSION_2..=STREAM_VERSION_4).contains(&version) || bytes[5] != VOICE_COUNT {
            return Err(error("invalid SGUB pooled version or voice count"));
        }
        let (card_count, frame_period) = if version4 {
            (bytes[38] as usize, read_u32(bytes, 40).unwrap())
        } else {
            (0, 0)
        };
        if bytes[6] & !1 != 0
            || bytes[7] != 0
            || read_u16(bytes, 36) != Some(header_size as u16)
            || (!version4 && read_u16(bytes, 38) != Some(0))
            || (version4 && (bytes[39] != 0 || read_u32(bytes, 44) != Some(0) || card_count > 8))
        {
            return Err(error(
                "SGUB pooled header contains invalid flags or reserved values",
            ));
        }
        if frame_period != 0 && !(MIN_FRAME_PERIOD..=MAX_FRAME_PERIOD).contains(&frame_period) {
            return Err(error("SGUB v4 frame period is out of range"));
        }
        let cards = parse_v4_cards(bytes, card_count)?;
        let frame_count = read_u32(bytes, 8).ok_or_else(fail)?;
        let pool_count = read_u32(bytes, 12).ok_or_else(fail)? as usize;
        let initial_count = read_u32(bytes, 16).ok_or_else(fail)?;
        let directory_offset = read_u32(bytes, 20).ok_or_else(fail)? as usize;
        let initial_start = read_u32(bytes, 24).ok_or_else(fail)? as usize;
        let frame_start = read_u32(bytes, 28).ok_or_else(fail)? as usize;
        let total_size = read_u32(bytes, 32).ok_or_else(fail)? as usize;
        if frame_count == 0
            || pool_count > 65_536
            || total_size != bytes.len()
            || total_size > MAX_STREAM_SIZE
            || directory_offset != header_size + card_count * V4_CARD_ENTRY_SIZE
        {
            return Err(fail());
        }
        let directory_end = directory_offset
            .checked_add(
                pool_count
                    .checked_mul(V2_POOL_ENTRY_SIZE)
                    .ok_or_else(fail)?,
            )
            .ok_or_else(fail)?;
        if directory_end > bytes.len() {
            return Err(fail());
        }
        let mut payload_cursor = directory_end;
        let mut pools = Vec::with_capacity(pool_count);
        for id in 0..pool_count {
            let entry = directory_offset + id * V2_POOL_ENTRY_SIZE;
            let entry_id = read_u16(bytes, entry).ok_or_else(fail)? as usize;
            let kind = *bytes.get(entry + 2).ok_or_else(fail)?;
            let codec = *bytes.get(entry + 3).ok_or_else(fail)?;
            let raw_size = read_u32(bytes, entry + 4).ok_or_else(fail)? as usize;
            let stored_size = read_u32(bytes, entry + 8).ok_or_else(fail)? as usize;
            let offset = read_u32(bytes, entry + 12).ok_or_else(fail)? as usize;
            if entry_id != id
                || codec != 0
                || raw_size == 0
                || raw_size != stored_size
                || offset != payload_cursor
            {
                return Err(error("SGUB v2 pool directory is not canonical"));
            }
            match kind {
                1 if raw_size != 256 => {
                    return Err(error("SGUB v2 wave pool entries must be 256 bytes"));
                }
                1 | 2 => {}
                3 if version == STREAM_VERSION_3 && raw_size <= 0x400000 => {}
                3 if version4 && raw_size <= SLOT_APERTURE as usize => {}
                _ => return Err(error("unsupported SGUB pooled stream pool type")),
            }
            if kind == 2 && raw_size > PCM_RAM_SIZE {
                return Err(error("SGUB v2 PCM pool entry exceeds 1 MiB"));
            }
            payload_cursor = offset.checked_add(stored_size).ok_or_else(fail)?;
            if payload_cursor > bytes.len() {
                return Err(error("SGUB v2 pool payload is truncated"));
            }
            pools.push((kind, offset, raw_size));
        }
        if initial_start != payload_cursor
            || initial_start > frame_start
            || frame_start > total_size
        {
            return Err(fail());
        }
        let validate = |cursor: usize, end: usize| {
            if version4 {
                validate_v4_operation(bytes, cursor, end, &pools, &cards)
            } else {
                validate_v2_operation(bytes, cursor, end, &pools)
            }
        };
        let mut uses_si22 = false;
        let mut cursor = initial_start;
        for _ in 0..initial_count {
            let next = validate(cursor, frame_start)?;
            uses_si22 |= version4 && bytes[cursor + 1] == SGUB_TARGET_SI22;
            cursor = next;
        }
        if cursor != frame_start {
            return Err(error(
                "SGUB v2 initial operation count or offset is inconsistent",
            ));
        }
        for _ in 0..frame_count {
            let count = read_u16(bytes, cursor).ok_or_else(fail)? as u32;
            cursor = cursor.checked_add(2).ok_or_else(fail)?;
            for _ in 0..count {
                let next = validate(cursor, total_size)?;
                uses_si22 |= version4 && bytes[cursor + 1] == SGUB_TARGET_SI22;
                cursor = next;
            }
        }
        if cursor != total_size {
            return Err(error("SGUB v2 has trailing data or truncated frames"));
        }
        Ok(Self {
            bytes: bytes.to_vec(),
            version,
            frame_count,
            initial_count,
            pools,
            initial_start,
            frame_start,
            cursor: frame_start,
            frame_index: 0,
            repeat: bytes[6] & 1 != 0,
            initialized: false,
            finished: false,
            frame_period,
            cards,
            uses_si22,
            device_writes: Vec::new(),
            wave_sources: [[None; 8]; 2],
        })
    }

    fn apply_records(
        &self,
        sgus: &[Rc<RefCell<S3w2Sound>>; 2],
        start: usize,
        count: u32,
    ) -> Result<(), MusicError> {
        let mut cursor = start;
        for _ in 0..count {
            let record = self
                .bytes
                .get(cursor..cursor + RECORD_SIZE)
                .ok_or_else(|| error("validated SGUB record became truncated"))?;
            let target = record[0];
            let address =
                (u32::from(record[1]) << 16) | (u32::from(record[2]) << 8) | u32::from(record[3]);
            match target {
                0 | 1 => sgus[target as usize]
                    .borrow_mut()
                    .write_register(address, record[4]),
                2 => {
                    for sgu in sgus {
                        sgu.borrow_mut().write_pcm_ram(address, record[4]);
                    }
                }
                _ => return Err(error("validated SGUB record has an invalid target")),
            }
            cursor += RECORD_SIZE;
        }
        Ok(())
    }

    fn apply_v2_operations(
        &mut self,
        sgus: &[Rc<RefCell<S3w2Sound>>; 2],
        start: usize,
        count: u32,
    ) -> Result<usize, MusicError> {
        let mut cursor = start;
        for _ in 0..count {
            let opcode = self.bytes[cursor];
            let target = self.bytes[cursor + 1];
            let address_size = if self.version >= STREAM_VERSION_3 {
                4
            } else {
                3
            };
            let address = if address_size == 4 {
                read_u32(&self.bytes, cursor + 2).unwrap()
            } else {
                (u32::from(self.bytes[cursor + 2]) << 16)
                    | (u32::from(self.bytes[cursor + 3]) << 8)
                    | u32::from(self.bytes[cursor + 4])
            };
            let data = cursor + 2 + address_size;
            match opcode {
                0 => {
                    let value = self.bytes[data];
                    if target < 2 {
                        sgus[target as usize]
                            .borrow_mut()
                            .write_register(address, value);
                    } else if target == 2 {
                        for sgu in sgus {
                            sgu.borrow_mut().write_pcm_ram(address, value);
                        }
                    } else {
                        self.device_writes.push((target, address, value));
                    }
                    cursor += 2 + address_size + 1;
                }
                1 => {
                    let length = read_u32(&self.bytes, data).unwrap() as usize;
                    let value = self.bytes[data + 4];
                    for index in 0..length {
                        if target < 2 {
                            sgus[target as usize]
                                .borrow_mut()
                                .write_register(address + index as u32, value);
                        } else if target == 2 {
                            for sgu in sgus {
                                sgu.borrow_mut()
                                    .write_pcm_ram(address + index as u32, value);
                            }
                        } else {
                            self.device_writes
                                .push((target, address + index as u32, value));
                        }
                    }
                    cursor += 2 + address_size + 5;
                }
                2 => {
                    let pool_id = read_u16(&self.bytes, data).unwrap() as usize;
                    let source = read_u32(&self.bytes, data + 2).unwrap() as usize;
                    let length = read_u32(&self.bytes, data + 6).unwrap() as usize;
                    let (kind, pool_start, _) = self.pools[pool_id];
                    if kind == 1 && target < 2 && address < 0x800 {
                        // Validated: a whole 256-byte entry at a 0x100 boundary.
                        self.wave_sources[target as usize][(address >> 8) as usize] =
                            Some(pool_id as u16);
                    }
                    for index in 0..length {
                        let value = self.bytes[pool_start + source + index];
                        if target < 2 {
                            sgus[target as usize]
                                .borrow_mut()
                                .write_register(address + index as u32, value);
                        } else if target == 2 {
                            for sgu in sgus {
                                sgu.borrow_mut()
                                    .write_pcm_ram(address + index as u32, value);
                            }
                        } else {
                            self.device_writes
                                .push((target, address + index as u32, value));
                        }
                    }
                    cursor += 2 + address_size + 10;
                }
                _ => return Err(error("validated SGUB v2 operation became invalid")),
            }
        }
        Ok(cursor)
    }

    fn stop_all(&mut self, sgus: &[Rc<RefCell<S3w2Sound>>; 2]) {
        if self.uses_si22 {
            self.device_writes.push((SGUB_TARGET_SI22, 0x0c, 0));
        }
        for sgu in sgus.iter() {
            let mut sgu = sgu.borrow_mut();
            for channel in 0..8u32 {
                let base = 0x800 + channel * 0x20;
                sgu.write_register(base + 3, 0);
                sgu.write_register(base + 0x19, 0);
            }
        }
    }

    /// SGUB format version (1..=4).
    pub fn version(&self) -> u8 {
        self.version
    }

    /// Number of 60 Hz frames in one pass of the stream.
    pub fn frame_count(&self) -> u32 {
        self.frame_count
    }

    /// Frames applied in the current pass (0 before the first VSYNC).
    pub fn frame_index(&self) -> u32 {
        self.frame_index
    }

    /// Whether the header requests restarting at the first frame after the last one.
    pub fn repeats(&self) -> bool {
        self.repeat
    }

    /// True once a non-repeating stream has played its last frame and silenced all voices.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Frame period in 48 MHz cycles; 0 means one frame per VSync (v1..v3
    /// always). Hosts call [`Self::advance_vsync`] once per frame period.
    pub fn frame_period(&self) -> u32 {
        self.frame_period
    }

    /// True when a v4 stream writes the 34SI22 (target 11).
    pub fn uses_si22(&self) -> bool {
        self.uses_si22
    }

    /// v4 card requirements (empty for v1..v3, whose v3 card is slot 0).
    pub fn card_requirements(&self) -> &[CardRequirement] {
        &self.cards
    }

    /// Drain ordered writes for targets the player cannot apply itself:
    /// `(target, address, value)` with target 3..=10 for expansion slots 0..7
    /// and [`SGUB_TARGET_SI22`] for the 34SI22, including the end-of-stream stop.
    /// Wave pool entry last copied by COPY_POOL into `core`'s `channel` wave
    /// SRAM. Later WRITE/FILL to that SRAM do not clear it; compare against
    /// [`Self::pool_payload`] to know whether the SRAM still holds the entry.
    pub fn wave_source(&self, core: usize, channel: usize) -> Option<u16> {
        *self.wave_sources.get(core)?.get(channel)?
    }

    /// Pool entry type (1 wave, 2 PCM, 3 card) and payload.
    pub fn pool_payload(&self, id: u16) -> Option<(u8, &[u8])> {
        let (kind, start, len) = *self.pools.get(usize::from(id))?;
        Some((kind, &self.bytes[start..start + len]))
    }

    pub fn take_card_writes(&mut self) -> Vec<(u8, u32, u8)> {
        std::mem::take(&mut self.device_writes)
    }

    /// Apply initial uploads on the first call, then exactly one frame per VSYNC.
    pub fn advance_vsync(&mut self, sgus: &[Rc<RefCell<S3w2Sound>>; 2]) -> Result<(), MusicError> {
        if self.finished {
            return Ok(());
        }
        if !self.initialized {
            if self.version == STREAM_VERSION {
                self.apply_records(sgus, self.initial_start, self.initial_count)?;
            } else {
                self.apply_v2_operations(sgus, self.initial_start, self.initial_count)?;
            }
            self.initialized = true;
            self.cursor = self.frame_start;
        } else if self.frame_index == self.frame_count {
            if self.repeat {
                self.frame_index = 0;
                self.cursor = self.frame_start;
            } else {
                self.stop_all(sgus);
                self.finished = true;
                return Ok(());
            }
        }
        let count = read_u16(&self.bytes, self.cursor)
            .ok_or_else(|| error("validated SGUB frame is truncated"))? as u32;
        let records_start = self.cursor + 2;
        if self.version == STREAM_VERSION {
            self.apply_records(sgus, records_start, count)?;
            self.cursor = records_start + count as usize * RECORD_SIZE;
        } else {
            self.cursor = self.apply_v2_operations(sgus, records_start, count)?;
        }
        self.frame_index += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::sgu::s3w2::S3w2Sound;

    fn cores() -> [Rc<RefCell<S3w2Sound>>; 2] {
        std::array::from_fn(|_| Rc::new(RefCell::new(S3w2Sound::new())))
    }

    fn temp_dir() -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sgub_{}_{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn wave(samples: &[i16], rate: u32, channels: u16) -> Vec<u8> {
        let data_size = (samples.len() * 2) as u32;
        let align = channels * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36u32 + data_size).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
        bytes.extend_from_slice(&align.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    fn set_note(song: &mut Song, row: usize, channel: usize, note: u8) {
        song.patterns[0].rows[row][channel].note = Some(Note::On(note));
        song.patterns[0].rows[row][channel].instrument = Some(0);
    }

    fn expect_header(stream: &[u8], frame_count: u32, initial_count: u32) {
        assert_eq!(&stream[..4], b"SGUB");
        assert_eq!(&stream[4..8], &[1, 16, 1, 0]);
        assert_eq!(
            u32::from_be_bytes(stream[8..12].try_into().unwrap()),
            frame_count
        );
        assert_eq!(
            u32::from_be_bytes(stream[12..16].try_into().unwrap()),
            initial_count
        );
        assert_eq!(
            u32::from_be_bytes(stream[16..20].try_into().unwrap()) as usize,
            stream.len()
        );
    }

    #[test]
    fn first_vsync_loads_initial_registers_then_plays_a4_on_both_sgu_chips() {
        let mut song = Song::default();
        song.tempo_bpm = 60;
        for voice in 0..16 {
            set_note(&mut song, 0, voice, 69);
        }
        let stream = compile_song(&song, Path::new(".")).unwrap();
        expect_header(&stream, 960, 96);
        let cores = cores();
        let mut player = StreamPlayer::from_bytes(&stream).unwrap();
        player.advance_vsync(&cores).unwrap();
        for voice in 0..16 {
            let (chip, channel) = (voice / 8, voice % 8);
            let mut sgu = cores[chip].borrow_mut();
            let base = 0x800 + channel as u32 * 0x20;
            assert_eq!(sgu.read_register(base), 0x01);
            assert_eq!(sgu.read_register(base + 1), 0xb8);
            assert_eq!(sgu.read_register(base + 2), 0);
            assert_eq!(sgu.read_register(base + 3), 0xff);
            assert_eq!(sgu.read_channel_wavetable(channel as usize, 0), 0x80);
        }
        assert_eq!(cores[0].borrow().channels[0].phase, 0);
        assert_eq!(cores[1].borrow().channels[0].phase, 0);
        let (left, right) = cores[0].borrow_mut().clock_mixed(800);
        assert!(left.iter().any(|sample| *sample > 0) && left.iter().any(|sample| *sample < 0));
        assert_eq!(left, right);
    }

    #[test]
    fn v2_deduplicates_wavetable_pool_and_player_copies_it_to_each_voice() {
        let mut song = Song::default();
        song.tempo_bpm = 150;
        song.ticks_per_row = 1;
        let samples: Vec<u8> = (0..=255).map(|value| value as u8).collect();
        song.instruments[0] = Instrument::Wavetable {
            samples: samples.clone(),
            volume: 255,
            pan: 0xff,
            macro_: Default::default(),
        };
        set_note(&mut song, 0, 0, 69);
        set_note(&mut song, 0, 1, 69);
        let v1 = compile_song(&song, Path::new(".")).unwrap();
        let v2 = compile_song_v2(&song, Path::new(".")).unwrap();
        assert_eq!(&v2[4..8], &[2, 16, 1, 0]);
        assert_eq!(read_u32(&v2, 12), Some(1));
        assert!(v2.len() < v1.len());
        let cores = cores();
        let mut player = StreamPlayer::from_bytes(&v2).unwrap();
        player.advance_vsync(&cores).unwrap();
        for channel in 0..2 {
            let sgu = cores[0].borrow();
            for (index, sample) in samples.iter().enumerate() {
                assert_eq!(sgu.read_channel_wavetable(channel, index as u8), *sample);
            }
        }
    }

    #[test]
    fn lowest_channel_set_tempo_takes_effect_at_row_start() {
        let mut song = Song::default();
        song.tempo_bpm = 150;
        song.ticks_per_row = 1;
        song.patterns[0].rows[0][2].effect = Some(Effect::SetTempo(120));
        song.patterns[0].rows[0][5].effect = Some(Effect::SetTempo(60));
        let stream = compile_song(&song, Path::new(".")).unwrap();
        assert_eq!(u32::from_be_bytes(stream[8..12].try_into().unwrap()), 80);
    }

    #[test]
    fn macro_applies_step_zero_and_advances_each_vsync_with_looping_hold() {
        let mut song = Song::default();
        song.tempo_bpm = 150;
        song.ticks_per_row = 1;
        let Instrument::Wavetable { macro_, .. } = &mut song.instruments[0] else {
            panic!()
        };
        macro_.steps = vec![
            MacroStep {
                volume: Some(100),
                pitch_semitones: None,
                pan: None,
            },
            MacroStep {
                volume: None,
                pitch_semitones: Some(12),
                pan: None,
            },
            MacroStep {
                volume: Some(50),
                pitch_semitones: None,
                pan: Some(0x0f),
            },
        ];
        macro_.loop_start = Some(1);
        set_note(&mut song, 0, 0, 69);
        let stream = compile_song(&song, Path::new(".")).unwrap();
        let cores = cores();
        let mut player = StreamPlayer::from_bytes(&stream).unwrap();
        player.advance_vsync(&cores).unwrap();
        assert_eq!(cores[0].borrow().channels[0].volume, 100);
        player.advance_vsync(&cores).unwrap();
        assert_eq!(cores[0].borrow().channels[0].volume, 100);
        assert_eq!(cores[0].borrow().channels[0].frequency, 880);
        assert_eq!(cores[0].borrow().channels[0].panpot, 0xff);
        player.advance_vsync(&cores).unwrap();
        assert_eq!(cores[0].borrow().channels[0].volume, 50);
        assert_eq!(cores[0].borrow().channels[0].panpot, 0x0f);
    }

    #[test]
    fn pcm_assets_upload_mono_and_stereo_bytes_and_program_loop_registers() {
        let root = temp_dir();
        std::fs::write(
            root.join("mono.wav"),
            wave(&[i16::MIN, 0, i16::MAX], 32_000, 1),
        )
        .unwrap();
        std::fs::write(
            root.join("stereo.wav"),
            wave(&[1000, 2000, -1000, -3000], 44_100, 2),
        )
        .unwrap();
        let mut song = Song::default();
        song.instruments[0] = Instrument::Pcm {
            path: "mono.wav".into(),
            base_note: 69,
            loop_start: Some(2),
            volume: 200,
            pan: 0x34,
            macro_: Default::default(),
        };
        song.instruments.push(Instrument::Pcm {
            path: "stereo.wav".into(),
            base_note: 60,
            loop_start: None,
            volume: 255,
            pan: 0xff,
            macro_: Default::default(),
        });
        song.patterns[0].rows[0][0] = super::super::project::Cell {
            note: Some(Note::On(69)),
            instrument: Some(0),
            volume: None,
            effect: None,
        };
        let stream = compile_song(&song, &root).unwrap();
        let cores = cores();
        let mut player = StreamPlayer::from_bytes(&stream).unwrap();
        player.advance_vsync(&cores).unwrap();
        let core = cores[0].borrow();
        assert_eq!(core.read_pcm_ram(0), 0);
        assert_eq!(core.read_pcm_ram(1), 128);
        assert_eq!(core.read_pcm_ram(2), 255);
        assert_eq!(core.read_pcm_ram(3), 133);
        assert_eq!(core.read_pcm_ram(4), 120);
        let channel = &core.channels[0];
        assert_eq!(channel.pcm_start_addr, 0);
        assert_eq!(channel.pcm_end_addr, 3);
        assert_eq!(channel.pcm_loop_addr, 2);
        assert_eq!(channel.pcm_control, 3);
        assert_eq!(channel.frequency, 1000);
        drop(core);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_pcm_rate_empty_loop_and_ram_overflow() {
        let root = temp_dir();
        std::fs::write(root.join("empty.wav"), wave(&[], 44_100, 1)).unwrap();
        std::fs::write(root.join("fast.wav"), wave(&[0], 2_097_121, 1)).unwrap();
        let mut song = Song::default();
        song.instruments[0] = Instrument::Pcm {
            path: "fast.wav".into(),
            base_note: 60,
            loop_start: None,
            volume: 255,
            pan: 255,
            macro_: Default::default(),
        };
        assert!(
            compile_song(&song, &root)
                .unwrap_err()
                .to_string()
                .contains("sample rate")
        );
        if let Instrument::Pcm { path, .. } = &mut song.instruments[0] {
            *path = "empty.wav".into();
        }
        assert!(
            compile_song(&song, &root)
                .unwrap_err()
                .to_string()
                .contains("no samples")
        );
        std::fs::write(
            root.join("big.wav"),
            wave(&vec![0; PCM_RAM_SIZE], 44_100, 1),
        )
        .unwrap();
        if let Instrument::Pcm {
            path, loop_start, ..
        } = &mut song.instruments[0]
        {
            *path = "big.wav".into();
            *loop_start = Some(PCM_RAM_SIZE);
        }
        assert!(
            compile_song(&song, &root)
                .unwrap_err()
                .to_string()
                .contains("loop_start")
        );
        if let Instrument::Pcm { loop_start, .. } = &mut song.instruments[0] {
            *loop_start = None;
        }
        let at_capacity = compile_song(&song, &root).unwrap();
        let mut player = StreamPlayer::from_bytes(&at_capacity).unwrap();
        let cores = cores();
        player.advance_vsync(&cores).unwrap();
        assert_eq!(
            cores[0].borrow().read_pcm_ram((PCM_RAM_SIZE - 1) as u32),
            128
        );
        song.instruments.push(song.instruments[0].clone());
        assert!(
            compile_song(&song, &root)
                .unwrap_err()
                .to_string()
                .contains("1 MiB")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn minimal_stream(flags: u8, frame_count: u32, frames: &[u8]) -> Vec<u8> {
        let size = (HEADER_SIZE + frames.len()) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"SGUB");
        bytes.extend_from_slice(&[1, 16, flags, 0]);
        bytes.extend_from_slice(&frame_count.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(frames);
        bytes
    }

    #[test]
    fn validates_stream_header_lengths_records_and_trailing_data_before_playback() {
        let valid = minimal_stream(0, 1, &[0, 0]);
        StreamPlayer::from_bytes(&valid).unwrap();
        for end in 0..valid.len() {
            assert!(
                StreamPlayer::from_bytes(&valid[..end]).is_err(),
                "length {end}"
            );
        }
        let mut bad = valid.clone();
        bad[4] = 2;
        assert!(StreamPlayer::from_bytes(&bad).is_err());
        let mut bad = valid.clone();
        bad[5] = 15;
        assert!(StreamPlayer::from_bytes(&bad).is_err());
        let mut bad = valid.clone();
        bad[6] = 2;
        assert!(StreamPlayer::from_bytes(&bad).is_err());
        let mut bad = valid.clone();
        bad[7] = 1;
        assert!(StreamPlayer::from_bytes(&bad).is_err());
        let mut bad = valid.clone();
        bad[8..12].copy_from_slice(&0u32.to_be_bytes());
        assert!(StreamPlayer::from_bytes(&bad).is_err());
        let mut bad = valid.clone();
        bad[16..20].copy_from_slice(&21u32.to_be_bytes());
        assert!(StreamPlayer::from_bytes(&bad).is_err());
        let mut bad = minimal_stream(0, 1, &[0, 1, 0, 0, 9, 0]);
        assert!(StreamPlayer::from_bytes(&bad).is_err());
        bad = minimal_stream(0, 1, &[0, 1, 3, 0, 0, 0]);
        assert!(StreamPlayer::from_bytes(&bad).is_err());
        bad = minimal_stream(0, 1, &[0, 1, 0, 0, 9, 0, 0]);
        assert!(StreamPlayer::from_bytes(&bad).is_err());
        bad = minimal_stream(0, 1, &[0, 0, 0xff]);
        assert!(StreamPlayer::from_bytes(&bad).is_err());
    }

    #[test]
    fn repeat_restarts_first_frame_and_nonrepeat_stops_all_voices() {
        let frame = vec![0, 1, 1, 0, 8, 3, 0xff];
        let repeating = minimal_stream(1, 1, &frame);
        let cores = cores();
        let mut player = StreamPlayer::from_bytes(&repeating).unwrap();
        player.advance_vsync(&cores).unwrap();
        assert_eq!(cores[1].borrow().channels[0].volume, 0xff);
        player.advance_vsync(&cores).unwrap();
        assert_eq!(cores[1].borrow().channels[0].volume, 0xff);
        let nonrepeat = minimal_stream(0, 1, &frame);
        let mut player = StreamPlayer::from_bytes(&nonrepeat).unwrap();
        player.advance_vsync(&cores).unwrap();
        assert_eq!(cores[1].borrow().channels[0].volume, 0xff);
        player.advance_vsync(&cores).unwrap();
        assert_eq!(cores[1].borrow().channels[0].volume, 0);
    }

    // --- SGUB v4 -----------------------------------------------------------

    struct V4 {
        flags: u8,
        period: u32,
        cards: Vec<[u8; 32]>,
        pools: Vec<(u8, Vec<u8>)>,
        initial: Vec<Vec<u8>>,
        frames: Vec<Vec<Vec<u8>>>,
    }

    fn card(slot: u8, id_offset: u32, id: &[u8], bulk: (u32, u32)) -> [u8; 32] {
        let mut entry = [0u8; 32];
        entry[0] = slot;
        entry[1] = id.len() as u8;
        entry[4..8].copy_from_slice(&id_offset.to_be_bytes());
        entry[8..12].copy_from_slice(&bulk.0.to_be_bytes());
        entry[12..16].copy_from_slice(&bulk.1.to_be_bytes());
        entry[16..16 + id.len()].copy_from_slice(id);
        entry
    }

    fn write4(target: u8, address: u32, value: u8) -> Vec<u8> {
        let mut op = vec![0, target];
        op.extend_from_slice(&address.to_be_bytes());
        op.push(value);
        op
    }

    fn fill4(target: u8, address: u32, length: u32, value: u8) -> Vec<u8> {
        let mut op = vec![1, target];
        op.extend_from_slice(&address.to_be_bytes());
        op.extend_from_slice(&length.to_be_bytes());
        op.push(value);
        op
    }

    fn copy4(target: u8, address: u32, pool: u16, source: u32, length: u32) -> Vec<u8> {
        let mut op = vec![2, target];
        op.extend_from_slice(&address.to_be_bytes());
        op.extend_from_slice(&pool.to_be_bytes());
        op.extend_from_slice(&source.to_be_bytes());
        op.extend_from_slice(&length.to_be_bytes());
        op
    }

    impl V4 {
        fn new() -> Self {
            Self {
                flags: 0,
                period: 0,
                cards: Vec::new(),
                pools: Vec::new(),
                initial: Vec::new(),
                frames: vec![Vec::new()],
            }
        }

        fn build(&self) -> Vec<u8> {
            let directory = 48 + 32 * self.cards.len();
            let mut payload_offset = directory + 16 * self.pools.len();
            let mut body = Vec::new();
            for card in &self.cards {
                body.extend_from_slice(card);
            }
            for (id, (kind, data)) in self.pools.iter().enumerate() {
                body.extend_from_slice(&(id as u16).to_be_bytes());
                body.extend_from_slice(&[*kind, 0]);
                body.extend_from_slice(&(data.len() as u32).to_be_bytes());
                body.extend_from_slice(&(data.len() as u32).to_be_bytes());
                body.extend_from_slice(&(payload_offset as u32).to_be_bytes());
                payload_offset += data.len();
            }
            for (_, data) in &self.pools {
                body.extend_from_slice(data);
            }
            let initial_offset = 48 + body.len();
            for op in &self.initial {
                body.extend_from_slice(op);
            }
            let frame_offset = 48 + body.len();
            for frame in &self.frames {
                body.extend_from_slice(&(frame.len() as u16).to_be_bytes());
                for op in frame {
                    body.extend_from_slice(op);
                }
            }
            let total = 48 + body.len();
            let mut bytes = b"SGUB".to_vec();
            bytes.extend_from_slice(&[4, 16, self.flags, 0]);
            for value in [
                self.frames.len(),
                self.pools.len(),
                self.initial.len(),
                directory,
                initial_offset,
                frame_offset,
                total,
            ] {
                bytes.extend_from_slice(&(value as u32).to_be_bytes());
            }
            bytes.extend_from_slice(&48u16.to_be_bytes());
            bytes.extend_from_slice(&[self.cards.len() as u8, 0]);
            bytes.extend_from_slice(&self.period.to_be_bytes());
            bytes.extend_from_slice(&0u32.to_be_bytes());
            bytes.extend_from_slice(&body);
            bytes
        }
    }

    const HS816_ID: &[u8] = b"3HS816\x01";

    #[test]
    fn v4_routes_si22_and_cards_in_order_and_stops_si22() {
        let mut stream = V4::new();
        stream.period = 400_000;
        stream.cards = vec![
            card(0, 0x4003F0, b"3WS88PWN4", (0, 0x400000)),
            card(5, 0x4F8, HS816_ID, (0, 0)),
        ];
        stream.pools = vec![(2, vec![1, 2, 3]), (3, vec![9, 8])];
        stream.initial = vec![copy4(2, 0x100, 0, 0, 3), copy4(3, 0x10, 1, 0, 2)];
        stream.frames = vec![vec![
            write4(SGUB_TARGET_SI22, 0x0c, 0x0f),
            write4(8, 0x1e, 1),
            write4(8, 0x1e, 1),
            write4(0, 0x803, 0x40),
        ]];
        let bytes = stream.build();
        let cores = cores();
        let mut player = StreamPlayer::from_bytes(&bytes).unwrap();
        assert_eq!(player.version(), 4);
        assert_eq!(player.frame_period(), 400_000);
        assert_eq!(player.card_requirements().len(), 2);
        assert_eq!(player.card_requirements()[1].id, HS816_ID);
        player.advance_vsync(&cores).unwrap();
        assert_eq!(cores[0].borrow().read_pcm_ram(0x101), 2);
        assert_eq!(cores[0].borrow().channels[0].volume, 0x40);
        assert_eq!(
            player.take_card_writes(),
            vec![
                (3, 0x10, 9),
                (3, 0x11, 8),
                (SGUB_TARGET_SI22, 0x0c, 0x0f),
                (8, 0x1e, 1),
                (8, 0x1e, 1),
            ]
        );
        player.advance_vsync(&cores).unwrap();
        assert!(player.is_finished());
        assert_eq!(player.take_card_writes(), vec![(SGUB_TARGET_SI22, 0x0c, 0)]);
        assert_eq!(cores[0].borrow().channels[0].volume, 0);
    }

    #[test]
    fn v4_without_si22_does_not_touch_it_at_the_end() {
        let mut stream = V4::new();
        stream.frames = vec![vec![write4(1, 0x803, 1)]];
        let mut player = StreamPlayer::from_bytes(&stream.build()).unwrap();
        let cores = cores();
        player.advance_vsync(&cores).unwrap();
        player.advance_vsync(&cores).unwrap();
        assert!(player.is_finished());
        assert!(player.take_card_writes().is_empty());
    }

    #[test]
    fn v4_rejects_invalid_headers_cards_and_operations() {
        let valid = || {
            let mut stream = V4::new();
            stream.cards = vec![card(2, 0x4F8, HS816_ID, (0, 0))];
            stream.frames = vec![vec![write4(5, 0x1e, 1)]];
            stream
        };
        assert!(StreamPlayer::from_bytes(&valid().build()).is_ok());
        let rejects = |stream: V4| StreamPlayer::from_bytes(&stream.build()).is_err();
        // Frame period range.
        for period in [1, 47_999, 48_000_001] {
            let mut stream = valid();
            stream.period = period;
            assert!(rejects(stream), "period {period}");
        }
        for period in [48_000, 800_000, 48_000_000] {
            let mut stream = valid();
            stream.period = period;
            assert!(!rejects(stream), "period {period}");
        }
        // A slot without a requirement.
        let mut stream = valid();
        stream.frames[0].push(write4(3, 0, 0));
        assert!(rejects(stream));
        // Slots must ascend; IDs need padding of zeros; windows inside the aperture.
        let mut stream = valid();
        stream.cards.push(card(2, 0, b"X", (0, 0)));
        assert!(rejects(stream));
        let mut stream = valid();
        stream.cards[0][30] = 1;
        assert!(rejects(stream));
        let mut stream = valid();
        stream.cards[0] = card(2, 0x01FF_FFFF, b"AB", (0, 0));
        assert!(rejects(stream));
        let mut stream = valid();
        stream.cards[0] = card(2, 0, b"A", (0x01FF_0000, 0x10001));
        assert!(rejects(stream));
        let mut stream = valid();
        stream.cards[0] = card(8, 0, b"A", (0, 0));
        assert!(rejects(stream));
        // 34SI22: WRITE up to 0x3F only, never bulk.
        let mut stream = valid();
        stream.frames[0].push(write4(SGUB_TARGET_SI22, 0x3f, 0));
        assert!(!rejects(stream));
        let mut stream = valid();
        stream.frames[0].push(write4(SGUB_TARGET_SI22, 0x40, 0));
        assert!(rejects(stream));
        let mut stream = valid();
        stream.frames[0].push(fill4(SGUB_TARGET_SI22, 0, 1, 0));
        assert!(rejects(stream));
        // Card bulk only inside the declared window; no window means WRITE only.
        let mut stream = valid();
        stream.frames[0].push(fill4(5, 0, 1, 0));
        assert!(rejects(stream));
        let mut stream = valid();
        stream.cards[0] = card(2, 0x4F8, HS816_ID, (0x100, 0x100));
        stream.frames[0].push(fill4(5, 0x100, 0x100, 0));
        assert!(!rejects(stream));
        let mut stream = valid();
        stream.cards[0] = card(2, 0x4F8, HS816_ID, (0x100, 0x100));
        stream.frames[0].push(fill4(5, 0x101, 0x100, 0));
        assert!(rejects(stream));
        // Pool type and target must agree.
        let mut stream = valid();
        stream.cards[0] = card(2, 0x4F8, HS816_ID, (0, 0x10));
        stream.pools = vec![(2, vec![0; 4])];
        stream.frames[0].push(copy4(5, 0, 0, 0, 4));
        assert!(rejects(stream));
        let mut stream = valid();
        stream.pools = vec![(3, vec![0; 4])];
        stream.frames[0].push(copy4(2, 0, 0, 0, 4));
        assert!(rejects(stream));
        // Reserved targets, header fields and trailing bytes.
        let mut stream = valid();
        stream.frames[0].push(write4(12, 0, 0));
        assert!(rejects(stream));
        let mut bytes = valid().build();
        bytes[44] = 1;
        assert!(StreamPlayer::from_bytes(&bytes).is_err());
        let mut bytes = valid().build();
        bytes[36..38].copy_from_slice(&40u16.to_be_bytes());
        assert!(StreamPlayer::from_bytes(&bytes).is_err());
        let mut bytes = valid().build();
        bytes.push(0);
        let total = bytes.len() as u32;
        bytes[32..36].copy_from_slice(&total.to_be_bytes());
        assert!(StreamPlayer::from_bytes(&bytes).is_err());
    }

    #[test]
    fn v4_repeat_restarts_frames_without_initial_operations() {
        let mut stream = V4::new();
        stream.flags = 1;
        stream.initial = vec![write4(0, 0x803, 7)];
        stream.frames = vec![vec![write4(SGUB_TARGET_SI22, 0x0d, 1)], vec![]];
        let mut player = StreamPlayer::from_bytes(&stream.build()).unwrap();
        let cores = cores();
        for _ in 0..5 {
            player.advance_vsync(&cores).unwrap();
        }
        assert!(!player.is_finished());
        assert_eq!(
            player.take_card_writes(),
            vec![(SGUB_TARGET_SI22, 0x0d, 1); 3]
        );
    }
}

#[cfg(test)]
mod column_tests {
    use super::*;

    #[test]
    fn volume_and_pan_coexist_and_note_after_off_synthesizes_again() {
        let mut song = Song::default();
        song.tempo_bpm = 150;
        song.ticks_per_row = 1;
        song.patterns[0].rows[0][0] = super::super::Cell {
            note: Some(Note::On(69)),
            instrument: Some(0),
            volume: Some(128),
            effect: Some(Effect::SetPan(0xf0)),
        };
        song.patterns[0].rows[1][0].note = Some(Note::Off);
        song.patterns[0].rows[2][0].note = Some(Note::On(69));
        let bytes = compile_song(&song, Path::new(".")).unwrap();
        let cores = std::array::from_fn(|_| Rc::new(RefCell::new(S3w2Sound::new())));
        let mut player = StreamPlayer::from_bytes(&bytes).unwrap();
        player.advance_vsync(&cores).unwrap();
        assert_eq!(cores[0].borrow().channels[0].volume, 128);
        assert_eq!(cores[0].borrow().channels[0].panpot, 0xf0);
        player.advance_vsync(&cores).unwrap();
        let (left, right) = cores[0].borrow_mut().clock_mixed(800);
        assert!(left.iter().chain(&right).all(|sample| *sample == 0));
        player.advance_vsync(&cores).unwrap();
        let (left, right) = cores[0].borrow_mut().clock_mixed(800);
        assert!(left.iter().any(|sample| *sample != 0));
        assert!(right.iter().all(|sample| *sample == 0));
    }
    #[test]
    fn playback_advances_by_each_patterns_actual_row_count() {
        let mut song = Song::default();
        song.tempo_bpm = 150;
        song.ticks_per_row = 1;
        song.repeat = false;
        song.patterns
            .push(crate::music::project::Pattern::default());
        song.patterns[0].rows.truncate(2);
        song.patterns[1].rows.truncate(3);
        song.patterns[0].rows[1][0].note = Some(Note::On(69));
        song.patterns[0].rows[1][0].instrument = Some(0);
        song.patterns[1].rows[2][0].note = Some(Note::Off);
        song.orders = vec![0, 1];

        let bytes = compile_song(&song, Path::new(".")).unwrap();
        assert_eq!(u32::from_be_bytes(bytes[8..12].try_into().unwrap()), 5);
        let cores = std::array::from_fn(|_| Rc::new(RefCell::new(S3w2Sound::new())));
        let mut player = StreamPlayer::from_bytes(&bytes).unwrap();
        for _ in 0..2 {
            player.advance_vsync(&cores).unwrap();
        }
        assert_eq!(cores[0].borrow().channels[0].volume, 255);
        for _ in 2..5 {
            player.advance_vsync(&cores).unwrap();
        }
        assert_eq!(cores[0].borrow().channels[0].volume, 0);
    }
}
