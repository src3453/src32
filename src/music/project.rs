use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Component, Path, PathBuf};

pub const FORMAT_VERSION: u16 = 1;
pub const CHANNEL_COUNT: usize = 16;
pub const ROW_COUNT: usize = 64;
pub const MAX_PATTERN_ROWS: usize = 256;
pub const MAX_ITEMS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Song {
    pub format_version: u16,
    pub tempo_bpm: u16,
    pub ticks_per_row: u8,
    pub repeat: bool,
    pub orders: Vec<u16>,
    pub patterns: Vec<Pattern>,
    pub instruments: Vec<Instrument>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pattern {
    pub rows: Vec<Vec<Cell>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Note>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<Effect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Note {
    On(u8),
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Effect {
    SetVolume(u8),
    SetPan(u8),
    SetTempo(u16),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Instrument {
    Wavetable {
        samples: Vec<u8>,
        volume: u8,
        pan: u8,
        #[serde(default, rename = "macro")]
        macro_: Macro,
    },
    Pcm {
        path: PathBuf,
        base_note: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        loop_start: Option<usize>,
        volume: u8,
        pan: u8,
        #[serde(default, rename = "macro")]
        macro_: Macro,
    },
    Noise {
        volume: u8,
        pan: u8,
        #[serde(default, rename = "macro")]
        macro_: Macro,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Macro {
    #[serde(default)]
    pub steps: Vec<MacroStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_start: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroStep {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch_semitones: Option<i8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<u8>,
}

#[derive(Debug)]
pub enum ProjectError {
    Io(std::io::Error),
    Decode(toml::de::Error),
    Encode(toml::ser::Error),
    Validation(String),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "project I/O: {error}"),
            Self::Decode(error) => write!(f, "invalid project TOML: {error}"),
            Self::Encode(error) => write!(f, "cannot encode project TOML: {error}"),
            Self::Validation(message) => write!(f, "invalid project: {message}"),
        }
    }
}

impl std::error::Error for ProjectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Decode(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::Validation(_) => None,
        }
    }
}

impl From<std::io::Error> for ProjectError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

fn invalid(message: impl Into<String>) -> ProjectError {
    ProjectError::Validation(message.into())
}

impl Default for Pattern {
    fn default() -> Self {
        Self {
            rows: vec![vec![Cell::default(); CHANNEL_COUNT]; ROW_COUNT],
        }
    }
}

impl Default for Instrument {
    fn default() -> Self {
        let samples = (0..256)
            .map(|index| {
                (128.0 + 127.0 * (std::f64::consts::TAU * index as f64 / 256.0).sin()).round() as u8
            })
            .collect();
        Self::Wavetable {
            samples,
            volume: 255,
            pan: 0xff,
            macro_: Macro::default(),
        }
    }
}

impl Instrument {
    pub fn macro_data(&self) -> &Macro {
        match self {
            Self::Wavetable { macro_, .. }
            | Self::Pcm { macro_, .. }
            | Self::Noise { macro_, .. } => macro_,
        }
    }

    pub fn macro_data_mut(&mut self) -> &mut Macro {
        match self {
            Self::Wavetable { macro_, .. }
            | Self::Pcm { macro_, .. }
            | Self::Noise { macro_, .. } => macro_,
        }
    }

    pub fn volume_pan(&self) -> (u8, u8) {
        match self {
            Self::Wavetable { volume, pan, .. }
            | Self::Pcm { volume, pan, .. }
            | Self::Noise { volume, pan, .. } => (*volume, *pan),
        }
    }
}

impl Macro {
    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.steps.len() > MAX_ITEMS {
            return Err(invalid("macro has more than 256 steps"));
        }
        if self
            .loop_start
            .is_some_and(|index| index >= self.steps.len())
        {
            return Err(invalid("macro loop_start must address an existing step"));
        }
        for (index, step) in self.steps.iter().enumerate() {
            if step
                .pitch_semitones
                .is_some_and(|pitch| !(-48..=48).contains(&pitch))
            {
                return Err(invalid(format!(
                    "macro step {index} pitch must be -48..=48"
                )));
            }
        }
        Ok(())
    }
}

impl Default for Song {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            tempo_bpm: 120,
            ticks_per_row: 6,
            repeat: true,
            orders: vec![0],
            patterns: vec![Pattern::default()],
            instruments: vec![Instrument::default()],
        }
    }
}

impl Song {
    pub fn load(path: &Path) -> Result<Self, ProjectError> {
        let text = std::fs::read_to_string(path)?;
        let song: Self = toml::from_str(&text).map_err(ProjectError::Decode)?;
        song.validate()?;
        Ok(song)
    }

    /// Saves the model without opening PCM assets. Paths are relative to this project's directory.
    pub fn save(&self, path: &Path) -> Result<(), ProjectError> {
        self.validate()?;
        let text = toml::to_string_pretty(self).map_err(ProjectError::Encode)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Writes a rebased copy and returns it; callers replace their model only after success.
    pub fn save_as(&self, path: &Path, old_project_dir: &Path) -> Result<Self, ProjectError> {
        let new_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let song = self.rebased(old_project_dir, new_dir)?;
        song.save(path)?;
        Ok(song)
    }

    /// Preserves PCM targets without requiring the files or directories to exist.
    /// Assets on a different filesystem volume remain absolute paths.
    pub fn rebased(
        &self,
        old_project_dir: &Path,
        new_project_dir: &Path,
    ) -> Result<Self, ProjectError> {
        let old_dir = absolute_normalized(old_project_dir)?;
        let new_dir = absolute_normalized(new_project_dir)?;
        let mut song = self.clone();
        for instrument in &mut song.instruments {
            if let Instrument::Pcm { path, .. } = instrument {
                let target = if path.is_absolute() {
                    absolute_normalized(path)?
                } else {
                    absolute_normalized(&old_dir.join(&*path))?
                };
                *path = relative_path(&target, &new_dir).unwrap_or(target);
            }
        }
        Ok(song)
    }

    /// Checks schema constraints only; WAV contents and PCM loop bounds need loaded assets.
    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.format_version != FORMAT_VERSION {
            return Err(invalid(format!(
                "unsupported format_version {}",
                self.format_version
            )));
        }
        if !(30..=150).contains(&self.tempo_bpm) {
            return Err(invalid("tempo_bpm must be 30..=150"));
        }
        if !(1..=31).contains(&self.ticks_per_row) {
            return Err(invalid("ticks_per_row must be 1..=31"));
        }
        if self.orders.is_empty() || self.orders.len() > MAX_ITEMS {
            return Err(invalid("orders must contain 1..=256 entries"));
        }
        if self.patterns.len() > MAX_ITEMS || self.instruments.len() > MAX_ITEMS {
            return Err(invalid(
                "patterns and instruments must each contain at most 256 entries",
            ));
        }
        for (index, pattern_id) in self.orders.iter().enumerate() {
            if usize::from(*pattern_id) >= self.patterns.len() {
                return Err(invalid(format!(
                    "order {index} references missing pattern {pattern_id}"
                )));
            }
        }
        for (pattern_index, pattern) in self.patterns.iter().enumerate() {
            if !(1..=MAX_PATTERN_ROWS).contains(&pattern.rows.len()) {
                return Err(invalid(format!(
                    "pattern {pattern_index} must have 1..=256 rows"
                )));
            }
            for (row_index, row) in pattern.rows.iter().enumerate() {
                if row.len() != CHANNEL_COUNT {
                    return Err(invalid(format!(
                        "pattern {pattern_index} row {row_index} must have 16 cells"
                    )));
                }
                for (channel, cell) in row.iter().enumerate() {
                    let location =
                        || format!("pattern {pattern_index} row {row_index} channel {channel}");
                    if matches!(cell.note, Some(Note::On(note)) if note > 127) {
                        return Err(invalid(format!("{} note must be 0..=127", location())));
                    }
                    if cell
                        .instrument
                        .is_some_and(|id| usize::from(id) >= self.instruments.len())
                    {
                        return Err(invalid(format!(
                            "{} references missing instrument",
                            location()
                        )));
                    }
                    if matches!(cell.effect, Some(Effect::SetTempo(tempo)) if !(30..=150).contains(&tempo))
                    {
                        return Err(invalid(format!("{} tempo must be 30..=150", location())));
                    }
                }
            }
        }
        for (index, instrument) in self.instruments.iter().enumerate() {
            match instrument {
                Instrument::Wavetable { samples, .. } if samples.len() != 256 => {
                    return Err(invalid(format!(
                        "instrument {index} wavetable must have 256 samples"
                    )));
                }
                Instrument::Pcm { base_note, .. } if *base_note > 127 => {
                    return Err(invalid(format!(
                        "instrument {index} base_note must be 0..=127"
                    )));
                }
                _ => {}
            }
            instrument
                .macro_data()
                .validate()
                .map_err(|error| invalid(format!("instrument {index}: {error}")))?;
        }
        Ok(())
    }
}

fn absolute_normalized(path: &Path) -> Result<PathBuf, ProjectError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

fn relative_path(target: &Path, base: &Path) -> Option<PathBuf> {
    let target: Vec<_> = target.components().collect();
    let base: Vec<_> = base.components().collect();
    if target.first() != base.first() {
        return None;
    }
    let common = target.iter().zip(&base).take_while(|(a, b)| a == b).count();
    // A distinct Windows prefix/root cannot be traversed with '..'.
    if target[common..]
        .iter()
        .chain(&base[common..])
        .any(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
    {
        return None;
    }
    let mut result = PathBuf::new();
    for _ in common..base.len() {
        result.push("..");
    }
    for component in &target[common..] {
        result.push(component.as_os_str());
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("src32_music_{}_{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn pcm(path: &str) -> Instrument {
        Instrument::Pcm {
            path: path.into(),
            base_note: 60,
            loop_start: Some(999),
            volume: 240,
            pan: 0xf1,
            macro_: Macro::default(),
        }
    }

    #[test]
    fn default_song_has_exact_sine_and_empty_grid() {
        let song = Song::default();
        song.validate().unwrap();
        assert_eq!(
            (song.tempo_bpm, song.ticks_per_row, song.repeat),
            (120, 6, true)
        );
        assert_eq!(song.orders, vec![0]);
        assert_eq!(song.patterns[0].rows.len(), ROW_COUNT);
        assert!(
            song.patterns[0]
                .rows
                .iter()
                .flatten()
                .all(|cell| *cell == Cell::default())
        );
        let Instrument::Wavetable {
            samples,
            volume,
            pan,
            ..
        } = &song.instruments[0]
        else {
            panic!()
        };
        assert_eq!((*volume, *pan), (255, 255));
        assert_eq!(
            (samples[0], samples[64], samples[128], samples[192]),
            (128, 255, 128, 1)
        );
    }

    #[test]
    fn pattern_lengths_accept_one_through_256_rows() {
        let mut song = Song::default();
        song.patterns[0].rows.truncate(1);
        song.validate().unwrap();
        song.patterns[0]
            .rows
            .resize_with(MAX_PATTERN_ROWS, || vec![Cell::default(); CHANNEL_COUNT]);
        song.validate().unwrap();
        song.patterns[0]
            .rows
            .push(vec![Cell::default(); CHANNEL_COUNT]);
        assert!(song.validate().is_err());
    }

    #[test]
    fn toml_roundtrip_preserves_notes_effects_and_all_instrument_kinds() {
        let root = temp_dir();
        let path = root.join("song.toml");
        let mut song = Song::default();
        song.patterns[0].rows.truncate(3);
        let mut second_pattern = Pattern::default();
        second_pattern.rows.truncate(1);
        song.patterns.push(second_pattern);
        song.orders.push(1);
        song.instruments.push(pcm("missing.wav"));
        song.instruments.push(Instrument::Noise {
            volume: 15,
            pan: 0x1f,
            macro_: Macro {
                steps: vec![
                    MacroStep {
                        volume: Some(0),
                        pitch_semitones: Some(-48),
                        pan: None,
                    },
                    MacroStep {
                        volume: None,
                        pitch_semitones: Some(48),
                        pan: Some(255),
                    },
                ],
                loop_start: Some(1),
            },
        });
        song.patterns[0].rows[0][15] = Cell {
            note: Some(Note::On(127)),
            instrument: Some(2),
            volume: Some(128),
            effect: Some(Effect::SetTempo(30)),
        };
        song.patterns[0].rows[1][0] = Cell {
            note: Some(Note::Off),
            instrument: None,
            volume: None,
            effect: Some(Effect::SetVolume(255)),
        };
        song.patterns[0].rows[2][0].effect = Some(Effect::SetPan(0));
        song.save(&path).unwrap();
        assert_eq!(Song::load(&path).unwrap(), song);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn schema_rejects_bad_dimensions_references_and_ranges() {
        let mut mutations: Vec<Box<dyn Fn(&mut Song)>> = vec![
            Box::new(|s| s.format_version = 2),
            Box::new(|s| s.tempo_bpm = 29),
            Box::new(|s| s.tempo_bpm = 151),
            Box::new(|s| s.ticks_per_row = 0),
            Box::new(|s| s.ticks_per_row = 32),
            Box::new(|s| s.orders.clear()),
            Box::new(|s| s.orders = vec![0; 257]),
            Box::new(|s| s.orders[0] = 1),
            Box::new(|s| s.patterns = vec![Pattern::default(); 257]),
            Box::new(|s| s.instruments = vec![Instrument::default(); 257]),
            Box::new(|s| s.patterns[0].rows.clear()),
            Box::new(|s| {
                s.patterns[0].rows.resize_with(MAX_PATTERN_ROWS + 1, || {
                    vec![Cell::default(); CHANNEL_COUNT]
                });
            }),
            Box::new(|s| {
                s.patterns[0].rows[0].pop();
            }),
            Box::new(|s| s.patterns[0].rows[0][0].note = Some(Note::On(128))),
            Box::new(|s| s.patterns[0].rows[0][0].instrument = Some(1)),
            Box::new(|s| s.patterns[0].rows[0][0].effect = Some(Effect::SetTempo(151))),
            Box::new(|s| {
                if let Instrument::Wavetable { samples, .. } = &mut s.instruments[0] {
                    samples.pop();
                }
            }),
            Box::new(|s| {
                s.instruments[0] = pcm("x");
                if let Instrument::Pcm { base_note, .. } = &mut s.instruments[0] {
                    *base_note = 128;
                }
            }),
        ];
        for mutation in mutations.drain(..) {
            let mut song = Song::default();
            mutation(&mut song);
            assert!(matches!(song.validate(), Err(ProjectError::Validation(_))));
        }
    }

    #[test]
    fn macro_limits_allow_hold_steps_but_reject_invalid_loops_and_pitch() {
        let mut data = Macro::default();
        data.validate().unwrap();
        data.loop_start = Some(0);
        assert!(data.validate().is_err());
        data.steps = vec![MacroStep::default(); 256];
        data.loop_start = Some(255);
        data.validate().unwrap();
        data.loop_start = Some(256);
        assert!(data.validate().is_err());
        data.loop_start = None;
        data.steps[0].pitch_semitones = Some(-49);
        assert!(data.validate().is_err());
        data.steps[0].pitch_semitones = Some(49);
        assert!(data.validate().is_err());
        data.steps[0].pitch_semitones = None;
        data.steps.push(MacroStep::default());
        assert!(data.validate().is_err());
    }

    #[test]
    fn save_as_preserves_missing_asset_target_and_returns_new_model() {
        let root = temp_dir();
        let old = root.join("old");
        let new = root.join("new");
        std::fs::create_dir_all(&new).unwrap();
        let mut song = Song::default();
        song.instruments.push(pcm("assets/../missing.wav"));
        let saved = song.save_as(&new.join("song.toml"), &old).unwrap();
        let Instrument::Pcm {
            path, loop_start, ..
        } = &saved.instruments[1]
        else {
            panic!()
        };
        assert_eq!(*path, PathBuf::from("../old/missing.wav"));
        assert_eq!(*loop_start, Some(999)); // Asset bounds are deliberately not schema validation.
        assert_eq!(Song::load(&new.join("song.toml")).unwrap(), saved);
        let Instrument::Pcm { path, .. } = &song.instruments[1] else {
            panic!()
        };
        assert_eq!(*path, PathBuf::from("assets/../missing.wav"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn load_rejects_invalid_version_and_numeric_overflow() {
        let root = temp_dir();
        let path = root.join("song.toml");
        let text = toml::to_string(&Song::default()).unwrap();
        std::fs::write(
            &path,
            text.replace("format_version = 1", "format_version = 2"),
        )
        .unwrap();
        assert!(matches!(
            Song::load(&path),
            Err(ProjectError::Validation(_))
        ));
        std::fs::write(
            &path,
            text.replace("ticks_per_row = 6", "ticks_per_row = 256"),
        )
        .unwrap();
        assert!(matches!(Song::load(&path), Err(ProjectError::Decode(_))));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn cross_volume_rebase_keeps_absolute_target() {
        let mut song = Song::default();
        song.instruments.push(pcm("assets/kick.wav"));
        let rebased = song
            .rebased(Path::new("C:\\music"), Path::new("D:\\songs"))
            .unwrap();
        let Instrument::Pcm { path, .. } = &rebased.instruments[1] else {
            panic!()
        };
        assert_eq!(*path, PathBuf::from("C:\\music\\assets\\kick.wav"));
    }
}
