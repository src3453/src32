use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;
use zip::{CompressionMethod, ZipArchive};

const MANIFEST_PATH: &str = "manifest.yaml";
const MANIFEST_LIMIT: u64 = 1024 * 1024;
const ASSET_TOTAL_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    abi_version: u32,
    module: String,
    #[serde(default)]
    asset: Vec<ManifestAsset>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestAsset {
    id: u32,
    path: String,
}

pub(super) struct PackageAssets {
    archive: ZipArchive<File>,
    paths: Vec<String>,
    sizes: Vec<u64>,
    cache: Vec<Option<Vec<u8>>>,
}

impl PackageAssets {
    pub(super) fn load(path: &Path) -> Result<(Vec<u8>, Self), String> {
        let file = File::open(path).map_err(|error| error.to_string())?;
        let mut archive = ZipArchive::new(file).map_err(|error| error.to_string())?;
        let mut entries = HashSet::with_capacity(archive.len());
        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .map_err(|error| format!("invalid ZIP entry {index}: {error}"))?;
            if !safe_archive_path(entry.name()) {
                return Err(format!(
                    "unsafe or invalid ZIP entry path {:?}",
                    entry.name()
                ));
            }
            if !entry.is_file() {
                return Err(format!(
                    "ZIP entry {:?} is not a regular file",
                    entry.name()
                ));
            }
            if !matches!(
                entry.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            ) {
                return Err(format!(
                    "unsupported compression for ZIP entry {:?}",
                    entry.name()
                ));
            }
            if !entries.insert(entry.name().to_owned()) {
                return Err(format!("duplicate ZIP entry {:?}", entry.name()));
            }
        }
        if !entries.contains(MANIFEST_PATH) {
            return Err(format!("missing {MANIFEST_PATH}"));
        }

        let manifest_bytes = read_entry(&mut archive, MANIFEST_PATH, MANIFEST_LIMIT)?;
        let manifest_text = std::str::from_utf8(&manifest_bytes)
            .map_err(|error| format!("{MANIFEST_PATH} is not UTF-8: {error}"))?;
        let manifest: Manifest = yaml_serde::from_str(manifest_text)
            .map_err(|error| format!("invalid {MANIFEST_PATH}: {error}"))?;
        if manifest.abi_version != 1 {
            return Err(format!(
                "manifest ABI version {} is unsupported; expected 1",
                manifest.abi_version
            ));
        }
        validate_manifest_path(&manifest.module)?;
        if manifest.module == MANIFEST_PATH {
            return Err("module path must not name manifest.yaml".to_owned());
        }

        let module = read_entry(&mut archive, &manifest.module, u64::MAX)
            .map_err(|error| format!("module {:?}: {error}", manifest.module))?;

        let count = manifest.asset.len();
        let mut by_id: Vec<Option<String>> = std::iter::repeat_with(|| None).take(count).collect();
        let mut unique_paths = HashSet::with_capacity(count + 1);
        unique_paths.insert(manifest.module.clone());
        let mut sizes = vec![0; count];
        let mut total_size = 0u64;
        for asset in manifest.asset {
            validate_manifest_path(&asset.path)?;
            if asset.path == MANIFEST_PATH || !unique_paths.insert(asset.path.clone()) {
                return Err(format!("duplicate or reserved asset path {:?}", asset.path));
            }
            let id = usize::try_from(asset.id)
                .map_err(|_| format!("asset ID {} cannot be represented", asset.id))?;
            let Some(path_slot) = by_id.get_mut(id) else {
                return Err(format!(
                    "asset IDs must be contiguous from 0; found {}",
                    asset.id
                ));
            };
            if path_slot.is_some() {
                return Err(format!("duplicate asset ID {}", asset.id));
            }
            let size = entry_size(&mut archive, &asset.path)
                .map_err(|error| format!("asset {:?}: {error}", asset.path))?;
            total_size = total_size
                .checked_add(size)
                .ok_or_else(|| "asset size total overflow".to_owned())?;
            if total_size > ASSET_TOTAL_LIMIT {
                return Err("package assets exceed 64 MiB uncompressed".to_owned());
            }
            sizes[id] = size;
            *path_slot = Some(asset.path);
        }
        if by_id.iter().any(Option::is_none) {
            return Err("asset IDs must be contiguous from 0".to_owned());
        }
        let paths = by_id.into_iter().flatten().collect::<Vec<_>>();
        if i32::try_from(paths.len()).is_err()
            || sizes.iter().any(|&size| i32::try_from(size).is_err())
        {
            return Err("asset count and sizes must fit in i32".to_owned());
        }
        Ok((
            module,
            Self {
                archive,
                paths,
                sizes,
                cache: std::iter::repeat_with(|| None).take(count).collect(),
            },
        ))
    }

    pub(super) fn count(&self) -> i32 {
        self.paths.len() as i32
    }

    pub(super) fn size(&self, id: i32) -> wasmtime::Result<i32> {
        let index = usize::try_from(id)
            .map_err(|_| wasmtime::Error::msg("asset_size received an invalid asset ID"))?;
        let size = *self
            .sizes
            .get(index)
            .ok_or_else(|| wasmtime::Error::msg("asset_size received an invalid asset ID"))?;
        Ok(size as i32)
    }

    pub(super) fn read_byte(&mut self, id: i32, offset: i32) -> wasmtime::Result<i32> {
        let index = usize::try_from(id)
            .map_err(|_| wasmtime::Error::msg("asset_read_byte received an invalid asset ID"))?;
        let offset = usize::try_from(offset)
            .map_err(|_| wasmtime::Error::msg("asset_read_byte received a negative offset"))?;
        let size = *self
            .sizes
            .get(index)
            .ok_or_else(|| wasmtime::Error::msg("asset_read_byte received an invalid asset ID"))?;
        if offset >= size as usize {
            return Err(wasmtime::Error::msg(
                "asset_read_byte offset is out of range",
            ));
        }
        if self.cache[index].is_none() {
            let mut entry = self
                .archive
                .by_name(&self.paths[index])
                .map_err(|error| wasmtime::Error::msg(error.to_string()))?;
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(size as usize)
                .map_err(|error| wasmtime::Error::msg(error.to_string()))?;
            entry
                .read_to_end(&mut bytes)
                .map_err(|error| wasmtime::Error::msg(error.to_string()))?;
            if bytes.len() as u64 != size {
                return Err(wasmtime::Error::msg("asset size changed while reading"));
            }
            self.cache[index] = Some(bytes);
        }
        Ok(i32::from(self.cache[index].as_ref().unwrap()[offset]))
    }
}

fn read_entry(
    archive: &mut ZipArchive<File>,
    path: &str,
    max_size: u64,
) -> Result<Vec<u8>, String> {
    let mut entry = archive
        .by_name(path)
        .map_err(|error| format!("cannot read ZIP entry {path:?}: {error}"))?;
    if !entry.is_file() {
        return Err(format!("ZIP entry {path:?} is not a regular file"));
    }
    let expected_size = entry.size();
    if expected_size > max_size {
        return Err(format!("ZIP entry {path:?} exceeds its size limit"));
    }
    let capacity = usize::try_from(expected_size)
        .map_err(|_| format!("ZIP entry {path:?} is too large for this host"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|error| format!("cannot allocate ZIP entry {path:?}: {error}"))?;
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot decompress ZIP entry {path:?}: {error}"))?;
    if bytes.len() as u64 != expected_size {
        return Err(format!("ZIP entry {path:?} has an inconsistent size"));
    }
    Ok(bytes)
}

fn entry_size(archive: &mut ZipArchive<File>, path: &str) -> Result<u64, String> {
    let entry = archive
        .by_name(path)
        .map_err(|error| format!("missing ZIP entry {path:?}: {error}"))?;
    if !entry.is_file() {
        return Err(format!("ZIP entry {path:?} is not a regular file"));
    }
    Ok(entry.size())
}

fn validate_manifest_path(path: &str) -> Result<(), String> {
    if !safe_archive_path(path) {
        return Err(format!("invalid package path {path:?}"));
    }
    Ok(())
}

fn safe_archive_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
