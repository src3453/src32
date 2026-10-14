use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use cpt32::music::project::Song;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_directory() -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("cpt32-music-yaml-{}-{id}", std::process::id()));
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn music_project_save_and_load_use_yaml() {
    let dir = temporary_directory();
    let path = dir.join("song.yaml");
    let song = Song::default();

    song.save(&path).unwrap();
    let serialized = fs::read_to_string(&path).unwrap();
    assert!(
        serialized.starts_with("format_version: 1\n"),
        "{serialized}"
    );
    assert_eq!(Song::load(&path).unwrap(), song);

    fs::remove_dir_all(dir).unwrap();
}
