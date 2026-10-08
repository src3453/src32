use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::bus::{Bus, Device};

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

#[derive(Debug)]
pub struct Drive {
    kind: DriveType,
    image: File,
    capacity_sectors: u32,
    read_only: bool,
    transfer_rate_bytes_per_second: u64,
    access_latency_ms: u64,
}

impl Drive {
    fn read_sector(&mut self, lba: u32, buffer: &mut [u8; SECTOR_SIZE]) -> std::io::Result<()> {
        self.image
            .seek(SeekFrom::Start(u64::from(lba) * SECTOR_SIZE as u64))?;
        self.image.read_exact(buffer)
    }

    fn write_sector(&mut self, lba: u32, buffer: &[u8; SECTOR_SIZE]) -> std::io::Result<()> {
        if self.read_only {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "image is read-only",
            ));
        }
        let offset = u64::from(lba) * SECTOR_SIZE as u64;
        let mut original = [0; SECTOR_SIZE];
        self.image.seek(SeekFrom::Start(offset))?;
        self.image.read_exact(&mut original)?;
        self.image.seek(SeekFrom::Start(offset))?;
        if let Err(error) = self
            .image
            .write_all(buffer)
            .and_then(|()| self.image.flush())
        {
            let rollback = self
                .image
                .seek(SeekFrom::Start(offset))
                .and_then(|_| self.image.write_all(&original))
                .and_then(|()| self.image.flush());
            if let Err(rollback_error) = rollback {
                return Err(std::io::Error::new(
                    error.kind(),
                    format!("{error}; restoring the uncommitted sector failed: {rollback_error}"),
                ));
            }
            return Err(error);
        }
        Ok(())
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
    buffer: [u8; SECTOR_SIZE],
    cursor: usize,
    read_ready: bool,
    dma_channel: Option<usize>,
    abort_requested: bool,
}

impl ActiveCommand {
    fn deadline(&self, drive: &Drive, completed_sector_count: u32) -> u128 {
        let transfer_bytes = u128::from(completed_sector_count) * SECTOR_SIZE as u128;
        let transfer_cycles = (transfer_bytes * CYCLES_PER_SECOND)
            .div_ceil(u128::from(drive.transfer_rate_bytes_per_second));
        self.start_cycle
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
        }
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

        let root: toml::Value = toml::from_str(&contents)
            .map_err(|error| format!("invalid TOML in {}: {error}", config_path.display()))?;
        let mut drives: [Option<Drive>; 4] = std::array::from_fn(|_| None);
        let Some(disk_value) = root.get("disk") else {
            return Ok(drives);
        };
        let disks = disk_value.as_array().ok_or_else(|| {
            format!(
                "{}: `disk` must be an array of [[disk]] tables",
                config_path.display()
            )
        })?;
        let config_parent = config_path.parent().unwrap_or_else(|| Path::new("."));

        for (index, disk_value) in disks.iter().enumerate() {
            let table = disk_value.as_table().ok_or_else(|| {
                format!(
                    "{}: disk entry {} must be a table",
                    config_path.display(),
                    index + 1
                )
            })?;
            let slot = table
                .get("slot")
                .and_then(toml::Value::as_integer)
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
                if !matches!(
                    key.as_str(),
                    "slot"
                        | "type"
                        | "image"
                        | "read_only"
                        | "transfer_rate_bytes_per_second"
                        | "access_latency_ms"
                ) {
                    return Err(slot_error(&format!("unknown configuration key `{key}`")));
                }
            }

            let kind_name = table
                .get("type")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| slot_error("`type` must be `fdd` or `hdd`"))?;
            let kind = DriveType::from_config(kind_name)
                .ok_or_else(|| slot_error(&format!("unknown drive type `{kind_name}`")))?;
            let image_name = table
                .get("image")
                .and_then(toml::Value::as_str)
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
                    let rate = value.as_integer().ok_or_else(|| {
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
                    let latency = value.as_integer().ok_or_else(|| {
                        slot_error("`access_latency_ms` must be a non-negative integer")
                    })?;
                    if latency < 0 {
                        return Err(slot_error("`access_latency_ms` must not be negative"));
                    }
                    latency as u64
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
            if length == 0 || length % SECTOR_SIZE as u64 != 0 {
                return Err(slot_error(&format!(
                    "image {} must be non-empty and its length must be a multiple of 512 bytes",
                    image_path.display()
                )));
            }
            let capacity = length / SECTOR_SIZE as u64;
            let capacity_sectors = u32::try_from(capacity).map_err(|_| {
                slot_error(&format!(
                    "image {} exceeds the 32-bit sector capacity",
                    image_path.display()
                ))
            })?;
            drives[slot] = Some(Drive {
                kind,
                image,
                capacity_sectors,
                read_only,
                transfer_rate_bytes_per_second,
                access_latency_ms,
            });
        }
        Ok(drives)
    }

    pub(crate) fn tick(&mut self) -> Option<usize> {
        self.cycle = self.cycle.saturating_add(1);
        let Some(active) = self.active.as_ref() else {
            return None;
        };
        let slot = active.slot;
        let direction = active.direction;
        let sector_index = active.completed_sectors;
        let deadline_sector_count = sector_index.saturating_add(1);
        let deadline = {
            let drive = self.drives[slot]
                .as_ref()
                .expect("active IDC command has a drive");
            active.deadline(drive, deadline_sector_count)
        };

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
                if complete {
                    self.finish_success();
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
        let (abort_at_boundary, complete_read) = {
            let Some(active) = self.active.as_mut() else {
                return false;
            };
            if active.dma_channel != Some(channel) {
                return false;
            }
            active.dma_bytes_completed += 1;
            let complete_read =
                active.direction == DmaDirection::DeviceToMemory && active.cursor == SECTOR_SIZE;
            if complete_read {
                active.completed_sectors += 1;
                active.cursor = 0;
                active.read_ready = false;
            }
            (
                active.abort_requested && active.dma_bytes_completed % SECTOR_SIZE as u64 == 0,
                complete_read && active.completed_sectors == active.sector_count,
            )
        };
        if abort_at_boundary {
            self.finish_error(5);
            return true;
        }
        if complete_read {
            let finished = self
                .active
                .as_ref()
                .is_some_and(|active| active.completed_sectors == active.sector_count);
            if finished {
                self.finish_success();
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
            buffer: [0; SECTOR_SIZE],
            cursor: 0,
            read_ready: false,
            dma_channel: None,
            abort_requested: false,
        });
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
    let drives = Idc::load_drives(Path::new("config.toml"))?;
    Ok(connect_idc(bus, drives))
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
        let config = root.join("config.toml");
        fs::write(
            &config,
            format!(
                "[[disk]]\nslot = 0\ntype = \"fdd\"\nimage = \"disk.img\"\nread_only = {read_only}\ntransfer_rate_bytes_per_second = {}\naccess_latency_ms = {latency_ms}\n",
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

        let duplicate = root.join("duplicate.toml");
        fs::write(
            &duplicate,
            "[[disk]]\nslot=0\ntype=\"fdd\"\nimage=\"disk.img\"\n[[disk]]\nslot=0\ntype=\"hdd\"\nimage=\"disk.img\"\n",
        )
        .unwrap();
        assert!(
            Idc::load_drives(&duplicate)
                .unwrap_err()
                .contains("disk slot 0")
        );

        fs::write(&image_path, [0u8; SECTOR_SIZE + 1]).unwrap();
        let invalid_length = root.join("invalid-length.toml");
        fs::write(
            &invalid_length,
            "[[disk]]\nslot=0\ntype=\"fdd\"\nimage=\"disk.img\"\n",
        )
        .unwrap();
        assert!(
            Idc::load_drives(&invalid_length)
                .unwrap_err()
                .contains("multiple of 512")
        );

        fs::write(&image_path, [0u8; SECTOR_SIZE]).unwrap();
        let missing = root.join("missing.toml");
        fs::write(
            &missing,
            "[[disk]]\nslot=0\ntype=\"fdd\"\nimage=\"absent.img\"\n",
        )
        .unwrap();
        assert!(
            Idc::load_drives(&missing)
                .unwrap_err()
                .contains("disk slot 0")
        );

        let zero_rate = root.join("zero-rate.toml");
        fs::write(
            &zero_rate,
            "[[disk]]\nslot=0\ntype=\"fdd\"\nimage=\"disk.img\"\ntransfer_rate_bytes_per_second=0\n",
        )
        .unwrap();
        assert!(
            Idc::load_drives(&zero_rate)
                .unwrap_err()
                .contains("greater than zero")
        );
        let negative_latency = root.join("negative-latency.toml");
        fs::write(
            &negative_latency,
            "[[disk]]\nslot=0\ntype=\"fdd\"\nimage=\"disk.img\"\naccess_latency_ms=-1\n",
        )
        .unwrap();
        assert!(
            Idc::load_drives(&negative_latency)
                .unwrap_err()
                .contains("must not be negative")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn drive_type_profiles_supply_rate_latency_and_writable_default() {
        let root = temp_root();
        fs::write(root.join("fdd.img"), [0u8; SECTOR_SIZE]).unwrap();
        fs::write(root.join("hdd.img"), [0u8; SECTOR_SIZE]).unwrap();
        let config = root.join("config.toml");
        fs::write(
            &config,
            "[[disk]]\nslot=0\ntype=\"fdd\"\nimage=\"fdd.img\"\n[[disk]]\nslot=1\ntype=\"hdd\"\nimage=\"hdd.img\"\n",
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
    fn missing_config_leaves_all_slots_unconnected() {
        let root = temp_root();
        let drives = Idc::load_drives(&root.join("config.toml")).unwrap();
        assert!(drives.iter().all(Option::is_none));
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
