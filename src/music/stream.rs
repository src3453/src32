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
    frame_count: u32,
    initial_count: u32,
    frame_start: usize,
    cursor: usize,
    frame_index: u32,
    repeat: bool,
    initialized: bool,
    finished: bool,
}

impl StreamPlayer {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MusicError> {
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
            frame_count,
            initial_count,
            frame_start,
            cursor: frame_start,
            frame_index: 0,
            repeat: bytes[6] & 1 != 0,
            initialized: false,
            finished: false,
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

    fn stop_all(sgus: &[Rc<RefCell<S3w2Sound>>; 2]) {
        for sgu in sgus.iter() {
            let mut sgu = sgu.borrow_mut();
            for channel in 0..8u32 {
                let base = 0x800 + channel * 0x20;
                sgu.write_register(base + 3, 0);
                sgu.write_register(base + 0x19, 0);
            }
        }
    }

    /// Apply initial uploads on the first call, then exactly one frame per VSYNC.
    pub fn advance_vsync(&mut self, sgus: &[Rc<RefCell<S3w2Sound>>; 2]) -> Result<(), MusicError> {
        if self.finished {
            return Ok(());
        }
        if !self.initialized {
            self.apply_records(sgus, HEADER_SIZE, self.initial_count)?;
            self.initialized = true;
            self.cursor = self.frame_start;
        } else if self.frame_index == self.frame_count {
            if self.repeat {
                self.frame_index = 0;
                self.cursor = self.frame_start;
            } else {
                Self::stop_all(sgus);
                self.finished = true;
                return Ok(());
            }
        }
        let count = read_u16(&self.bytes, self.cursor)
            .ok_or_else(|| error("validated SGUB frame is truncated"))? as u32;
        let records_start = self.cursor + 2;
        self.apply_records(sgus, records_start, count)?;
        self.cursor = records_start + count as usize * RECORD_SIZE;
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
        assert_eq!(cores[0].borrow().channels[0].phase, 0.0);
        assert_eq!(cores[1].borrow().channels[0].phase, 0.0);
        let (left, right) = cores[0].borrow_mut().clock_mixed(800);
        assert!(left.iter().any(|sample| *sample > 0) && left.iter().any(|sample| *sample < 0));
        assert_eq!(left, right);
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
