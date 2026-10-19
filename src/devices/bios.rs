//! BIOS ROM: read-only firmware window loaded from `config.yaml`.
//!
//! The `bios.image` key names a raw ROM image, or a WAX executable whose
//! load extent lies inside the ROM window. The image is mapped read-only at
//! [`BIOS_BASE`]; the mapped window is always the full 64 KiB, zero-padded.
//! Guest writes to the window are ignored. When configured, the CPU boots
//! from the ROM entry instead of the RAM program entry.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bus::Device;

pub const BIOS_BASE: u32 = 0xFF00_0000;
pub const BIOS_SIZE: usize = 0x1_0000;
const BIOS_END: u64 = BIOS_BASE as u64 + BIOS_SIZE as u64;

/// Read-only BIOS window, always mapped at full size and zero-padded.
pub struct BiosRom {
    data: Box<[u8; BIOS_SIZE]>,
}

impl BiosRom {
    /// Raw image loaded at the ROM base.
    pub fn from_raw(image: &[u8]) -> Result<Self, String> {
        if image.is_empty() {
            return Err("BIOS image is empty".to_owned());
        }
        if image.len() > BIOS_SIZE {
            return Err(format!(
                "BIOS image is {} bytes, larger than the 64 KiB ROM window",
                image.len()
            ));
        }
        let mut data = Box::new([0u8; BIOS_SIZE]);
        data[..image.len()].copy_from_slice(image);
        Ok(Self { data })
    }

    /// WAX executable whose load extent lies inside the ROM window.
    /// Returns the device and the boot entry address.
    pub fn from_wax(image: &crate::wax::WaxImage<'_>) -> Result<(Self, u32), String> {
        if image.is_lib {
            return Err("cannot execute .wal library as BIOS".to_owned());
        }
        let mut data = Box::new([0u8; BIOS_SIZE]);
        for block in &image.blocks {
            let block_start = u64::from(block.load_address);
            let block_end = block_start + block.data.len() as u64;
            if block_start < u64::from(BIOS_BASE) || block_end > BIOS_END {
                return Err(format!(
                    "BIOS WAX block at 0x{:08X} ({} bytes) lies outside the ROM window",
                    block.load_address,
                    block.data.len(),
                ));
            }
            let offset = (block_start - u64::from(BIOS_BASE)) as usize;
            data[offset..offset + block.data.len()].copy_from_slice(block.data);
        }
        Ok((Self { data }, image.entry))
    }
}

impl Device for BiosRom {
    fn read(&mut self, addr: u32) -> u8 {
        self.data[addr as usize]
    }

    fn debug_read(&self, addr: u32) -> Option<u8> {
        self.data.get(addr as usize).copied()
    }

    fn write(&mut self, _addr: u32, _value: u8) {
        // ROM is read-only: guest stores are silently ignored.
    }

    fn size(&self) -> u32 {
        BIOS_SIZE as u32
    }
}

/// Load the configured BIOS image into a read-only bus window.
///
/// Returns the boot PC (`Some`) when `bios.image` is configured, or `None`
/// when the config file, the key, or the section is absent, so callers keep
/// the RAM program entry in that case.
pub fn load_bios(config_path: &Path, bus: &mut crate::bus::Bus) -> Result<Option<u32>, String> {
    let contents = match fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", config_path.display())),
    };
    let root: yaml_serde::Value = yaml_serde::from_str(&contents)
        .map_err(|error| format!("invalid YAML in {}: {error}", config_path.display()))?;
    let Some(bios) = root.get("bios") else {
        return Ok(None);
    };
    if bios.is_null() {
        return Ok(None);
    }
    let table = bios
        .as_mapping()
        .ok_or_else(|| format!("`bios` in {} must be a mapping", config_path.display()))?;
    let image_name = table
        .get("image")
        .and_then(yaml_serde::Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            format!(
                "`bios.image` in {} must be a non-empty path string",
                config_path.display()
            )
        })?;
    let image_path = PathBuf::from(image_name);
    let image_path = if image_path.is_absolute() {
        image_path
    } else {
        config_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .join(image_path)
    };
    let bytes = fs::read(&image_path)
        .map_err(|error| format!("cannot read BIOS image {}: {error}", image_path.display()))?;
    if crate::wax::is_wax(&bytes) {
        let image = crate::wax::parse(&bytes)
            .map_err(|error| format!("invalid WAX BIOS image {}: {error}", image_path.display()))?;
        let (rom, entry) = BiosRom::from_wax(&image)?;
        bus.add_device(BIOS_BASE, "BIOS ROM", Box::new(rom));
        tracing::info!(path = %image_path.display(), entry, "Loaded WAX BIOS ROM");
        return Ok(Some(entry));
    }
    let len = bytes.len();
    let rom = BiosRom::from_raw(&bytes)
        .map_err(|error| format!("invalid BIOS image {}: {error}", image_path.display()))?;
    bus.add_device(BIOS_BASE, "BIOS ROM", Box::new(rom));
    tracing::info!(path = %image_path.display(), bytes = len, "Loaded BIOS ROM");
    Ok(Some(BIOS_BASE))
}
