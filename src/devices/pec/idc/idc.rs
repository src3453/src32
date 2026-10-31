use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::bus::{Bus, Device};
use crate::cartridge::{CartridgeDisk, CartridgeDiskKind, CartridgeDiskMode};
use crate::cow::{BaseImage, CowOverlay};

use super::audio::IdcAudio;
pub const IDC_BASE: u32 = 0x8004_1000;
pub const IDC_SIZE: u32 = 0x1000;
pub const IDC_DATA_ADDRESS: u32 = IDC_BASE + 0x24;
const SECTOR_SIZE: usize = 512;
const MAX_SECTOR_COUNT: u32 = 8_388_607;
const CYCLES_PER_SECOND: u128 = 48_000_000;
const CYCLES_PER_MILLISECOND: u128 = 48_000;

const STATUS_BUSY: u32 = 1;
const STATUS_DONE: u32 = 1 << 1;
const STATUS_ERROR: u32 = 1 << 2;
const STATUS_DRQ: u32 = 1 << 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveType {
    Fdd,
    Hdd,
}

impl DriveType {
    fn from_config(value: &str) -> Option<Self> {
        match value {
            "fdd" => Some(Self::Fdd),
            "hdd" => Some(Self::Hdd),
            _ => None,
        }
    }

    fn register_value(self) -> u32 {
        match self {
            Self::Fdd => 1,
            Self::Hdd => 2,
        }
    }

    fn defaults(self) -> (u64, u64) {
        match self {
            Self::Fdd => (62_500, 100),
            Self::Hdd => (5_000_000, 10),
        }
    }
}

/// Policy for an image whose length is not a multiple of 512 bytes.
/// Applies only to the trailing bytes of the final sector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IncompleteSectorPolicy {
    /// Refuse the image at load time (default; legacy behavior).
    Reject,
    /// Count the trailing bytes as one extra zero-padded sector.
    /// Reads of that sector return the tail bytes followed by zeros;
    /// writes commit only the valid tail prefix and never extend the
    /// image.
    Pad,
    /// Ignore the trailing bytes; capacity covers whole sectors only.
    Truncate,
}

impl IncompleteSectorPolicy {
    fn from_config(value: &str) -> Option<Self> {
        match value {
            "reject" => Some(Self::Reject),
            "pad" => Some(Self::Pad),
            "truncate" => Some(Self::Truncate),
            _ => None,
        }
    }
}

enum DriveImage {
    File(File),
    Bytes(Arc<[u8]>),
    Cow(CowOverlay),
}

pub struct Drive {
    kind: DriveType,
    image: DriveImage,
    capacity_sectors: u32,
    /// Byte length of the trailing partial sector (pad policy only).
    trailing_bytes: usize,
    read_only: bool,
    transfer_rate_bytes_per_second: u64,
    access_latency_ms: u64,
}

impl Drive {
    /// Length of the zero-padded final sector for `lba`, when this
    /// drive carries a partial trailing sector and `lba` is that
    /// last sector.
    fn partial_tail(&self, lba: u32) -> Option<usize> {
        (self.trailing_bytes > 0 && u64::from(lba) + 1 == u64::from(self.capacity_sectors))
            .then_some(self.trailing_bytes)
    }

    fn read_sector(&mut self, lba: u32, buffer: &mut [u8; SECTOR_SIZE]) -> std::io::Result<()> {
        let offset = u64::from(lba) * SECTOR_SIZE as u64;
        let tail = self.partial_tail(lba);
        match &mut self.image {
            DriveImage::File(image) => {
                image.seek(SeekFrom::Start(offset))?;
                match tail {
                    None => image.read_exact(buffer),
                    Some(tail) => {
                        buffer.fill(0);
                        image.read_exact(&mut buffer[..tail])
                    }
                }
            }
            DriveImage::Bytes(image) => {
                let start = offset as usize;
                match tail {
                    None => buffer.copy_from_slice(&image[start..start + SECTOR_SIZE]),
                    Some(tail) => {
                        buffer.fill(0);
                        buffer[..tail].copy_from_slice(&image[start..start + tail]);
                    }
                }
                Ok(())
            }
            DriveImage::Cow(image) => image.read_at(offset, buffer).map_err(cow_io_error),
        }
    }
    fn write_sector(&mut self, lba: u32, buffer: &[u8; SECTOR_SIZE]) -> std::io::Result<()> {
        if self.read_only {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "image is read-only",
            ));
        }
        let offset = u64::from(lba) * SECTOR_SIZE as u64;
        // A padded final sector commits only its valid tail prefix,
        // so writes never extend the image beyond its loaded length.
        let committed = self.partial_tail(lba).unwrap_or(SECTOR_SIZE);
        match &mut self.image {
            DriveImage::File(image) => {
                let mut original = [0; SECTOR_SIZE];
                image.seek(SeekFrom::Start(offset))?;
                image.read_exact(&mut original[..committed])?;
                image.seek(SeekFrom::Start(offset))?;
                if let Err(error) = image
                    .write_all(&buffer[..committed])
                    .and_then(|()| image.flush())
                {
                    let rollback = image
                        .seek(SeekFrom::Start(offset))
                        .and_then(|_| image.write_all(&original[..committed]))
                        .and_then(|()| image.flush());
                    if let Err(rollback_error) = rollback {
                        return Err(std::io::Error::new(
                            error.kind(),
                            format!(
                                "{error}; restoring the uncommitted sector failed: {rollback_error}"
                            ),
                        ));
                    }
                    return Err(error);
                }
                Ok(())
            }
            DriveImage::Bytes(_) => Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "image is read-only",
            )),
            DriveImage::Cow(image) => image
                .write_block(u64::from(lba), buffer)
                .map_err(cow_io_error),
        }
    }
}

fn cow_io_error(error: crate::cow::CowError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, error)
}

#[derive(Clone, Debug)]
pub struct IdcSoundConfig {
    /// Asset-pack search roots in priority order. Each root is a directory
    /// containing `sounds/` (i.e. a checkout of
    /// <https://github.com/86Box/assets>).
    pub asset_roots: Vec<PathBuf>,
    /// Global gain for disk operation sounds (0.0 = muted).
    pub volume: f32,
    /// Explicit per-slot profile names (`internal_name` from the asset-pack
    /// profile configs).
    pub slots: [Option<String>; 4],
    /// Fallback profile for FDD / HDD slots without an explicit name.
    /// Used for cartridge disks, which carry no sound assignment.
    pub default_fdd_profile: Option<String>,
    pub default_hdd_profile: Option<String>,
}

impl IdcSoundConfig {
    pub fn with_roots(asset_roots: Vec<PathBuf>) -> Self {
        Self {
            asset_roots,
            volume: 1.0,
            slots: std::array::from_fn(|_| None),
            default_fdd_profile: None,
            default_hdd_profile: None,
        }
    }

    pub fn muted() -> Self {
        Self::with_roots(Vec::new())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DmaDirection {
    DeviceToMemory,
    MemoryToDevice,
}

struct ActiveCommand {
    direction: DmaDirection,
    slot: usize,
    start_lba: u32,
    sector_count: u32,
    completed_sectors: u32,
    dma_bytes_completed: u64,
    start_cycle: u128,
    next_deadline_cycle: u128,
    buffer: [u8; SECTOR_SIZE],
    cursor: usize,
    read_ready: bool,
    dma_channel: Option<usize>,
    abort_requested: bool,
}

impl ActiveCommand {
    fn deadline(start_cycle: u128, drive: &Drive, completed_sector_count: u32) -> u128 {
        let transfer_bytes = u128::from(completed_sector_count) * SECTOR_SIZE as u128;
        let transfer_cycles = (transfer_bytes * CYCLES_PER_SECOND)
            .div_ceil(u128::from(drive.transfer_rate_bytes_per_second));
        start_cycle
            .saturating_add(u128::from(drive.access_latency_ms) * CYCLES_PER_MILLISECOND)
            .saturating_add(transfer_cycles)
    }
}

pub struct Idc {
    control: u32,
    irq_enable: u32,
    irq_status: u32,
    drive_select: u32,
    lba: u32,
    sector_count: u32,
    error_code: u32,
    done: bool,
    error: bool,
    drive_type: u32,
    sector_size: u32,
    capacity_sectors: u32,
    drive_flags: u32,
    command_word: u32,
    cycle: u128,
    drives: [Option<Drive>; 4],
    active: Option<ActiveCommand>,
    audio: IdcAudio,
}

impl Idc {
    pub fn new(drives: [Option<Drive>; 4]) -> Self {
        Self {
            control: 1,
            irq_enable: 0,
            irq_status: 0,
            drive_select: 0,
            lba: 0,
            sector_count: 0,
            error_code: 0,
            done: false,
            error: false,
            drive_type: 0,
            sector_size: 0,
            capacity_sectors: 0,
            drive_flags: 0,
            command_word: 0,
            cycle: 0,
            drives,
            active: None,
            audio: IdcAudio::new(),
        }
    }

    /// Additively mix this frame's disk operation sounds into stereo buffers.
    /// Silent when no sound profile is assigned or no audio asset is loaded.
    pub fn render_audio(&mut self, sample_rate: u32, left: &mut [i16], right: &mut [i16]) {
        self.audio.render_into(sample_rate, left, right);
    }

    /// Human-readable sound status per slot, for diagnostics and the UI.
    pub fn sound_status(&self) -> [Option<(String, String)>; 4] {
        std::array::from_fn(|slot| {
            let index = self.audio.slot_profile(slot)?;
            self.audio
                .profile_names()
                .get(index)
                .cloned()
                .map(|(internal_name, name)| (internal_name, name))
        })
    }

    pub fn load_cartridge_drives(
        disks: &[CartridgeDisk],
        data_root: &Path,
    ) -> Result<[Option<Drive>; 4], String> {
        let mut drives: [Option<Drive>; 4] = std::array::from_fn(|_| None);
        for disk in disks {
            let slot = usize::from(disk.slot);
            let slot_error = |reason: &str| format!("disk slot {}: {reason}", disk.slot);
            if slot >= drives.len() {
                return Err(slot_error("slot is outside 0..=3"));
            }
            if drives[slot].is_some() {
                return Err(slot_error("configured more than once"));
            }
            let length = disk.image.len();
            if length == 0 || length % SECTOR_SIZE != 0 {
                return Err(slot_error(
                    "image must be non-empty and its length must be a multiple of 512 bytes",
                ));
            }
            let capacity_sectors = u32::try_from(length / SECTOR_SIZE)
                .map_err(|_| slot_error("image exceeds the 32-bit sector capacity"))?;
            let kind = match disk.kind {
                CartridgeDiskKind::Fdd => DriveType::Fdd,
                CartridgeDiskKind::Hdd => DriveType::Hdd,
            };
            let (transfer_rate_bytes_per_second, access_latency_ms) = kind.defaults();
            let (image, read_only) = match disk.mode {
                CartridgeDiskMode::ReadOnly => (DriveImage::Bytes(Arc::clone(&disk.image)), true),
                CartridgeDiskMode::ReadWrite => {
                    let cow_path = data_root.join(format!("dd{}.cow", disk.slot));
                    let overlay = CowOverlay::open(
                        BaseImage::Bytes(Arc::clone(&disk.image)),
                        &cow_path,
                        1,
                        SECTOR_SIZE as u32,
                    )
                    .map_err(|error| {
                        slot_error(&format!(
                            "cannot open COW sidecar {}: {error}",
                            cow_path.display()
                        ))
                    })?;
                    (DriveImage::Cow(overlay), false)
                }
            };
            drives[slot] = Some(Drive {
                kind,
                image,
                capacity_sectors,
                trailing_bytes: 0,
                read_only,
                transfer_rate_bytes_per_second,
                access_latency_ms,
            });
        }
        Ok(drives)
    }

    pub fn load_drives(config_path: &Path) -> Result<[Option<Drive>; 4], String> {
        let contents = match fs::read_to_string(config_path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(std::array::from_fn(|_| None));
            }
            Err(error) => {
                return Err(format!("cannot read {}: {error}", config_path.display()));
            }
        };

        let root: yaml_serde::Value = yaml_serde::from_str(&contents)
            .map_err(|error| format!("invalid YAML in {}: {error}", config_path.display()))?;
        let mut drives: [Option<Drive>; 4] = std::array::from_fn(|_| None);
        let Some(disk_value) = root.get("disk") else {
            return Ok(drives);
        };
        let disks = disk_value
            .as_sequence()
            .ok_or_else(|| format!("{}: `disk` must be a sequence", config_path.display()))?;
        let config_parent = config_path.parent().unwrap_or_else(|| Path::new("."));

        for (index, disk_value) in disks.iter().enumerate() {
            let table = disk_value.as_mapping().ok_or_else(|| {
                format!(
                    "{}: disk entry {} must be a mapping",
                    config_path.display(),
                    index + 1
                )
            })?;
            let slot = table
                .get("slot")
                .and_then(yaml_serde::Value::as_i64)
                .ok_or_else(|| {
                    format!(
                        "{}: disk entry {} has a missing or non-integer slot",
                        config_path.display(),
                        index + 1
                    )
                })?;
            if !(0..=3).contains(&slot) {
                return Err(format!(
                    "{}: disk entry {} slot {} is outside 0..=3",
                    config_path.display(),
                    index + 1,
                    slot
                ));
            }
            let slot = slot as usize;
            let slot_error = |reason: &str| format!("disk slot {slot}: {reason}");
            if drives[slot].is_some() {
                return Err(slot_error("configured more than once"));
            }

            for key in table.keys() {
                let Some(key_name) = key.as_str() else {
                    return Err(slot_error(&format!(
                        "configuration keys must be strings, found {key:?}"
                    )));
                };
                if !matches!(
                    key_name,
                    "slot"
                        | "type"
                        | "image"
                        | "read_only"
                        | "transfer_rate_bytes_per_second"
                        | "access_latency_ms"
                        | "incomplete_sector_policy"
                        | "sound_profile"
                ) {
                    return Err(slot_error(&format!(
                        "unknown configuration key `{key_name}`"
                    )));
                }
            }

            let kind_name = table
                .get("type")
                .and_then(yaml_serde::Value::as_str)
                .ok_or_else(|| slot_error("`type` must be `fdd` or `hdd`"))?;
            let kind = DriveType::from_config(kind_name)
                .ok_or_else(|| slot_error(&format!("unknown drive type `{kind_name}`")))?;
            let image_name = table
                .get("image")
                .and_then(yaml_serde::Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| slot_error("`image` must be a non-empty path"))?;
            let read_only = match table.get("read_only") {
                None => false,
                Some(value) => value
                    .as_bool()
                    .ok_or_else(|| slot_error("`read_only` must be a boolean"))?,
            };
            let (default_rate, default_latency) = kind.defaults();
            let transfer_rate_bytes_per_second = match table.get("transfer_rate_bytes_per_second") {
                None => default_rate,
                Some(value) => {
                    let rate = value.as_i64().ok_or_else(|| {
                        slot_error("`transfer_rate_bytes_per_second` must be a positive integer")
                    })?;
                    if rate <= 0 {
                        return Err(slot_error(
                            "`transfer_rate_bytes_per_second` must be greater than zero",
                        ));
                    }
                    rate as u64
                }
            };
            let access_latency_ms = match table.get("access_latency_ms") {
                None => default_latency,
                Some(value) => {
                    let latency = value.as_i64().ok_or_else(|| {
                        slot_error("`access_latency_ms` must be a non-negative integer")
                    })?;
                    if latency < 0 {
                        return Err(slot_error("`access_latency_ms` must not be negative"));
                    }
                    latency as u64
                }
            };
            let incomplete_sector_policy = match table.get("incomplete_sector_policy") {
                None => IncompleteSectorPolicy::Reject,
                Some(value) => {
                    let name = value
                        .as_str()
                        .ok_or_else(|| slot_error("`incomplete_sector_policy` must be a string"))?;
                    IncompleteSectorPolicy::from_config(name).ok_or_else(|| {
                        slot_error(&format!("unknown `incomplete_sector_policy` `{name}`"))
                    })?
                }
            };

            let image_path = PathBuf::from(image_name);
            let image_path = if image_path.is_absolute() {
                image_path
            } else {
                config_parent.join(image_path)
            };
            let mut options = OpenOptions::new();
            options.read(true).write(!read_only);
            let image = options.open(&image_path).map_err(|error| {
                slot_error(&format!(
                    "cannot open image {}: {error}",
                    image_path.display()
                ))
            })?;
            let metadata = image.metadata().map_err(|error| {
                slot_error(&format!(
                    "cannot inspect image {}: {error}",
                    image_path.display()
                ))
            })?;
            if !metadata.is_file() {
                return Err(slot_error(&format!(
                    "image {} is not a regular file",
                    image_path.display()
                )));
            }
            let length = metadata.len();
            if length == 0 {
                return Err(slot_error(&format!(
                    "image {} must be non-empty and its length must be a multiple of 512 bytes",
                    image_path.display()
                )));
            }
            let full_sectors = length / SECTOR_SIZE as u64;
            let tail = (length % SECTOR_SIZE as u64) as usize;
            let (capacity, trailing_bytes) = match incomplete_sector_policy {
                IncompleteSectorPolicy::Reject => {
                    if tail != 0 {
                        return Err(slot_error(&format!(
                            "image {} must be non-empty and its length must be a multiple of 512 bytes",
                            image_path.display()
                        )));
                    }
                    (full_sectors, 0)
                }
                IncompleteSectorPolicy::Truncate => (full_sectors, 0),
                IncompleteSectorPolicy::Pad => (full_sectors + u64::from(tail != 0), tail),
            };
            if capacity == 0 {
                return Err(slot_error(&format!(
                    "image {} has no complete 512-byte sectors",
                    image_path.display()
                )));
            }
            let capacity_sectors = u32::try_from(capacity).map_err(|_| {
                slot_error(&format!(
                    "image {} exceeds the 32-bit sector capacity",
                    image_path.display()
                ))
            })?;
            drives[slot] = Some(Drive {
                kind,
                image: DriveImage::File(image),
                capacity_sectors,
                trailing_bytes,
                read_only,
                transfer_rate_bytes_per_second,
                access_latency_ms,
            });
        }
        Ok(drives)
    }

    /// Parse the optional `disk_sound` section and per-disk `sound_profile`
    /// names from a config file. Never fails the boot on its own: callers
    /// downgrade errors to warnings so a broken sound setup stays silent
    /// instead of refusing to start the machine.
    pub fn load_sound_config(config_path: &Path) -> Result<IdcSoundConfig, String> {
        let config_parent = config_path.parent().unwrap_or_else(|| Path::new("."));
        let contents = match fs::read_to_string(config_path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(IdcSoundConfig::with_roots(vec![
                    config_parent.to_path_buf(),
                    PathBuf::from("."),
                ]));
            }
            Err(error) => {
                return Err(format!("cannot read {}: {error}", config_path.display()));
            }
        };
        let root: yaml_serde::Value = yaml_serde::from_str(&contents)
            .map_err(|error| format!("invalid YAML in {}: {error}", config_path.display()))?;
        let mut config =
            IdcSoundConfig::with_roots(vec![config_parent.to_path_buf(), PathBuf::from(".")]);
        if let Some(sound) = root.get("disk_sound") {
            let table = sound.as_mapping().ok_or_else(|| {
                format!("{}: `disk_sound` must be a mapping", config_path.display())
            })?;
            for key in table.keys() {
                let Some(key_name) = key.as_str() else {
                    return Err(format!(
                        "{}: `disk_sound` keys must be strings",
                        config_path.display()
                    ));
                };
                if !matches!(
                    key_name,
                    "asset_root" | "volume" | "default_fdd_profile" | "default_hdd_profile"
                ) {
                    return Err(format!(
                        "{}: unknown `disk_sound` key `{key_name}`",
                        config_path.display()
                    ));
                }
            }
            if let Some(root_value) = table.get("asset_root") {
                let root_name = root_value.as_str().ok_or_else(|| {
                    format!(
                        "{}: `disk_sound.asset_root` must be a path string",
                        config_path.display()
                    )
                })?;
                let root_path = PathBuf::from(root_name);
                let root_path = if root_path.is_absolute() {
                    root_path
                } else {
                    config_parent.join(root_path)
                };
                config.asset_roots.insert(0, root_path);
            }
            if let Some(volume_value) = table.get("volume") {
                let volume = volume_value
                    .as_f64()
                    .or_else(|| volume_value.as_i64().map(|v| v as f64))
                    .ok_or_else(|| {
                        format!(
                            "{}: `disk_sound.volume` must be a number",
                            config_path.display()
                        )
                    })?;
                if !volume.is_finite() || volume < 0.0 || volume > 4.0 {
                    return Err(format!(
                        "{}: `disk_sound.volume` must be within 0.0..=4.0",
                        config_path.display()
                    ));
                }
                config.volume = volume as f32;
            }
            for (key, target) in [
                ("default_fdd_profile", &mut config.default_fdd_profile),
                ("default_hdd_profile", &mut config.default_hdd_profile),
            ] {
                if let Some(value) = table.get(key) {
                    let name = value
                        .as_str()
                        .filter(|name| !name.is_empty())
                        .ok_or_else(|| {
                            format!(
                                "{}: `disk_sound.{key}` must be a non-empty profile name",
                                config_path.display()
                            )
                        })?;
                    *target = Some(name.to_string());
                }
            }
        }
        let Some(disk_value) = root.get("disk") else {
            return Ok(config);
        };
        let disks = disk_value
            .as_sequence()
            .ok_or_else(|| format!("{}: `disk` must be a sequence", config_path.display()))?;
        for (index, disk_value) in disks.iter().enumerate() {
            let Some(table) = disk_value.as_mapping() else {
                continue;
            };
            let Some(slot_value) = table.get("slot") else {
                continue;
            };
            let Some(slot) = slot_value.as_i64().filter(|slot| (0..=3).contains(slot)) else {
                continue;
            };
            if let Some(profile_value) = table.get("sound_profile") {
                let name = profile_value
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| {
                        format!(
                            "{}: disk entry {} `sound_profile` must be a non-empty profile name",
                            config_path.display(),
                            index + 1
                        )
                    })?;
                config.slots[slot as usize] = Some(name.to_string());
            }
        }
        Ok(config)
    }

    /// Load user-supplied 86Box-compatible sound assets and assign profiles
    /// to slots. Explicit per-disk `sound_profile` names win; otherwise the
    /// kind defaults apply. Unknown names or missing assets only warn.
    pub fn apply_sound_config(&mut self, config: &IdcSoundConfig) {
        self.audio.set_volume(config.volume);
        let mut found_root = None;
        for root in &config.asset_roots {
            self.audio.load_from_asset_root(root);
            if self.audio.profile_count() > 0 {
                found_root = Some(root.clone());
                break;
            }
        }
        let Some(found_root) = found_root else {
            tracing::info!("IDC audio: no sound asset pack found; disk operation sounds are muted");
            return;
        };
        tracing::info!(
            path = %found_root.display(),
            profiles = self.audio.profile_count(),
            "IDC audio: loaded disk sound profiles",
        );
        for slot in 0..4 {
            let requested = config.slots[slot].clone().or_else(|| {
                let kind = self.drives[slot].as_ref().map(|drive| drive.kind)?;
                match kind {
                    DriveType::Fdd => config.default_fdd_profile.clone(),
                    DriveType::Hdd => config.default_hdd_profile.clone(),
                }
            });
            let Some(name) = requested else {
                continue;
            };
            if self.drives[slot].is_none() {
                continue;
            }
            match self.audio.find_profile(&name) {
                Some(index) => self.audio.set_slot_profile(slot, Some(index)),
                None => {
                    tracing::warn!(
                        slot,
                        profile = %name,
                        "IDC audio: unknown sound profile; slot stays muted",
                    );
                    self.audio.set_slot_profile(slot, None);
                }
            }
        }
    }

    pub(crate) fn tick(&mut self) -> Option<usize> {
        self.cycle = self.cycle.saturating_add(1);
        let Some(active) = self.active.as_ref() else {
            return None;
        };
        let slot = active.slot;
        let direction = active.direction;
        let deadline = active.next_deadline_cycle;

        if direction == DmaDirection::DeviceToMemory {
            let should_load = self
                .active
                .as_ref()
                .is_some_and(|active| !active.read_ready && self.cycle >= deadline);
            if should_load {
                let lba = self
                    .active
                    .as_ref()
                    .map(|active| active.start_lba + active.completed_sectors)
                    .expect("active IDC command");
                let read_result = {
                    let active = self.active.as_mut().expect("active IDC command");
                    self.drives[slot]
                        .as_mut()
                        .expect("active IDC command has a drive")
                        .read_sector(lba, &mut active.buffer)
                };
                if read_result.is_err() {
                    let channel = self.active.as_ref().and_then(|active| active.dma_channel);
                    self.finish_error(6);
                    return channel;
                }
                let active = self.active.as_mut().expect("active IDC command");
                active.cursor = 0;
                active.read_ready = true;
                self.audio.notify_sector_tick(slot, lba, self.cycle);
            }
        } else {
            let should_commit = self
                .active
                .as_ref()
                .is_some_and(|active| active.cursor == SECTOR_SIZE && self.cycle >= deadline);
            if should_commit {
                let lba = self
                    .active
                    .as_ref()
                    .map(|active| active.start_lba + active.completed_sectors)
                    .expect("active IDC command");
                let write_result = {
                    let active = self.active.as_ref().expect("active IDC command");
                    self.drives[slot]
                        .as_mut()
                        .expect("active IDC command has a drive")
                        .write_sector(lba, &active.buffer)
                };
                if write_result.is_err() {
                    let channel = self.active.as_ref().and_then(|active| active.dma_channel);
                    self.finish_error(6);
                    return channel;
                }
                let complete = {
                    let active = self.active.as_mut().expect("active IDC command");
                    active.completed_sectors += 1;
                    active.cursor = 0;
                    active.buffer.fill(0);
                    active.completed_sectors == active.sector_count
                };
                self.audio.notify_sector_tick(slot, lba, self.cycle);
                if complete {
                    self.finish_success();
                } else {
                    self.update_next_deadline();
                }
            }
        }
        None
    }

    pub(crate) fn dma_requirements(&self) -> Option<(DmaDirection, u32)> {
        self.active
            .as_ref()
            .map(|active| (active.direction, active.sector_count * SECTOR_SIZE as u32))
    }

    pub(crate) fn dma_channel(&self) -> Option<usize> {
        self.active.as_ref().and_then(|active| active.dma_channel)
    }

    pub(crate) fn attach_dma(&mut self, channel: usize) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        if active.dma_channel.is_some() {
            return false;
        }
        active.dma_channel = Some(channel);
        true
    }

    pub(crate) fn dma_request_ready(&self, channel: usize) -> bool {
        let Some(active) = self.active.as_ref() else {
            return false;
        };
        if active.dma_channel != Some(channel) {
            return false;
        }
        match active.direction {
            DmaDirection::DeviceToMemory => active.read_ready && active.cursor < SECTOR_SIZE,
            DmaDirection::MemoryToDevice => active.cursor < SECTOR_SIZE,
        }
    }

    pub(crate) fn dma_read_data(&mut self, channel: usize) -> Option<u8> {
        if !self.dma_request_ready(channel) {
            return None;
        }
        let active = self.active.as_mut()?;
        let value = active.buffer[active.cursor];
        active.cursor += 1;
        Some(value)
    }

    pub(crate) fn dma_write_data(&mut self, channel: usize, value: u8) -> bool {
        if !self.dma_request_ready(channel) {
            return false;
        }
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        active.buffer[active.cursor] = value;
        active.cursor += 1;
        true
    }

    /// Called after the DMAC has completed both bus beats for one byte.
    /// Returns true when an IDC-requested abort has reached a sector boundary.
    pub(crate) fn dma_byte_completed(&mut self, channel: usize) -> bool {
        let (abort_at_boundary, sector_completed, transfer_completed) = {
            let Some(active) = self.active.as_mut() else {
                return false;
            };
            if active.dma_channel != Some(channel) {
                return false;
            }
            active.dma_bytes_completed += 1;
            let sector_completed =
                active.direction == DmaDirection::DeviceToMemory && active.cursor == SECTOR_SIZE;
            if sector_completed {
                active.completed_sectors += 1;
                active.cursor = 0;
                active.read_ready = false;
            }
            (
                active.abort_requested && active.dma_bytes_completed % SECTOR_SIZE as u64 == 0,
                sector_completed,
                sector_completed && active.completed_sectors == active.sector_count,
            )
        };
        if abort_at_boundary {
            self.finish_error(5);
            return true;
        }
        if sector_completed {
            if transfer_completed {
                self.finish_success();
            } else {
                self.update_next_deadline();
            }
        }
        false
    }
    pub(crate) fn dma_failed(&mut self, channel: usize) {
        if self.dma_channel() == Some(channel) {
            self.finish_error(5);
        }
    }

    pub(crate) fn abort_dma_at_boundary(&mut self, channel: usize) -> bool {
        let should_abort = self.active.as_ref().is_some_and(|active| {
            active.dma_channel == Some(channel)
                && active.abort_requested
                && active.dma_bytes_completed % SECTOR_SIZE as u64 == 0
        });
        if should_abort {
            self.finish_error(5);
        }
        should_abort
    }

    pub(crate) fn dma_start_failed(&mut self) {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.dma_channel.is_none())
        {
            self.finish_error(5);
        }
    }

    pub(crate) fn irq_asserted(&self) -> bool {
        self.irq_status & self.irq_enable != 0
    }
    pub(crate) fn is_busy(&self) -> bool {
        self.active.is_some()
    }

    pub(crate) fn advance_idle_cycles(&mut self, cycles: u64) {
        assert!(
            self.cycles_without_event() >= u128::from(cycles),
            "cannot batch across an IDC deadline"
        );
        self.cycle = self.cycle.saturating_add(u128::from(cycles));
    }

    /// How many following `tick` calls are guaranteed to change nothing but
    /// the cycle counter: no sector load/commit, no abort and no new DMA
    /// readiness. Unbounded when idle.
    pub(crate) fn cycles_without_event(&self) -> u128 {
        let Some(active) = self.active.as_ref() else {
            return u128::MAX;
        };
        if active.abort_requested {
            return 0;
        }
        let waiting_for_deadline = match active.direction {
            DmaDirection::DeviceToMemory => !active.read_ready,
            DmaDirection::MemoryToDevice => active.cursor == SECTOR_SIZE,
        };
        if !waiting_for_deadline {
            // Progress depends on DMA beats, which the DMAC request mask tracks.
            return u128::MAX;
        }
        // tick() fires once the incremented cycle reaches the deadline.
        active
            .next_deadline_cycle
            .saturating_sub(self.cycle)
            .saturating_sub(1)
    }

    fn read_data_byte(&mut self) -> u8 {
        let Some(active) = self.active.as_mut() else {
            return 0;
        };
        let ready = match active.direction {
            DmaDirection::DeviceToMemory => active.read_ready && active.cursor < SECTOR_SIZE,
            DmaDirection::MemoryToDevice => false,
        };
        if !ready {
            return 0;
        }
        let value = active.buffer[active.cursor];
        active.cursor += 1;
        let sector_completed = active.cursor == SECTOR_SIZE && active.dma_channel.is_none();
        let transfer_completed = if sector_completed {
            active.completed_sectors += 1;
            active.cursor = 0;
            active.read_ready = false;
            active.completed_sectors == active.sector_count
        } else {
            false
        };
        if sector_completed {
            if transfer_completed {
                self.finish_success();
            } else {
                self.update_next_deadline();
            }
        }
        value
    }

    fn write_data_byte(&mut self, value: u8) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if active.direction == DmaDirection::MemoryToDevice && active.cursor < SECTOR_SIZE {
            active.buffer[active.cursor] = value;
            active.cursor += 1;
        }
    }

    pub(crate) fn peek_data_byte(&self) -> u8 {
        self.active
            .as_ref()
            .filter(|active| {
                active.direction == DmaDirection::DeviceToMemory
                    && active.read_ready
                    && active.cursor < SECTOR_SIZE
            })
            .map(|active| active.buffer[active.cursor])
            .unwrap_or(0)
    }

    fn drq(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| match active.direction {
                DmaDirection::DeviceToMemory => active.read_ready && active.cursor < SECTOR_SIZE,
                DmaDirection::MemoryToDevice => active.cursor < SECTOR_SIZE,
            })
    }

    fn execute_command(&mut self, command: u32) {
        if command == 4 {
            if self.active.is_none() {
                return;
            }
            let has_dma = self
                .active
                .as_ref()
                .is_some_and(|active| active.dma_channel.is_some());
            if !has_dma {
                self.finish_error(5);
            } else if let Some(active) = self.active.as_mut() {
                active.abort_requested = true;
            }
            return;
        }
        if self.active.is_some() || self.control & 1 == 0 {
            return;
        }
        self.error_code = 0;
        match command {
            1 | 2 => self.start_transfer(if command == 1 {
                DmaDirection::DeviceToMemory
            } else {
                DmaDirection::MemoryToDevice
            }),
            3 => self.identify(),
            _ => self.finish_error(1),
        }
    }

    fn start_transfer(&mut self, direction: DmaDirection) {
        if self.drive_select > 3 {
            self.finish_error(1);
            return;
        }
        if self.sector_count == 0 || self.sector_count > MAX_SECTOR_COUNT {
            self.finish_error(1);
            return;
        }
        let slot = self.drive_select as usize;
        let Some((capacity_sectors, read_only)) = self.drives[slot]
            .as_ref()
            .map(|drive| (drive.capacity_sectors, drive.read_only))
        else {
            self.finish_error(2);
            return;
        };
        let end_lba = u64::from(self.lba) + u64::from(self.sector_count);
        if end_lba > u64::from(capacity_sectors) {
            self.finish_error(3);
            return;
        }
        if direction == DmaDirection::MemoryToDevice && read_only {
            self.finish_error(4);
            return;
        }
        self.active = Some(ActiveCommand {
            direction,
            slot,
            start_lba: self.lba,
            sector_count: self.sector_count,
            completed_sectors: 0,
            dma_bytes_completed: 0,
            start_cycle: self.cycle,
            next_deadline_cycle: self.cycle,
            buffer: [0; SECTOR_SIZE],
            cursor: 0,
            read_ready: false,
            dma_channel: None,
            abort_requested: false,
        });
        self.update_next_deadline();
        let start_lba = self.lba;
        let sector_count = self.sector_count;
        let cycle = self.cycle;
        self.audio
            .notify_transfer_start(slot, start_lba, sector_count, cycle);
    }

    fn update_next_deadline(&mut self) {
        let Some((slot, start_cycle, completed_sectors)) = self
            .active
            .as_ref()
            .map(|active| (active.slot, active.start_cycle, active.completed_sectors))
        else {
            return;
        };
        let drive = self.drives[slot]
            .as_ref()
            .expect("active IDC command has a drive");
        let deadline =
            ActiveCommand::deadline(start_cycle, drive, completed_sectors.saturating_add(1));
        self.active
            .as_mut()
            .expect("active IDC command")
            .next_deadline_cycle = deadline;
    }

    fn identify(&mut self) {
        self.drive_type = 0;
        self.sector_size = 0;
        self.capacity_sectors = 0;
        self.drive_flags = 0;
        if self.drive_select > 3 {
            self.finish_error(1);
            return;
        }
        let Some(drive) = self.drives[self.drive_select as usize].as_ref() else {
            self.finish_error(2);
            return;
        };
        self.drive_type = drive.kind.register_value();
        self.sector_size = SECTOR_SIZE as u32;
        self.capacity_sectors = drive.capacity_sectors;
        self.drive_flags = 1 | (u32::from(drive.read_only) << 1);
        self.finish_success();
    }

    fn finish_success(&mut self) {
        self.active = None;
        self.done = true;
        if self.irq_enable & 1 != 0 {
            self.irq_status |= 1;
        }
    }

    fn finish_error(&mut self, code: u32) {
        self.active = None;
        self.error_code = code;
        self.error = true;
        if self.irq_enable & 2 != 0 {
            self.irq_status |= 2;
        }
    }

    fn status(&self) -> u32 {
        u32::from(self.active.is_some()) * STATUS_BUSY
            | u32::from(self.done) * STATUS_DONE
            | u32::from(self.error) * STATUS_ERROR
            | u32::from(self.drq()) * STATUS_DRQ
    }

    fn read_register(&self, register: u32) -> u32 {
        match register {
            0x00 => 0x4944_4301,
            0x04 => self.control,
            0x08 => self.status(),
            0x0C => self.irq_enable,
            0x10 => self.irq_status,
            0x14 => self.drive_select,
            0x18 => 0,
            0x1C => self.lba,
            0x20 => self.sector_count,
            0x24 => 0,
            0x28 => self.drive_type,
            0x2C => self.sector_size,
            0x30 => self.capacity_sectors,
            0x34 => self.drive_flags,
            0x38 => self.error_code,
            _ => 0,
        }
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        let register = address & !3;
        let lane = address & 3;
        let shift = (3 - lane) * 8;
        let byte_value = u32::from(value) << shift;
        match register {
            0x04 => {
                self.control = (self.control & !(0xFF << shift) | byte_value) & 1;
            }
            0x08 => {
                if byte_value & STATUS_DONE != 0 {
                    self.done = false;
                }
                if byte_value & STATUS_ERROR != 0 {
                    self.error = false;
                }
            }
            0x0C => {
                self.irq_enable = (self.irq_enable & !(0xFF << shift) | byte_value) & 3;
            }
            0x10 => {
                self.irq_status &= !byte_value;
            }
            0x14 if self.active.is_none() => {
                self.drive_select = self.drive_select & !(0xFF << shift) | byte_value;
            }
            0x18 => {
                self.command_word = self.command_word & !(0xFF << shift) | byte_value;
                if lane == 3 {
                    let command = std::mem::take(&mut self.command_word);
                    self.execute_command(command);
                }
            }
            0x1C if self.active.is_none() => {
                self.lba = self.lba & !(0xFF << shift) | byte_value;
            }
            0x20 if self.active.is_none() => {
                self.sector_count = self.sector_count & !(0xFF << shift) | byte_value;
            }
            0x24 if lane == 0 => self.write_data_byte(value),
            _ => {}
        }
    }
}

struct IdcRegisters(std::rc::Rc<std::cell::RefCell<Idc>>);

impl Device for IdcRegisters {
    fn peek_u32_be(&self, address: u32) -> Option<u32> {
        // DATA lane 0 consumes a byte; every other register read is pure.
        (address & 3 == 0 && address != 0x24).then(|| self.0.borrow().read_register(address))
    }

    fn read(&mut self, address: u32) -> u8 {
        let register = address & !3;
        let lane = address & 3;
        if register == 0x24 && lane == 0 {
            return self.0.borrow_mut().read_data_byte();
        }
        let value = self.0.borrow().read_register(register);
        let shift = (3 - lane) * 8;
        (value >> shift) as u8
    }

    fn write(&mut self, address: u32, value: u8) {
        self.0.borrow_mut().write_byte(address, value);
    }

    fn size(&self) -> u32 {
        IDC_SIZE
    }
}

pub fn connect_idc(
    bus: &mut Bus,
    drives: [Option<Drive>; 4],
) -> std::rc::Rc<std::cell::RefCell<Idc>> {
    let idc = std::rc::Rc::new(std::cell::RefCell::new(Idc::new(drives)));
    bus.add_device(
        IDC_BASE,
        "PeC IDC MMIO",
        Box::new(IdcRegisters(std::rc::Rc::clone(&idc))),
    );
    bus.attach_idc(std::rc::Rc::clone(&idc));
    idc
}

pub fn connect_idc_from_config(
    bus: &mut Bus,
) -> Result<std::rc::Rc<std::cell::RefCell<Idc>>, String> {
    let config_path = Path::new("config.yaml");
    let drives = Idc::load_drives(config_path)?;
    let idc = connect_idc(bus, drives);
    // Sound must never break the boot: a broken sound setup only warns.
    match Idc::load_sound_config(config_path) {
        Ok(sound) => idc.borrow_mut().apply_sound_config(&sound),
        Err(error) => tracing::warn!(%error, "IDC audio: ignoring sound configuration"),
    }
    Ok(idc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::bmc::connect_bmc;
    use crate::devices::dmac::dmac::{DMAC_BASE, connect_dmac};
    use crate::devices::irqc::irqc::{IRQC_BASE, connect_irqc};
    use crate::devices::ram::connect_ram;
    use std::sync::atomic::{AtomicU64, Ordering};

    const DMA_CH0: u32 = DMAC_BASE + 0x100;
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_root() -> PathBuf {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("cpt32-idc-{}-{id}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// `Drive` intentionally has no `Debug` impl (it owns open files), so
    /// error-path assertions go through `Result::err` instead of
    /// `unwrap_err`, which would require `T: Debug`.
    fn load_drives_err(config: &Path) -> String {
        Idc::load_drives(config)
            .err()
            .expect("expected a drive configuration error")
    }

    fn fixture_config(root: &Path, image: &[u8], read_only: bool) -> PathBuf {
        fixture_config_with_latency(root, image, read_only, 0)
    }

    fn fixture_config_with_latency(
        root: &Path,
        image: &[u8],
        read_only: bool,
        latency_ms: u64,
    ) -> PathBuf {
        fs::write(root.join("disk.img"), image).unwrap();
        let config = root.join("config.yaml");
        fs::write(
            &config,
            format!(
                "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n    read_only: {read_only}\n    transfer_rate_bytes_per_second: {}\n    access_latency_ms: {latency_ms}\n",
                i64::MAX
            ),
        )
        .unwrap();
        config
    }
    fn test_bus(config: &Path) -> Bus {
        let drives = Idc::load_drives(config).unwrap();
        let mut bus = Bus::new();
        connect_ram(&mut bus);
        let bmc = connect_bmc(&mut bus);
        connect_dmac(&mut bus, bmc);
        connect_idc(&mut bus, drives);
        let irq = connect_irqc(&mut bus);
        bus.attach_irq_controller(irq);
        bus
    }

    fn configure_dma(bus: &mut Bus, read: bool, memory: u32, count: u32) {
        bus.write_u32_be(DMAC_BASE + 0x008, 1);
        bus.write_u32_be(DMA_CH0, if read { IDC_DATA_ADDRESS } else { memory });
        bus.write_u32_be(DMA_CH0 + 4, if read { memory } else { IDC_DATA_ADDRESS });
        bus.write_u32_be(DMA_CH0 + 8, count);
        let source_fixed = u32::from(read) << 2;
        let destination_fixed = u32::from(!read) << 4;
        bus.write_u32_be(DMA_CH0 + 0x0C, source_fixed | destination_fixed | (1 << 8));
    }

    fn run_ticks(bus: &mut Bus, ticks: usize) {
        for _ in 0..ticks {
            bus.tick_devices();
        }
    }

    #[test]
    fn pio_read_completes_each_sector_and_signals_done() {
        let root = temp_root();
        let mut image = vec![0x11; SECTOR_SIZE];
        image.extend(vec![0xA5; SECTOR_SIZE]);
        let config = fixture_config(&root, &image, false);
        let mut bus = test_bus(&config);
        bus.write_u32_be(IDC_BASE + 0x0c, 1);
        bus.write_u32_be(IDC_BASE + 0x20, 2);
        bus.write_u32_be(IDC_BASE + 0x18, 1);
        for byte in [0x11, 0xA5] {
            run_ticks(&mut bus, 2048);
            assert_ne!(bus.read_u32_be(IDC_BASE + 8) & STATUS_DRQ, 0);
            for _ in 0..SECTOR_SIZE {
                assert_eq!(bus.read_u8(IDC_DATA_ADDRESS), byte);
            }
        }
        let status = bus.read_u32_be(IDC_BASE + 8);
        assert_eq!(status & (STATUS_BUSY | STATUS_DRQ | STATUS_ERROR), 0);
        assert_ne!(status & STATUS_DONE, 0);
        assert_ne!(bus.read_u32_be(IDC_BASE + 0x10) & 1, 0);
        drop(bus);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identify_read_and_range_error_follow_raw_image_contract() {
        let root = temp_root();
        let mut image = vec![0x11; SECTOR_SIZE];
        image.extend(vec![0xA5; SECTOR_SIZE]);
        let config = fixture_config(&root, &image, false);
        let mut bus = test_bus(&config);

        bus.write_u32_be(IDC_BASE + 0x00C, 1);
        bus.write_u32_be(IRQC_BASE + 4, 1 << 2);
        bus.write_u32_be(IDC_BASE + 0x014, 0);
        bus.write_u32_be(IDC_BASE + 0x018, 3);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x028), 1);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x02C), 512);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x030), 2);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x034), 1);

        bus.write_u32_be(IDC_BASE + 0x010, 1);
        bus.write_u8(IRQC_BASE + 1, 1 << 2);
        bus.write_u32_be(IDC_BASE + 0x01C, 1);
        bus.write_u32_be(IDC_BASE + 0x020, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 1);
        configure_dma(&mut bus, true, 0x2000, SECTOR_SIZE as u32);
        run_ticks(&mut bus, 2048);

        for index in 0..SECTOR_SIZE {
            assert_eq!(bus.read_u8(0x2000 + index as u32), 0xA5);
        }
        assert_ne!(bus.read_u32_be(IDC_BASE + 0x008) & STATUS_DONE, 0);
        assert_ne!(bus.read_u8(IRQC_BASE + 1) & (1 << 2), 0);

        for index in 0..SECTOR_SIZE {
            bus.write_u8(0x4000 + index as u32, 0x6D);
        }
        bus.write_u32_be(IDC_BASE + 0x01C, 1);
        bus.write_u32_be(IDC_BASE + 0x020, 2);
        bus.write_u32_be(IDC_BASE + 0x018, 1);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x038), 3);
        for index in 0..SECTOR_SIZE {
            assert_eq!(bus.read_u8(0x4000 + index as u32), 0x6D);
        }

        drop(bus);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn write_dma_commits_complete_sectors_and_read_only_rejects_write() {
        let root = temp_root();
        let image = vec![0x11; SECTOR_SIZE * 2];
        let config = fixture_config(&root, &image, false);
        let mut bus = test_bus(&config);
        for index in 0..SECTOR_SIZE * 2 {
            bus.write_u8(0x3000 + index as u32, (index % 251) as u8);
        }
        bus.write_u32_be(IDC_BASE + 0x014, 0);
        bus.write_u32_be(IDC_BASE + 0x01C, 0);
        bus.write_u32_be(IDC_BASE + 0x020, 2);
        bus.write_u32_be(IDC_BASE + 0x018, 2);
        configure_dma(&mut bus, false, 0x3000, (SECTOR_SIZE * 2) as u32);
        run_ticks(&mut bus, 4096);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x038), 0);
        assert_ne!(bus.read_u32_be(IDC_BASE + 0x008) & STATUS_DONE, 0);
        drop(bus);
        let written = fs::read(root.join("disk.img")).unwrap();
        assert_eq!(
            written,
            (0..SECTOR_SIZE * 2)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>()
        );
        fs::remove_dir_all(root).unwrap();

        let root = temp_root();
        let original = vec![0xC3; SECTOR_SIZE];
        let config = fixture_config(&root, &original, true);
        let mut bus = test_bus(&config);
        bus.write_u32_be(IDC_BASE + 0x014, 0);
        bus.write_u32_be(IDC_BASE + 0x01C, 0);
        bus.write_u32_be(IDC_BASE + 0x020, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 2);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x038), 4);
        drop(bus);
        assert_eq!(fs::read(root.join("disk.img")).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dma_configuration_mismatch_fails_both_controllers() {
        let root = temp_root();
        let config = fixture_config(&root, &[0x5A; SECTOR_SIZE], false);
        let mut bus = test_bus(&config);
        bus.write_u32_be(IDC_BASE + 0x014, 0);
        bus.write_u32_be(IDC_BASE + 0x01C, 0);
        bus.write_u32_be(IDC_BASE + 0x020, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 1);
        configure_dma(&mut bus, true, 0x2000, SECTOR_SIZE as u32 - 1);
        assert_eq!(bus.read_u32_be(DMA_CH0 + 0x10) >> 4 & 0xF, 1);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x038), 5);
        assert_ne!(bus.read_u32_be(IDC_BASE + 0x008) & STATUS_ERROR, 0);
        drop(bus);
        fs::remove_dir_all(root).unwrap();

        let root = temp_root();
        let config = fixture_config(&root, &[0x5A; SECTOR_SIZE], false);
        let mut bus = test_bus(&config);
        bus.write_u32_be(IDC_BASE + 0x014, 0);
        bus.write_u32_be(IDC_BASE + 0x01C, 0);
        bus.write_u32_be(IDC_BASE + 0x020, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 1);
        bus.write_u32_be(DMAC_BASE + 0x008, 1);
        bus.write_u32_be(DMA_CH0, IDC_DATA_ADDRESS + 4);
        bus.write_u32_be(DMA_CH0 + 4, 0x2000);
        bus.write_u32_be(DMA_CH0 + 8, SECTOR_SIZE as u32);
        bus.write_u32_be(DMA_CH0 + 0x0C, (1 << 2) | (1 << 8));
        assert_eq!(bus.read_u32_be(DMA_CH0 + 0x10) >> 4 & 0xF, 1);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x038), 5);
        drop(bus);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_one_dmac_channel_can_configure_the_idc_endpoint() {
        let root = temp_root();
        let config = fixture_config(&root, &[0x5A; SECTOR_SIZE], false);
        let mut bus = test_bus(&config);
        bus.write_u32_be(IDC_BASE + 0x014, 0);
        bus.write_u32_be(IDC_BASE + 0x01C, 0);
        bus.write_u32_be(IDC_BASE + 0x020, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 1);
        configure_dma(&mut bus, true, 0x2000, SECTOR_SIZE as u32);

        let channel1 = DMA_CH0 + 0x20;
        bus.write_u32_be(DMAC_BASE + 0x008, 3);
        bus.write_u32_be(channel1, IDC_DATA_ADDRESS);
        bus.write_u32_be(channel1 + 4, 0x3000);
        bus.write_u32_be(channel1 + 8, SECTOR_SIZE as u32);
        bus.write_u32_be(channel1 + 0x0C, (1 << 2) | (1 << 8));
        assert_eq!(bus.read_u32_be(channel1 + 0x10) >> 4 & 0xF, 1);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x038), 0);

        run_ticks(&mut bus, 2048);
        assert_ne!(bus.read_u32_be(IDC_BASE + 0x008) & STATUS_DONE, 0);
        drop(bus);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_rejects_duplicate_slots_invalid_lengths_missing_images_and_zero_rate() {
        let root = temp_root();
        let image_path = root.join("disk.img");
        fs::write(&image_path, [0u8; SECTOR_SIZE]).unwrap();

        let duplicate = root.join("duplicate.yaml");
        fs::write(
            &duplicate,
            "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n  - slot: 0\n    type: hdd\n    image: disk.img\n",
        )
        .unwrap();
        assert!(load_drives_err(&duplicate).contains("disk slot 0"));

        fs::write(&image_path, [0u8; SECTOR_SIZE + 1]).unwrap();
        let invalid_length = root.join("invalid-length.yaml");
        fs::write(
            &invalid_length,
            "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n",
        )
        .unwrap();
        assert!(load_drives_err(&invalid_length).contains("multiple of 512"));

        fs::write(&image_path, [0u8; SECTOR_SIZE]).unwrap();
        let missing = root.join("missing.yaml");
        fs::write(
            &missing,
            "disk:\n  - slot: 0\n    type: fdd\n    image: absent.img\n",
        )
        .unwrap();
        assert!(load_drives_err(&missing).contains("disk slot 0"));

        let zero_rate = root.join("zero-rate.yaml");
        fs::write(
            &zero_rate,
            "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n    transfer_rate_bytes_per_second: 0\n",
        )
        .unwrap();
        assert!(load_drives_err(&zero_rate).contains("greater than zero"));
        let negative_latency = root.join("negative-latency.yaml");
        fs::write(
            &negative_latency,
            "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n    access_latency_ms: -1\n",
        )
        .unwrap();
        assert!(load_drives_err(&negative_latency).contains("must not be negative"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_sector_policy_pad_reads_zero_padded_tail_and_writes_prefix() {
        let root = temp_root();
        let mut image = vec![0x5A; SECTOR_SIZE];
        image.extend(vec![0xB7; 100]);
        fs::write(root.join("disk.img"), &image).unwrap();
        let config = root.join("config.yaml");
        fs::write(
            &config,
            format!(
                "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n    transfer_rate_bytes_per_second: {}\n    access_latency_ms: 0\n    incomplete_sector_policy: pad\n",
                i64::MAX
            ),
        )
        .unwrap();
        let mut bus = test_bus(&config);
        bus.write_u32_be(IDC_BASE + 0x014, 0);
        bus.write_u32_be(IDC_BASE + 0x018, 3);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x030), 2);

        bus.write_u32_be(IDC_BASE + 0x01C, 1);
        bus.write_u32_be(IDC_BASE + 0x020, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 1);
        configure_dma(&mut bus, true, 0x2000, SECTOR_SIZE as u32);
        run_ticks(&mut bus, 2048);
        for index in 0..100 {
            assert_eq!(bus.read_u8(0x2000 + index as u32), 0xB7);
        }
        for index in 100..SECTOR_SIZE {
            assert_eq!(bus.read_u8(0x2000 + index as u32), 0);
        }
        assert_ne!(bus.read_u32_be(IDC_BASE + 0x008) & STATUS_DONE, 0);

        for index in 0..SECTOR_SIZE {
            bus.write_u8(0x3000 + index as u32, (index % 199) as u8);
        }
        bus.write_u32_be(IDC_BASE + 0x01C, 1);
        bus.write_u32_be(IDC_BASE + 0x020, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 2);
        configure_dma(&mut bus, false, 0x3000, SECTOR_SIZE as u32);
        run_ticks(&mut bus, 2048);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x038), 0);
        assert_ne!(bus.read_u32_be(IDC_BASE + 0x008) & STATUS_DONE, 0);
        drop(bus);
        let written = fs::read(root.join("disk.img")).unwrap();
        assert_eq!(written.len(), image.len());
        assert_eq!(&written[..SECTOR_SIZE], &image[..SECTOR_SIZE]);
        assert_eq!(
            &written[SECTOR_SIZE..],
            &(0..100)
                .map(|index| (index % 199) as u8)
                .collect::<Vec<_>>()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_sector_policy_truncate_ignores_trailing_bytes() {
        let root = temp_root();
        let mut image = vec![0x5A; SECTOR_SIZE];
        image.extend(vec![0xB7; 100]);
        fs::write(root.join("disk.img"), &image).unwrap();
        let config = root.join("config.yaml");
        fs::write(
            &config,
            format!(
                "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n    transfer_rate_bytes_per_second: {}\n    access_latency_ms: 0\n    incomplete_sector_policy: truncate\n",
                i64::MAX
            ),
        )
        .unwrap();
        let mut bus = test_bus(&config);
        bus.write_u32_be(IDC_BASE + 0x014, 0);
        bus.write_u32_be(IDC_BASE + 0x018, 3);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x030), 1);

        bus.write_u32_be(IDC_BASE + 0x01C, 0);
        bus.write_u32_be(IDC_BASE + 0x020, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 1);
        configure_dma(&mut bus, true, 0x2000, SECTOR_SIZE as u32);
        run_ticks(&mut bus, 2048);
        for index in 0..SECTOR_SIZE {
            assert_eq!(bus.read_u8(0x2000 + index as u32), 0x5A);
        }

        bus.write_u32_be(IDC_BASE + 0x01C, 1);
        bus.write_u32_be(IDC_BASE + 0x020, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 1);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x038), 3);
        drop(bus);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_sector_policy_validates_names_and_default_rejects() {
        let root = temp_root();
        let image_path = root.join("disk.img");
        fs::write(&image_path, [0u8; SECTOR_SIZE]).unwrap();

        let unknown = root.join("unknown-policy.yaml");
        fs::write(
            &unknown,
            "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n    incomplete_sector_policy: zero-fill\n",
        )
        .unwrap();
        assert!(load_drives_err(&unknown).contains("unknown `incomplete_sector_policy`"));

        let not_string = root.join("not-string.yaml");
        fs::write(
            &not_string,
            "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n    incomplete_sector_policy: 1\n",
        )
        .unwrap();
        assert!(
            load_drives_err(&not_string).contains("`incomplete_sector_policy` must be a string")
        );

        fs::write(&image_path, [0u8; SECTOR_SIZE + 3]).unwrap();
        let default_rejects = root.join("default.yaml");
        fs::write(
            &default_rejects,
            "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n",
        )
        .unwrap();
        assert!(load_drives_err(&default_rejects).contains("multiple of 512"));

        let pad_accepts = root.join("pad.yaml");
        fs::write(
            &pad_accepts,
            "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n    incomplete_sector_policy: pad\n",
        )
        .unwrap();
        let drives = Idc::load_drives(&pad_accepts).unwrap();
        assert_eq!(drives[0].as_ref().unwrap().capacity_sectors, 2);
        assert_eq!(drives[0].as_ref().unwrap().trailing_bytes, 3);

        fs::write(&image_path, [0u8; 100]).unwrap();
        let no_sectors = root.join("no-sectors.yaml");
        fs::write(
            &no_sectors,
            "disk:\n  - slot: 0\n    type: fdd\n    image: disk.img\n    incomplete_sector_policy: truncate\n",
        )
        .unwrap();
        assert!(load_drives_err(&no_sectors).contains("no complete 512-byte sectors"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn drive_type_profiles_supply_rate_latency_and_writable_default() {
        let root = temp_root();
        fs::write(root.join("fdd.img"), [0u8; SECTOR_SIZE]).unwrap();
        fs::write(root.join("hdd.img"), [0u8; SECTOR_SIZE]).unwrap();
        let config = root.join("config.yaml");
        fs::write(
            &config,
            "disk:\n  - slot: 0\n    type: fdd\n    image: fdd.img\n  - slot: 1\n    type: hdd\n    image: hdd.img\n",
        )
        .unwrap();
        let mut drives = Idc::load_drives(&config).unwrap();
        let fdd = drives[0].take().unwrap();
        let hdd = drives[1].take().unwrap();
        assert_eq!(fdd.kind, DriveType::Fdd);
        assert_eq!(fdd.transfer_rate_bytes_per_second, 62_500);
        assert_eq!(fdd.access_latency_ms, 100);
        assert!(!fdd.read_only);
        assert_eq!(hdd.kind, DriveType::Hdd);
        assert_eq!(hdd.transfer_rate_bytes_per_second, 5_000_000);
        assert_eq!(hdd.access_latency_ms, 10);
        drop((fdd, hdd));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cartridge_drives_read_package_bytes_and_use_read_only_or_cow_writes() {
        let root = temp_root();
        let image: Arc<[u8]> = Arc::from(vec![0xA6; SECTOR_SIZE]);
        let readonly_disk = CartridgeDisk {
            slot: 0,
            kind: CartridgeDiskKind::Fdd,
            mode: CartridgeDiskMode::ReadOnly,
            image: Arc::clone(&image),
        };
        let mut readonly = Idc::load_cartridge_drives(&[readonly_disk], &root).unwrap();
        let drive = readonly[0].as_mut().unwrap();
        let mut sector = [0; SECTOR_SIZE];
        drive.read_sector(0, &mut sector).unwrap();
        assert_eq!(sector, [0xA6; SECTOR_SIZE]);
        assert!(drive.read_only);
        assert_eq!(
            drive
                .write_sector(0, &[0x3C; SECTOR_SIZE])
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );

        let writable_disk = CartridgeDisk {
            slot: 2,
            kind: CartridgeDiskKind::Hdd,
            mode: CartridgeDiskMode::ReadWrite,
            image: Arc::clone(&image),
        };
        let mut writable = Idc::load_cartridge_drives(&[writable_disk], &root).unwrap();
        let drive = writable[2].as_mut().unwrap();
        assert!(!drive.read_only);
        drive.read_sector(0, &mut sector).unwrap();
        assert_eq!(sector, [0xA6; SECTOR_SIZE]);
        drive.write_sector(0, &[0x3C; SECTOR_SIZE]).unwrap();
        drive.read_sector(0, &mut sector).unwrap();
        assert_eq!(sector, [0x3C; SECTOR_SIZE]);
        assert!(root.join("dd2.cow").is_file());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_config_leaves_all_slots_unconnected() {
        let root = temp_root();
        let drives = Idc::load_drives(&root.join("config.yaml")).unwrap();
        assert!(drives.iter().all(Option::is_none));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn read_dma_uses_exact_deadline_for_each_sector_without_early_next_sector() {
        let mut image = vec![0xA5; SECTOR_SIZE];
        image.extend_from_slice(&[0x5A; SECTOR_SIZE]);
        let drive = Drive {
            kind: DriveType::Hdd,
            image: DriveImage::Bytes(Arc::from(image)),
            capacity_sectors: 2,
            trailing_bytes: 0,
            read_only: true,
            transfer_rate_bytes_per_second: CYCLES_PER_SECOND as u64,
            access_latency_ms: 0,
        };
        let mut idc = Idc::new([Some(drive), None, None, None]);
        idc.drive_select = 0;
        idc.lba = 0;
        idc.sector_count = 2;
        idc.execute_command(1);
        assert!(idc.attach_dma(0));

        for _ in 0..SECTOR_SIZE - 1 {
            idc.tick();
        }
        assert_eq!(idc.status() & STATUS_DRQ, 0);
        idc.tick();
        assert_ne!(idc.status() & STATUS_DRQ, 0);

        for _ in 0..SECTOR_SIZE {
            assert_eq!(idc.dma_read_data(0), Some(0xA5));
            assert!(!idc.dma_byte_completed(0));
        }
        assert_eq!(idc.status() & STATUS_DRQ, 0);
        for _ in 0..SECTOR_SIZE - 1 {
            idc.tick();
        }
        assert_eq!(idc.status() & STATUS_DRQ, 0);
        idc.tick();
        assert_ne!(idc.status() & STATUS_DRQ, 0);

        for _ in 0..SECTOR_SIZE {
            assert_eq!(idc.dma_read_data(0), Some(0x5A));
            assert!(!idc.dma_byte_completed(0));
        }
        assert_ne!(idc.status() & STATUS_DONE, 0);
    }
    #[test]
    fn write_dma_commits_each_sector_only_at_its_deadline() {
        let root = temp_root();
        let image_path = root.join("disk.img");
        fs::write(&image_path, [0; SECTOR_SIZE * 2]).unwrap();
        let drive = Drive {
            kind: DriveType::Hdd,
            image: DriveImage::File(
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&image_path)
                    .unwrap(),
            ),
            capacity_sectors: 2,
            trailing_bytes: 0,
            read_only: false,
            transfer_rate_bytes_per_second: CYCLES_PER_SECOND as u64,
            access_latency_ms: 0,
        };
        let mut idc = Idc::new([Some(drive), None, None, None]);
        idc.drive_select = 0;
        idc.lba = 0;
        idc.sector_count = 2;
        idc.execute_command(2);
        assert!(idc.attach_dma(0));

        for _ in 0..SECTOR_SIZE {
            assert!(idc.dma_write_data(0, 0xA5));
            assert!(!idc.dma_byte_completed(0));
        }
        let mut sector = [0; SECTOR_SIZE];
        idc.drives[0]
            .as_mut()
            .unwrap()
            .read_sector(0, &mut sector)
            .unwrap();
        assert_eq!(sector, [0; SECTOR_SIZE]);
        for _ in 0..SECTOR_SIZE - 1 {
            idc.tick();
        }
        idc.drives[0]
            .as_mut()
            .unwrap()
            .read_sector(0, &mut sector)
            .unwrap();
        assert_eq!(sector, [0; SECTOR_SIZE]);
        idc.tick();
        idc.drives[0]
            .as_mut()
            .unwrap()
            .read_sector(0, &mut sector)
            .unwrap();
        assert_eq!(sector, [0xA5; SECTOR_SIZE]);

        for _ in 0..SECTOR_SIZE {
            assert!(idc.dma_write_data(0, 0x5A));
            assert!(!idc.dma_byte_completed(0));
        }
        for _ in 0..SECTOR_SIZE - 1 {
            idc.tick();
        }
        idc.drives[0]
            .as_mut()
            .unwrap()
            .read_sector(1, &mut sector)
            .unwrap();
        assert_eq!(sector, [0; SECTOR_SIZE]);
        idc.tick();
        idc.drives[0]
            .as_mut()
            .unwrap()
            .read_sector(1, &mut sector)
            .unwrap();
        assert_eq!(sector, [0x5A; SECTOR_SIZE]);
        assert_ne!(idc.status() & STATUS_DONE, 0);
        drop(idc);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn read_dma_waits_for_sector_deadline_and_rejects_wide_fifo_access() {
        let root = temp_root();
        let config = fixture_config_with_latency(&root, &[0xA7; SECTOR_SIZE], false, 1);
        let mut bus = test_bus(&config);
        bus.write_u32_be(IDC_BASE + 0x014, 0);
        bus.write_u32_be(IDC_BASE + 0x01C, 0);
        bus.write_u32_be(IDC_BASE + 0x020, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 1);
        configure_dma(&mut bus, true, 0x2000, SECTOR_SIZE as u32);

        run_ticks(&mut bus, 48_000);
        assert_ne!(bus.read_u32_be(IDC_BASE + 0x008) & STATUS_BUSY, 0);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x008) & STATUS_DRQ, 0);
        assert_eq!(bus.read_u32_be(DMA_CH0 + 0x14), SECTOR_SIZE as u32);
        assert_eq!(bus.read_u8(0x2000), 0);
        assert!(bus.has_pending_dma());

        bus.tick_devices();
        assert_eq!(bus.read_u32_be(DMA_CH0 + 0x14), SECTOR_SIZE as u32);
        assert_eq!(bus.read_u8(0x2000), 0);
        bus.tick_devices();
        assert_eq!(bus.read_u32_be(DMA_CH0 + 0x14), SECTOR_SIZE as u32 - 1);
        assert_eq!(bus.read_u8(0x2000), 0xA7);

        bus.write_u32_be(IDC_DATA_ADDRESS, 0xDEAD_BEEF);
        assert!(bus.take_bus_error());
        drop(bus);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn abort_waits_for_sector_boundary_and_control_disable_does_not_stop_dma() {
        let root = temp_root();
        let config = fixture_config(&root, &[0x5C; SECTOR_SIZE], false);
        let mut bus = test_bus(&config);
        bus.write_u32_be(IDC_BASE + 0x014, 0);
        bus.write_u32_be(IDC_BASE + 0x01C, 0);
        bus.write_u32_be(IDC_BASE + 0x020, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 1);
        configure_dma(&mut bus, true, 0x2000, SECTOR_SIZE as u32);
        bus.write_u32_be(IDC_BASE + 0x004, 0);
        run_ticks(&mut bus, 1);
        bus.write_u32_be(IDC_BASE + 0x018, 4);
        assert_ne!(bus.read_u32_be(IDC_BASE + 0x008) & STATUS_BUSY, 0);
        run_ticks(&mut bus, 2048);
        assert_eq!(bus.read_u32_be(IDC_BASE + 0x038), 5);
        assert_eq!(bus.read_u32_be(DMA_CH0 + 0x10) >> 4 & 0xF, 3);
        for index in 0..SECTOR_SIZE {
            assert_eq!(bus.read_u8(0x2000 + index as u32), 0x5C);
        }
        drop(bus);
        fs::remove_dir_all(root).unwrap();
    }
}
