use std::collections::{HashMap, HashSet};
use std::io::{Read, SeekFrom};

use zip::{CompressionMethod, ZipArchive};

pub(super) const MANIFEST_PATH: &str = "manifest.yaml";
pub(super) const MANIFEST_LIMIT: u64 = 1024 * 1024;
pub(super) const FILE_LIMIT: u64 = 64 * 1024 * 1024;

pub(super) struct PackageArchive<R> {
    archive: ZipArchive<R>,
    files: HashMap<String, u64>,
}

impl<R: Read + std::io::Seek> PackageArchive<R> {
    pub(super) fn new(reader: R) -> Result<Self, String> {
        let archive = ZipArchive::new(reader).map_err(|e| format!("invalid ZIP archive: {e}"))?;
        let central_start = archive.central_directory_start();
        let mut reader = archive.into_inner();
        reject_duplicate_central_names(&mut reader, central_start)?;
        let mut archive =
            ZipArchive::new(reader).map_err(|e| format!("invalid ZIP archive: {e}"))?;
        let mut files = HashMap::new();
        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .map_err(|e| format!("invalid ZIP entry {index}: {e}"))?;
            let raw_name = entry.name();
            let directory = entry.is_dir();
            let normalized = if directory {
                raw_name.strip_suffix('/').unwrap_or(raw_name)
            } else {
                raw_name
            };
            if !safe_path(normalized) {
                return Err(format!("unsafe or invalid ZIP entry path {raw_name:?}"));
            }
            if !matches!(
                entry.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            ) {
                return Err(format!(
                    "unsupported compression for ZIP entry {raw_name:?}"
                ));
            }
            if directory {
                if entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o040000)
                {
                    return Err(format!("ZIP entry {raw_name:?} is not a directory"));
                }
            } else {
                if !entry.is_file()
                    || entry
                        .unix_mode()
                        .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
                {
                    return Err(format!("ZIP entry {raw_name:?} is not a regular file"));
                }
                if entry.size() > FILE_LIMIT {
                    return Err(format!(
                        "ZIP entry {raw_name:?} exceeds 64 MiB uncompressed"
                    ));
                }
                files.insert(normalized.to_owned(), entry.size());
            }
        }
        if !files.contains_key(MANIFEST_PATH) {
            return Err(format!("missing {MANIFEST_PATH}"));
        }
        Ok(Self { archive, files })
    }

    pub(super) fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    pub(super) fn read(&mut self, path: &str, limit: u64) -> Result<Vec<u8>, String> {
        let expected = *self
            .files
            .get(path)
            .ok_or_else(|| format!("missing ZIP file {path:?}"))?;
        if expected > limit {
            return Err(format!("ZIP entry {path:?} exceeds its size limit"));
        }
        let capacity =
            usize::try_from(expected).map_err(|_| format!("ZIP entry {path:?} is too large"))?;
        let entry = self
            .archive
            .by_name(path)
            .map_err(|e| format!("cannot read ZIP entry {path:?}: {e}"))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|e| format!("cannot allocate ZIP entry {path:?}: {e}"))?;
        let bounded_size = expected
            .checked_add(1)
            .ok_or_else(|| format!("ZIP entry {path:?} size overflow"))?;
        entry
            .take(bounded_size)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("cannot decompress ZIP entry {path:?}: {e}"))?;
        if bytes.len() != capacity {
            return Err(format!(
                "ZIP entry {path:?} has an inconsistent uncompressed size"
            ));
        }
        Ok(bytes)
    }
}

const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ',
    'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ',
    'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕',
    '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦',
    '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐',
    '▀', 'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', '≡', '±',
    '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{00A0}',
];

fn decode_zip_name(bytes: &[u8], utf8: bool) -> String {
    if utf8 {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        bytes
            .iter()
            .map(|&byte| {
                if byte < 0x80 {
                    char::from(byte)
                } else {
                    CP437_HIGH[usize::from(byte - 0x80)]
                }
            })
            .collect()
    }
}

fn reject_duplicate_central_names<R: Read + std::io::Seek>(
    reader: &mut R,
    central_start: u64,
) -> Result<(), String> {
    const CENTRAL_FILE_HEADER: u32 = 0x0201_4B50;
    reader
        .seek(SeekFrom::Start(central_start))
        .map_err(|error| format!("cannot seek to ZIP central directory: {error}"))?;
    let mut names = HashSet::new();
    loop {
        let header_start = reader
            .stream_position()
            .map_err(|error| format!("cannot inspect ZIP central directory: {error}"))?;
        let mut header = [0u8; 46];
        match reader.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(format!("cannot read ZIP central directory: {error}")),
        }
        if u32::from_le_bytes(header[..4].try_into().unwrap()) != CENTRAL_FILE_HEADER {
            return Ok(());
        }
        let flags = u16::from_le_bytes(header[8..10].try_into().unwrap());
        let name_len = u16::from_le_bytes(header[28..30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(header[30..32].try_into().unwrap()) as u64;
        let comment_len = u16::from_le_bytes(header[32..34].try_into().unwrap()) as u64;
        let mut raw_name = vec![0; name_len];
        reader
            .read_exact(&mut raw_name)
            .map_err(|error| format!("cannot read ZIP entry name: {error}"))?;
        let mut name = decode_zip_name(&raw_name, flags & (1 << 11) != 0);
        if name.ends_with('/') {
            name.pop();
        }
        if names.contains(&name) {
            return Err(format!("duplicate ZIP entry {name:?}"));
        }
        names.insert(name);
        let skip = extra_len + comment_len;
        let skip = i64::try_from(skip).map_err(|_| "ZIP central entry is too large".to_owned())?;
        reader
            .seek(SeekFrom::Current(skip))
            .map_err(|error| format!("cannot skip ZIP central entry: {error}"))?;
        if reader
            .stream_position()
            .map_err(|error| format!("cannot inspect ZIP central directory: {error}"))?
            <= header_start
        {
            return Err("invalid ZIP central directory entry length".to_owned());
        }
    }
}

pub(super) fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
