use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cpt32::cow::{BaseImage, CowOverlay, scan_cow};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_path() -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("cpt32-cow-smoke-{}-{id}.cow", std::process::id()))
}

fn base() -> Arc<[u8]> {
    Arc::from(vec![0x11; 1024])
}

#[test]
fn overlay_persists_latest_blocks_and_reports_recoverable_tail() {
    let cow_path = temporary_path();
    let base_bytes = base();
    let mut overlay =
        CowOverlay::open(BaseImage::Bytes(Arc::clone(&base_bytes)), &cow_path, 1, 512).unwrap();
    assert!(!cow_path.exists());
    let mut first = [0; 512];
    overlay.read_at(0, &mut first).unwrap();
    assert_eq!(first, [0x11; 512]);
    overlay.write_block(0, &[0x22; 512]).unwrap();
    assert!(cow_path.exists());
    overlay.write_block(0, &[0x33; 512]).unwrap();
    drop(overlay);

    OpenOptions::new()
        .append(true)
        .open(&cow_path)
        .unwrap()
        .write_all(b"BLK")
        .unwrap();
    let report = scan_cow(BaseImage::Bytes(Arc::clone(&base_bytes)), &cow_path, 1, 512).unwrap();
    assert_eq!(report.records, 2);
    assert_eq!(report.latest_blocks.len(), 1);
    assert!(report.recoverable_tail);

    let mut overlay = CowOverlay::open(BaseImage::Bytes(base_bytes), &cow_path, 1, 512).unwrap();
    let mut latest = [0; 512];
    overlay.read_at(0, &mut latest).unwrap();
    assert_eq!(latest, [0x33; 512]);
    overlay.write_block(1, &[0x44; 512]).unwrap();
    assert!(
        !scan_cow(BaseImage::Bytes(base()), &cow_path, 1, 512)
            .unwrap()
            .recoverable_tail
    );
    drop(overlay);
    fs::remove_file(cow_path).unwrap();
}

#[test]
fn overlay_rejects_base_mismatch_and_out_of_range_reads() {
    let cow_path = temporary_path();
    let mut overlay = CowOverlay::open(BaseImage::Bytes(base()), &cow_path, 1, 512).unwrap();
    assert!(overlay.read_at(1024, &mut [0]).is_err());
    overlay.write_block(0, &[0x77; 512]).unwrap();
    drop(overlay);
    let changed_base = Arc::from(vec![0x12; 1024]);
    assert!(CowOverlay::open(BaseImage::Bytes(changed_base), &cow_path, 1, 512).is_err());
    fs::remove_file(cow_path).unwrap();
}
