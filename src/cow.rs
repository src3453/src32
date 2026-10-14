use crc32c::crc32c;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const HEADER_SIZE: u64 = 64;
const RECORD_HEADER_SIZE: u64 = 32;
const HEADER_MAGIC: &[u8; 8] = b"CPTCOW1\0";
const RECORD_MAGIC: &[u8; 4] = b"BLK1";

#[derive(Debug)]
pub enum CowError {
    Io(io::Error),
    Invalid(String),
    BaseMismatch,
    ReadOnly,
    WritePoisoned,
}
impl std::fmt::Display for CowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "COW I/O error: {e}"),
            Self::Invalid(s) => f.write_str(s),
            Self::BaseMismatch => f.write_str("COW base image does not match"),
            Self::ReadOnly => f.write_str("COW overlay is read-only"),
            Self::WritePoisoned => f.write_str("COW overlay is unusable after failed rollback"),
        }
    }
}
impl std::error::Error for CowError {}
impl From<io::Error> for CowError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Immutable logical base image used beneath an append-only overlay.
pub enum BaseImage {
    File(File),
    Bytes(Arc<[u8]>),
}
impl BaseImage {
    fn len(&self) -> Result<u64, CowError> {
        match self {
            Self::File(f) => Ok(f.metadata()?.len()),
            Self::Bytes(b) => Ok(b.len() as u64),
        }
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), CowError> {
        match self {
            Self::File(f) => {
                f.seek(SeekFrom::Start(offset))?;
                f.read_exact(out)?;
            }
            Self::Bytes(b) => {
                let start = usize::try_from(offset)
                    .map_err(|_| CowError::Invalid("base offset overflow".into()))?;
                let end = start
                    .checked_add(out.len())
                    .ok_or_else(|| CowError::Invalid("base range overflow".into()))?;
                out.copy_from_slice(
                    b.get(start..end)
                        .ok_or_else(|| CowError::Invalid("base range out of bounds".into()))?,
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct CowScan {
    pub logical_size: u64,
    pub block_size: u32,
    pub device_type: u16,
    pub records: u64,
    pub recoverable_tail: bool,
    /// Latest record offset for each block index; offsets point to record data.
    pub latest_blocks: HashMap<u64, u64>,
    pub valid_end: u64,
}

fn hash_base(base: &mut BaseImage, size: u64) -> Result<[u8; 32], CowError> {
    let mut hash = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut at = 0;
    while at < size {
        let n = ((size - at).min(buf.len() as u64)) as usize;
        base.read_exact_at(at, &mut buf[..n])?;
        hash.update(&buf[..n]);
        at += n as u64;
    }
    Ok(hash.finalize().into())
}
fn validate_parameters(base: &mut BaseImage, block_size: u32) -> Result<(u64, [u8; 32]), CowError> {
    let size = base.len()?;
    if size == 0 {
        return Err(CowError::Invalid("base image must be non-empty".into()));
    }
    if !(1..=4096).contains(&block_size) {
        return Err(CowError::Invalid("block size must be in 1..=4096".into()));
    }
    let hash = hash_base(base, size)?;
    Ok((size, hash))
}
fn validate_device_type(device_type: u16) -> Result<(), CowError> {
    if matches!(device_type, 0..=3) {
        Ok(())
    } else {
        Err(CowError::Invalid("unsupported COW device type".into()))
    }
}

fn make_header(device_type: u16, block_size: u32, size: u64, hash: &[u8; 32]) -> [u8; 64] {
    let mut h = [0u8; 64];
    h[..8].copy_from_slice(HEADER_MAGIC);
    h[8..10].copy_from_slice(&1u16.to_le_bytes());
    h[10..12].copy_from_slice(&64u16.to_le_bytes());
    h[12..14].copy_from_slice(&device_type.to_le_bytes());
    h[16..20].copy_from_slice(&block_size.to_le_bytes());
    h[20..28].copy_from_slice(&size.to_le_bytes());
    h[28..60].copy_from_slice(hash);
    let crc = crc32c(&h[..60]);
    h[60..64].copy_from_slice(&crc.to_le_bytes());
    h
}

fn read_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn read_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn read_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Scan and validate a COW file without modifying it. A short final record is reported as recoverable tail.
pub fn scan_cow(
    mut base: BaseImage,
    cow_path: &Path,
    device_type: u16,
    block_size: u32,
) -> Result<CowScan, CowError> {
    validate_device_type(device_type)?;
    let (logical_size, base_hash) = validate_parameters(&mut base, block_size)?;
    let mut file = File::open(cow_path)?;
    let file_len = file.metadata()?.len();
    if file_len < HEADER_SIZE {
        return Err(CowError::Invalid("truncated COW header".into()));
    }
    let mut h = [0u8; 64];
    file.read_exact(&mut h)?;
    if &h[..8] != HEADER_MAGIC
        || read_u16(&h, 8) != 1
        || read_u16(&h, 10) != 64
        || read_u16(&h, 14) != 0
        || crc32c(&h[..60]) != read_u32(&h, 60)
    {
        return Err(CowError::Invalid("invalid COW header".into()));
    }
    if read_u16(&h, 12) != device_type
        || read_u32(&h, 16) != block_size
        || read_u64(&h, 20) != logical_size
    {
        return Err(CowError::Invalid("COW parameters do not match base".into()));
    }
    if h[28..60] != base_hash {
        return Err(CowError::BaseMismatch);
    }
    let blocks = logical_size
        .checked_add(block_size as u64 - 1)
        .ok_or_else(|| CowError::Invalid("logical block count overflow".into()))?
        / block_size as u64;
    let mut latest_blocks = HashMap::new();
    let mut offset = HEADER_SIZE;
    let mut expected_seq = 1u64;
    let mut records = 0u64;
    let mut tail = false;
    while offset < file_len {
        let remain = file_len - offset;
        if remain < RECORD_HEADER_SIZE {
            tail = true;
            break;
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut rh = [0u8; 32];
        file.read_exact(&mut rh)?;
        let data_end = offset
            .checked_add(RECORD_HEADER_SIZE)
            .and_then(|v| v.checked_add(block_size as u64))
            .ok_or_else(|| CowError::Invalid("record offset overflow".into()))?;
        if data_end > file_len {
            tail = true;
            break;
        }
        if &rh[..4] != RECORD_MAGIC || read_u32(&rh, 28) != 0 {
            return Err(CowError::Invalid("invalid COW record header".into()));
        }
        let header_crc = read_u32(&rh, 24);
        rh[24..28].fill(0);
        if crc32c(&rh) != header_crc {
            return Err(CowError::Invalid("invalid COW record header CRC32C".into()));
        }
        let index = read_u64(&rh, 4);
        let seq = read_u64(&rh, 12);
        if seq != expected_seq {
            return Err(CowError::Invalid("invalid COW record sequence".into()));
        }
        if index >= blocks {
            return Err(CowError::Invalid("COW block index out of range".into()));
        }
        let data_crc = read_u32(&rh, 20);
        let data_offset = offset + RECORD_HEADER_SIZE;
        file.seek(SeekFrom::Start(data_offset))?;
        let mut crc = 0u32;
        let mut remaining = block_size as usize;
        let mut buf = [0u8; 4096];
        while remaining > 0 {
            let n = remaining.min(buf.len());
            file.read_exact(&mut buf[..n])?;
            crc = crc32c_append(crc, &buf[..n]);
            remaining -= n;
        }
        if crc != data_crc {
            return Err(CowError::Invalid("invalid COW block data CRC32C".into()));
        }
        latest_blocks.insert(index, data_offset);
        records = records
            .checked_add(1)
            .ok_or_else(|| CowError::Invalid("record count overflow".into()))?;
        expected_seq = expected_seq
            .checked_add(1)
            .ok_or_else(|| CowError::Invalid("record sequence overflow".into()))?;
        offset = data_end;
    }
    Ok(CowScan {
        logical_size,
        block_size,
        device_type,
        records,
        recoverable_tail: tail,
        latest_blocks,
        valid_end: offset,
    })
}
/// Read COW parameters from its header, then perform the standard full scan.
pub fn scan_cow_header(base: BaseImage, cow_path: &Path) -> Result<CowScan, CowError> {
    let mut file = File::open(cow_path)?;
    let mut header = [0u8; 64];
    file.read_exact(&mut header)?;
    let device_type = read_u16(&header, 12);
    let block_size = read_u32(&header, 16);
    scan_cow(base, cow_path, device_type, block_size)
}

fn crc32c_append(previous: u32, bytes: &[u8]) -> u32 {
    // crc32c crate exposes the finalized-CRC continuation API.
    crc32c::crc32c_append(previous, bytes)
}

pub struct CowOverlay {
    base: BaseImage,
    cow: Option<File>,
    cow_path: PathBuf,
    device_type: u16,
    base_hash: [u8; 32],
    scan: CowScan,
    next_sequence: u64,
    committed_end: u64,
    poisoned: bool,
}

impl CowOverlay {
    pub fn open(
        mut base: BaseImage,
        cow_path: &Path,
        device_type: u16,
        block_size: u32,
    ) -> Result<Self, CowError> {
        validate_device_type(device_type)?;
        let (logical_size, base_hash) = validate_parameters(&mut base, block_size)?;
        let (cow, scan) = match OpenOptions::new().read(true).write(true).open(cow_path) {
            Ok(mut file) => {
                if file.metadata()?.len() == 0 {
                    return Err(CowError::Invalid("empty COW file".into()));
                }
                let scan = scan_cow_from_file(&mut base, &mut file, device_type, block_size)?;
                (Some(file), scan)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (
                None,
                CowScan {
                    logical_size,
                    block_size,
                    device_type,
                    records: 0,
                    recoverable_tail: false,
                    latest_blocks: HashMap::new(),
                    valid_end: HEADER_SIZE,
                },
            ),
            Err(error) => return Err(error.into()),
        };
        let committed_end = scan.valid_end;
        let next_sequence = scan
            .records
            .checked_add(1)
            .ok_or_else(|| CowError::Invalid("record sequence overflow".into()))?;
        Ok(Self {
            base,
            cow,
            cow_path: cow_path.to_path_buf(),
            device_type,
            base_hash,
            scan,
            next_sequence,
            committed_end,
            poisoned: false,
        })
    }

    pub fn logical_size(&self) -> u64 {
        self.scan.logical_size
    }

    pub fn block_size(&self) -> u32 {
        self.scan.block_size
    }

    pub fn read_at(&mut self, offset: u64, output: &mut [u8]) -> Result<(), CowError> {
        let end = offset
            .checked_add(output.len() as u64)
            .ok_or_else(|| CowError::Invalid("read range overflow".into()))?;
        if end > self.scan.logical_size {
            return Err(CowError::Invalid("read range exceeds logical image".into()));
        }
        let block_size = u64::from(self.scan.block_size);
        let mut done = 0usize;
        while done < output.len() {
            let position = offset + done as u64;
            let block_index = position / block_size;
            let within_block = (position % block_size) as usize;
            let count = (output.len() - done).min(self.scan.block_size as usize - within_block);
            if let Some(&data_offset) = self.scan.latest_blocks.get(&block_index) {
                let cow = self
                    .cow
                    .as_mut()
                    .ok_or_else(|| CowError::Invalid("COW index has no backing file".into()))?;
                cow.seek(SeekFrom::Start(data_offset + within_block as u64))?;
                cow.read_exact(&mut output[done..done + count])?;
            } else {
                self.base
                    .read_exact_at(position, &mut output[done..done + count])?;
            }
            done += count;
        }
        Ok(())
    }

    pub fn write_block(&mut self, block_index: u64, data: &[u8]) -> Result<(), CowError> {
        if self.poisoned {
            return Err(CowError::WritePoisoned);
        }
        if data.len() != self.scan.block_size as usize {
            return Err(CowError::Invalid(
                "write must contain exactly one full block".into(),
            ));
        }
        let block_count = self
            .scan
            .logical_size
            .checked_add(self.scan.block_size as u64 - 1)
            .ok_or_else(|| CowError::Invalid("logical block count overflow".into()))?
            / self.scan.block_size as u64;
        if block_index >= block_count {
            return Err(CowError::Invalid("COW block index out of range".into()));
        }

        if self.cow.is_none() {
            if let Some(parent) = self.cow_path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&self.cow_path)?;
            if let Err(error) = file
                .write_all(&make_header(
                    self.device_type,
                    self.scan.block_size,
                    self.scan.logical_size,
                    &self.base_hash,
                ))
                .and_then(|()| file.sync_data())
            {
                drop(file);
                if fs::remove_file(&self.cow_path).is_err() {
                    self.poisoned = true;
                }
                return Err(error.into());
            }
            self.cow = Some(file);
        }

        let start = self.committed_end;
        if self.scan.recoverable_tail {
            let cow = self.cow.as_mut().expect("existing overlay file");
            if let Err(error) = cow.set_len(start) {
                self.poisoned = true;
                return Err(error.into());
            }
            self.scan.recoverable_tail = false;
        }

        let append_result = (|| -> Result<(), CowError> {
            let mut header = [0u8; 32];
            header[..4].copy_from_slice(RECORD_MAGIC);
            header[4..12].copy_from_slice(&block_index.to_le_bytes());
            header[12..20].copy_from_slice(&self.next_sequence.to_le_bytes());
            header[20..24].copy_from_slice(&crc32c(data).to_le_bytes());
            let header_crc = crc32c(&header);
            header[24..28].copy_from_slice(&header_crc.to_le_bytes());
            let cow = self.cow.as_mut().expect("overlay file created above");
            cow.seek(SeekFrom::Start(start))?;
            cow.write_all(&header)?;
            cow.write_all(data)?;
            cow.sync_data()?;
            Ok(())
        })();
        if let Err(error) = append_result {
            let rollback = self
                .cow
                .as_mut()
                .expect("overlay file created above")
                .set_len(start)
                .and_then(|()| self.cow.as_mut().unwrap().sync_data());
            if rollback.is_err() {
                self.poisoned = true;
            }
            return Err(error);
        }

        let data_offset = start + RECORD_HEADER_SIZE;
        self.scan.latest_blocks.insert(block_index, data_offset);
        self.scan.records += 1;
        self.next_sequence += 1;
        self.committed_end = data_offset + data.len() as u64;
        self.scan.valid_end = self.committed_end;
        self.scan.recoverable_tail = false;
        Ok(())
    }
}

fn scan_cow_from_file(
    base: &mut BaseImage,
    cow: &mut File,
    device_type: u16,
    block_size: u32,
) -> Result<CowScan, CowError> {
    // Shared scanner logic without copying the COW: validate then build the same latest-record index.
    let (logical_size, base_hash) = validate_parameters(base, block_size)?;
    let file_len = cow.metadata()?.len();
    if file_len < HEADER_SIZE {
        return Err(CowError::Invalid("truncated COW header".into()));
    }
    cow.seek(SeekFrom::Start(0))?;
    let mut h = [0u8; 64];
    cow.read_exact(&mut h)?;
    if &h[..8] != HEADER_MAGIC
        || read_u16(&h, 8) != 1
        || read_u16(&h, 10) != 64
        || read_u16(&h, 12) != device_type
        || read_u16(&h, 14) != 0
        || read_u32(&h, 16) != block_size
        || read_u64(&h, 20) != logical_size
        || crc32c(&h[..60]) != read_u32(&h, 60)
    {
        return Err(CowError::Invalid("invalid or mismatched COW header".into()));
    }
    if h[28..60] != base_hash {
        return Err(CowError::BaseMismatch);
    }
    let blocks = logical_size
        .checked_add(block_size as u64 - 1)
        .ok_or_else(|| CowError::Invalid("logical block count overflow".into()))?
        / block_size as u64;
    let mut latest_blocks = HashMap::new();
    let mut offset = HEADER_SIZE;
    let mut seq = 1u64;
    let mut records = 0u64;
    let mut tail = false;
    while offset < file_len {
        let remain = file_len - offset;
        if remain < 32 {
            tail = true;
            break;
        }
        cow.seek(SeekFrom::Start(offset))?;
        let mut rh = [0u8; 32];
        cow.read_exact(&mut rh)?;
        let end = offset
            .checked_add(32 + block_size as u64)
            .ok_or_else(|| CowError::Invalid("record offset overflow".into()))?;
        if end > file_len {
            tail = true;
            break;
        }
        if &rh[..4] != RECORD_MAGIC || read_u32(&rh, 28) != 0 {
            return Err(CowError::Invalid("invalid COW record header".into()));
        }
        let hc = read_u32(&rh, 24);
        rh[24..28].fill(0);
        if crc32c(&rh) != hc {
            return Err(CowError::Invalid("invalid COW record header CRC32C".into()));
        }
        let index = read_u64(&rh, 4);
        if read_u64(&rh, 12) != seq {
            return Err(CowError::Invalid("invalid COW record sequence".into()));
        }
        if index >= blocks {
            return Err(CowError::Invalid("COW block index out of range".into()));
        }
        let expected = read_u32(&rh, 20);
        let data_at = offset + 32;
        cow.seek(SeekFrom::Start(data_at))?;
        let mut crc = 0u32;
        let mut left = block_size as usize;
        let mut buf = [0u8; 4096];
        while left > 0 {
            let n = left.min(buf.len());
            cow.read_exact(&mut buf[..n])?;
            crc = crc32c_append(crc, &buf[..n]);
            left -= n;
        }
        if crc != expected {
            return Err(CowError::Invalid("invalid COW block data CRC32C".into()));
        }
        latest_blocks.insert(index, data_at);
        records += 1;
        seq = seq
            .checked_add(1)
            .ok_or_else(|| CowError::Invalid("record sequence overflow".into()))?;
        offset = end;
    }
    Ok(CowScan {
        logical_size,
        block_size,
        device_type,
        records,
        recoverable_tail: tail,
        latest_blocks,
        valid_end: offset,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "cpt32-cow-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn base() -> BaseImage {
        BaseImage::Bytes(Arc::from(vec![0x11; 1024]))
    }
    #[test]
    fn fallback_repeat_and_reopen() {
        let p = path();
        let mut c = CowOverlay::open(base(), &p, 1, 512).unwrap();
        let mut out = [0; 512];
        c.read_at(0, &mut out).unwrap();
        assert_eq!(out, [0x11; 512]);
        c.write_block(0, &[0x22; 512]).unwrap();
        c.write_block(0, &[0x33; 512]).unwrap();
        drop(c);
        let mut c = CowOverlay::open(base(), &p, 1, 512).unwrap();
        c.read_at(0, &mut out).unwrap();
        assert_eq!(out, [0x33; 512]);
        drop(c);
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn recovers_incomplete_tail_and_truncates_before_append() {
        let p = path();
        let mut c = CowOverlay::open(base(), &p, 1, 512).unwrap();
        c.write_block(0, &[7; 512]).unwrap();
        drop(c);
        OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(b"BLK")
            .unwrap();
        let mut c = CowOverlay::open(base(), &p, 1, 512).unwrap();
        c.write_block(1, &[8; 512]).unwrap();
        assert!(!scan_cow(base(), &p, 1, 512).unwrap().recoverable_tail);
        drop(c);
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn rejects_crc_sequence_range_and_base_mismatch() {
        let p = path();
        let mut c = CowOverlay::open(base(), &p, 1, 512).unwrap();
        c.write_block(0, &[9; 512]).unwrap();
        drop(c);
        let mut bytes = std::fs::read(&p).unwrap();
        bytes[64 + 32] ^= 1;
        std::fs::write(&p, &bytes).unwrap();
        assert!(scan_cow(base(), &p, 1, 512).is_err());
        assert!(CowOverlay::open(BaseImage::Bytes(Arc::from(vec![2; 1024])), &p, 1, 512).is_err());
        std::fs::remove_file(p).unwrap();
        let p = path();
        let mut c = CowOverlay::open(base(), &p, 1, 512).unwrap();
        assert!(c.write_block(2, &[0; 512]).is_err());
        drop(c);
        std::fs::remove_file(p).unwrap();
    }
}
