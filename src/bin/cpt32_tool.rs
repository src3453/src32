use cpt32::cow::{BaseImage, CowError, scan_cow_header};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyReport {
    pub records: u64,
    pub unique_blocks: u64,
    pub recoverable_tail: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BakeReport {
    pub blocks_applied: u64,
    pub bytes_written: u64,
}

fn base_file(path: &Path) -> Result<File, CowError> {
    Ok(File::open(path)?)
}

pub fn verify_cow(base_path: &Path, cow_path: &Path) -> Result<VerifyReport, CowError> {
    let scan = scan_cow_header(BaseImage::File(base_file(base_path)?), cow_path)?;
    Ok(VerifyReport {
        records: scan.records,
        unique_blocks: scan.latest_blocks.len() as u64,
        recoverable_tail: scan.recoverable_tail,
    })
}

fn normalized_output(path: &Path) -> Result<PathBuf, CowError> {
    let name = path
        .file_name()
        .ok_or_else(|| CowError::Invalid("output path has no filename".into()))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(fs::canonicalize(parent)?.join(name))
}

fn ensure_distinct_output(
    base_path: &Path,
    cow_path: &Path,
    output_path: &Path,
) -> Result<(), CowError> {
    if fs::symlink_metadata(output_path).is_ok() {
        return Err(CowError::Invalid("output already exists".into()));
    }
    let output = normalized_output(output_path)?;
    if output == fs::canonicalize(base_path)? || output == fs::canonicalize(cow_path)? {
        return Err(CowError::Invalid("output aliases an input".into()));
    }
    Ok(())
}

pub fn bake_cow(
    base_path: &Path,
    cow_path: &Path,
    output_path: &Path,
) -> Result<BakeReport, CowError> {
    ensure_distinct_output(base_path, cow_path, output_path)?;
    let scan = scan_cow_header(BaseImage::File(base_file(base_path)?), cow_path)?;
    // Recheck atomically when creating: only this invocation may clean up this output.
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)?;
    let result = (|| -> Result<BakeReport, CowError> {
        let mut base = File::open(base_path)?;
        let mut overlay = File::open(cow_path)?;
        let mut remaining = scan.logical_size;
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            let count = remaining.min(buffer.len() as u64) as usize;
            base.read_exact(&mut buffer[..count])?;
            output.write_all(&buffer[..count])?;
            remaining -= count as u64;
        }
        let mut block = vec![0u8; scan.block_size as usize];
        for (&index, &data_offset) in &scan.latest_blocks {
            overlay.seek(SeekFrom::Start(data_offset))?;
            overlay.read_exact(&mut block)?;
            let offset = index
                .checked_mul(scan.block_size as u64)
                .ok_or_else(|| CowError::Invalid("block offset overflow".into()))?;
            let count = (scan.logical_size - offset).min(scan.block_size as u64) as usize;
            output.seek(SeekFrom::Start(offset))?;
            output.write_all(&block[..count])?;
        }
        output.sync_all()?;
        let actual_size = output.metadata()?.len();
        if actual_size != scan.logical_size {
            return Err(CowError::Invalid("baked output size mismatch".into()));
        }
        Ok(BakeReport {
            blocks_applied: scan.latest_blocks.len() as u64,
            bytes_written: actual_size,
        })
    })();
    if result.is_err() {
        drop(output);
        let _ = fs::remove_file(output_path);
    }
    result
}

fn run(args: &[String]) -> Result<(), CowError> {
    match args {
        [_, group, command, base, cow] if group == "cow" && command == "verify" => {
            let report = verify_cow(Path::new(base), Path::new(cow))?;
            println!(
                "records: {}\nunique blocks: {}\nrecoverable tail: {}",
                report.records, report.unique_blocks, report.recoverable_tail
            );
            Ok(())
        }
        [_, group, command, base, cow, option, output]
            if group == "cow" && command == "bake" && option == "-o" =>
        {
            let report = bake_cow(Path::new(base), Path::new(cow), Path::new(output))?;
            println!(
                "blocks applied: {}\nbytes written: {}",
                report.blocks_applied, report.bytes_written
            );
            Ok(())
        }
        _ => Err(CowError::Invalid(
            "usage: cpt32-tool cow verify BASE COW | cpt32-tool cow bake BASE COW -o OUTPUT".into(),
        )),
    }
}

fn main() {
    cpt32::init_logging();

    let args: Vec<String> = std::env::args().collect();
    if let Err(error) = run(&args) {
        tracing::error!(%error, "COW tool failed");
        std::process::exit(2);
    }
}
