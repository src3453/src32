mod archive;
mod manifest;

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use archive::{FILE_LIMIT, MANIFEST_LIMIT, MANIFEST_PATH, PackageArchive, safe_path};
use manifest::Manifest;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CartridgeDiskKind {
    Fdd,
    Hdd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CartridgeDiskMode {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug)]
pub struct CartridgeDisk {
    pub slot: u8,
    pub kind: CartridgeDiskKind,
    pub mode: CartridgeDiskMode,
    pub image: Arc<[u8]>,
}

#[derive(Debug)]
pub struct CartridgePlugin {
    pub slot: u8,
    pub path: String,
    pub package: Vec<u8>,
}

#[derive(Debug)]
pub struct Cartridge {
    pub id: String,
    pub title: String,
    pub program: Vec<u8>,
    pub disks: Vec<CartridgeDisk>,
    pub plugins: Vec<CartridgePlugin>,
}

impl Cartridge {
    pub fn load(path: &Path) -> Result<Self, String> {
        let file = File::open(path)
            .map_err(|e| format!("cannot open cartridge {}: {e}", path.display()))?;
        Self::load_reader(file)
    }

    fn load_reader<R: Read + std::io::Seek>(reader: R) -> Result<Self, String> {
        let mut archive = PackageArchive::new(reader)?;
        let manifest_bytes = archive.read(MANIFEST_PATH, MANIFEST_LIMIT)?;
        let text = std::str::from_utf8(&manifest_bytes)
            .map_err(|e| format!("{MANIFEST_PATH} is not UTF-8: {e}"))?;
        let manifest: Manifest =
            yaml_serde::from_str(text).map_err(|e| format!("invalid {MANIFEST_PATH}: {e}"))?;
        validate_manifest(&manifest)?;

        let mut referenced =
            HashSet::with_capacity(1 + manifest.disks.len() + manifest.plugins.len());
        referenced.insert(MANIFEST_PATH.to_owned());
        let mut add_path = |p: &str| -> Result<(), String> {
            if !safe_path(p) {
                return Err(format!("invalid package path {p:?}"));
            }
            if !referenced.insert(p.to_owned()) {
                return Err(format!(
                    "package path {p:?} is referenced by multiple roles or is manifest.yaml"
                ));
            }
            if !archive.contains(p) {
                return Err(format!("missing referenced package file {p:?}"));
            }
            Ok(())
        };
        add_path(&manifest.entrypoint.path)?;
        for disk in &manifest.disks {
            add_path(&disk.path)?;
        }
        for plugin in &manifest.plugins {
            add_path(&plugin.path)?;
        }
        drop(add_path);

        let program = archive.read(&manifest.entrypoint.path, FILE_LIMIT)?;
        if program.is_empty() {
            return Err("entrypoint program must not be empty".to_owned());
        }
        if program.len() > 16 * 1024 * 1024 {
            return Err("entrypoint program exceeds 16 MiB main RAM".to_owned());
        }

        let mut disks = Vec::with_capacity(manifest.disks.len());
        for disk in manifest.disks {
            let bytes = archive
                .read(&disk.path, FILE_LIMIT)
                .map_err(|e| format!("disk {:?}: {e}", disk.path))?;
            if bytes.is_empty() {
                return Err(format!("disk {:?} image must not be empty", disk.path));
            }
            if bytes.len() % 512 != 0 {
                return Err(format!(
                    "disk {:?} image size must be a multiple of 512 bytes",
                    disk.path
                ));
            }
            let sectors = bytes.len() / 512;
            if sectors > u32::MAX as usize {
                return Err(format!(
                    "disk {:?} exceeds 32-bit sector capacity",
                    disk.path
                ));
            }
            disks.push(CartridgeDisk {
                slot: disk.slot,
                kind: if disk.kind == "fdd" {
                    CartridgeDiskKind::Fdd
                } else {
                    CartridgeDiskKind::Hdd
                },
                mode: if disk.mode == "read_only" {
                    CartridgeDiskMode::ReadOnly
                } else {
                    CartridgeDiskMode::ReadWrite
                },
                image: Arc::from(bytes),
            });
        }
        let mut plugins = Vec::with_capacity(manifest.plugins.len());
        for plugin in manifest.plugins {
            let bytes = archive
                .read(&plugin.path, FILE_LIMIT)
                .map_err(|e| format!("plugin {:?}: {e}", plugin.path))?;
            plugins.push(CartridgePlugin {
                slot: plugin.slot,
                path: plugin.path,
                package: bytes,
            });
        }
        Ok(Self {
            id: manifest.id,
            title: manifest.title,
            program,
            disks,
            plugins,
        })
    }
}

fn validate_manifest(m: &Manifest) -> Result<(), String> {
    if m.format != "cpt32pkg" {
        return Err(format!("unsupported package format {:?}", m.format));
    }
    if m.format_version != 1 {
        return Err(format!(
            "unsupported package format_version {}; expected 1",
            m.format_version
        ));
    }
    if !canonical_uuid(&m.id) {
        return Err("id must be a canonical lowercase UUID".to_owned());
    }
    if m.title.trim().is_empty() {
        return Err("title must not be empty".to_owned());
    }
    if m.system.architecture != "SRC32" {
        return Err(format!(
            "unsupported architecture {:?}",
            m.system.architecture
        ));
    }
    if m.entrypoint.kind != "program" {
        return Err(format!(
            "unsupported entrypoint type {:?}",
            m.entrypoint.kind
        ));
    }
    let mut disk_slots = HashSet::new();
    for d in &m.disks {
        if d.slot > 3 || !disk_slots.insert(d.slot) {
            return Err(format!(
                "disk slots must be unique values in 0..=3; invalid slot {}",
                d.slot
            ));
        }
        if d.kind != "fdd" && d.kind != "hdd" {
            return Err(format!("invalid disk type {:?}", d.kind));
        }
        if d.mode != "read_only" && d.mode != "read_write" {
            return Err(format!("invalid disk mode {:?}", d.mode));
        }
    }
    let mut plugin_slots = HashSet::new();
    for p in &m.plugins {
        if p.slot > 7 || !plugin_slots.insert(p.slot) {
            return Err(format!(
                "plugin slots must be unique values in 0..=7; invalid slot {}",
                p.slot
            ));
        }
        if !p.path.ends_with(".cpt32card") {
            return Err(format!(
                "plugin path {:?} must name a .cpt32card package",
                p.path
            ));
        }
    }
    Ok(())
}

fn canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

pub fn default_user_data_root() -> Result<std::path::PathBuf, String> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_dir().join("CPT32").join("user_data"))
        .ok_or_else(|| "OS application data directory unavailable".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_requires_canonical_lowercase_shape() {
        assert!(canonical_uuid("00000000-0000-4000-8000-000000000001"));
        assert!(!canonical_uuid("00000000-0000-4000-8000-00000000000A"));
        assert!(!canonical_uuid("00000000000040008000000000000001"));
    }

    #[test]
    fn manifest_rejects_unknown_fields() {
        let yaml = "format: cpt32pkg\nformat_version: 1\nid: 00000000-0000-4000-8000-000000000001\ntitle: test\nsystem:\n  architecture: SRC32\nentrypoint:\n  type: program\n  path: program.bin\nextra: true\n";
        assert!(yaml_serde::from_str::<Manifest>(yaml).is_err());
    }
}
