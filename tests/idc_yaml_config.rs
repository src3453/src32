use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use cpt32::devices::pec::idc::idc::Idc;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_directory() -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("cpt32-idc-yaml-{}-{id}", std::process::id()));
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn yaml_disk_config_loads_a_relative_image_into_its_slot() {
    let dir = temporary_directory();
    fs::write(dir.join("disk.img"), [0u8; 512]).unwrap();
    let config = dir.join("config.yaml");
    fs::write(
        &config,
        "disk:\n  - slot: 2\n    type: hdd\n    image: disk.img\n    read_only: true\n    transfer_rate_bytes_per_second: 8000000\n    access_latency_ms: 4\n",
    )
    .unwrap();

    let drives = Idc::load_drives(&config).unwrap();
    assert!(drives[2].is_some());
    assert!(drives[..2].iter().all(Option::is_none));
    assert!(drives[3].is_none());

    drop(drives);
    fs::remove_dir_all(dir).unwrap();
}
