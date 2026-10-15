use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use cpt32::devices::pec::idc::audio::IdcAudio;
use cpt32::devices::pec::idc::idc::{Idc, IdcSoundConfig};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_directory(prefix: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("cpt32-{prefix}-{}-{id}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn drives_err(config: &Path) -> String {
    // `Drive` owns open files and has no `Debug` impl, so `unwrap_err`
    // (which requires `T: Debug`) cannot be used here.
    Idc::load_drives(config)
        .err()
        .expect("expected a drive configuration error")
}

fn wav_bytes_mono(samples: &[i16], rate: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    let data_len = samples.len() * 2;
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// Minimal 86Box-shaped asset pack: `sounds/hdd/hdd_audio_profiles.cfg`
/// plus the WAV files it references via `assets/sounds/...` paths.
fn write_hdd_pack(root: &Path, internal_name: &str, dir_name: &str) {
    let drive_dir = root.join("sounds").join("hdd").join(dir_name);
    fs::create_dir_all(&drive_dir).unwrap();
    // Something clearly audible after normalization: alternating extremes.
    let loud: Vec<i16> = (0..480)
        .map(|i| if i % 2 == 0 { 12000 } else { -12000 })
        .collect();
    fs::write(drive_dir.join("spin.wav"), wav_bytes_mono(&loud, 48_000)).unwrap();
    fs::write(drive_dir.join("seek.wav"), wav_bytes_mono(&loud, 48_000)).unwrap();
    let cfg = format!(
        "[Profile \"1\"]\nid = 1\nname = Test Drive\ninternal_name = {internal_name}\nrpm = 3600\n\
         spindlemotor_loop_file = assets/sounds/hdd/{dir_name}/spin.wav\n\
         spindlemotor_loop_volume = 0.5\n\
         seek_track_file = assets/sounds/hdd/{dir_name}/seek.wav\n\
         seek_track_volume = 1.0\n"
    );
    fs::write(
        root.join("sounds")
            .join("hdd")
            .join("hdd_audio_profiles.cfg"),
        cfg,
    )
    .unwrap();
}

#[test]
fn sound_config_parses_per_disk_profiles_and_defaults() {
    let dir = temporary_directory("idc-sound-cfg");
    fs::write(dir.join("disk.img"), [0u8; 512]).unwrap();
    let config = dir.join("config.yaml");
    fs::write(
        &config,
        "disk_sound:\n  asset_root: \"pack\"\n  volume: 0.5\n  default_hdd_profile: hdd_default\n\
         disk:\n  - slot: 0\n    type: \"hdd\"\n    image: \"disk.img\"\n    sound_profile: \"custom_hdd\"\n",
    )
    .unwrap();

    let parsed = Idc::load_sound_config(&config).unwrap();
    assert_eq!(parsed.asset_roots[0], dir.join("pack"));
    assert_eq!(parsed.volume, 0.5);
    assert_eq!(parsed.slots[0].as_deref(), Some("custom_hdd"));
    assert!(parsed.slots[1].is_none());
    assert_eq!(parsed.default_hdd_profile.as_deref(), Some("hdd_default"));
    assert!(parsed.default_fdd_profile.is_none());

    // Drives still load with the new key present.
    let drives = Idc::load_drives(&config).unwrap();
    assert!(drives[0].is_some());
    drop(drives);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sound_config_rejects_bad_values_but_never_breaks_drive_loading() {
    let dir = temporary_directory("idc-sound-bad");
    fs::write(dir.join("disk.img"), [0u8; 512]).unwrap();

    let bad_volume = dir.join("bad-volume.yaml");
    fs::write(
        &bad_volume,
        "disk_sound:\n  volume: 9.0\ndisk:\n  - slot: 0\n    type: \"hdd\"\n    image: \"disk.img\"\n",
    )
    .unwrap();
    assert!(
        Idc::load_sound_config(&bad_volume)
            .unwrap_err()
            .contains("volume")
    );
    // Drive loading itself is unaffected by sound settings.
    assert!(Idc::load_drives(&bad_volume).unwrap()[0].is_some());

    let bad_key = dir.join("bad-key.yaml");
    fs::write(
        &bad_key,
        "disk_sound:\n  bogus: 1\ndisk:\n  - slot: 0\n    type: \"hdd\"\n    image: \"disk.img\"\n",
    )
    .unwrap();
    assert!(
        Idc::load_sound_config(&bad_key)
            .unwrap_err()
            .contains("unknown `disk_sound` key")
    );

    let bad_disk_key = dir.join("bad-disk-key.yaml");
    fs::write(
        &bad_disk_key,
        "disk:\n  - slot: 0\n    type: \"hdd\"\n    image: \"disk.img\"\n    no_such_key: 1\n",
    )
    .unwrap();
    assert!(drives_err(&bad_disk_key).contains("unknown configuration key"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_pack_stays_silent_and_boots() {
    let dir = temporary_directory("idc-sound-missing");
    fs::write(dir.join("disk.img"), [0u8; 512]).unwrap();
    let config = dir.join("config.yaml");
    fs::write(
        &config,
        "disk:\n  - slot: 0\n    type: \"hdd\"\n    image: \"disk.img\"\n    sound_profile: \"anything\"\n",
    )
    .unwrap();

    let drives = Idc::load_drives(&config).unwrap();
    let mut idc = Idc::new(drives);
    let sound = Idc::load_sound_config(&config).unwrap();
    idc.apply_sound_config(&sound);
    assert!(idc.sound_status().iter().all(Option::is_none));
    let mut left = vec![0i16; 480];
    let mut right = vec![0i16; 480];
    idc.render_audio(48_000, &mut left, &mut right);
    assert!(left.iter().all(|&s| s == 0));
    assert!(right.iter().all(|&s| s == 0));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn engine_plays_spindle_and_seek_from_user_pack() {
    let dir = temporary_directory("idc-sound-pack");
    write_hdd_pack(&dir, "test_drive", "TestDrive");

    let mut audio = IdcAudio::new();
    assert_eq!(audio.load_from_asset_root(&dir), 1);
    assert_eq!(audio.find_profile("test_drive"), Some(0));
    assert_eq!(audio.find_profile("nope"), None);

    audio.set_slot_profile(0, Some(0));
    // Spindle loop alone is already audible.
    let (left, _) = audio.render(48_000, 480);
    assert!(left.iter().any(|&s| s != 0));

    // A transfer start layers a seek click on top of the loop.
    audio.notify_transfer_start(0, 100, 4, 0);
    let (left, _) = audio.render(48_000, 480);
    assert!(left.iter().any(|&s| s != 0));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn idc_assigns_per_disk_profile_and_mixes_audio() {
    let dir = temporary_directory("idc-sound-e2e");
    write_hdd_pack(&dir, "e2e_drive", "E2E");
    fs::write(dir.join("disk.img"), [0u8; 512]).unwrap();
    let config = dir.join("config.yaml");
    fs::write(
        &config,
        "disk_sound:\n  asset_root: \".\"\n\
         disk:\n  - slot: 1\n    type: \"hdd\"\n    image: \"disk.img\"\n    sound_profile: \"e2e_drive\"\n",
    )
    .unwrap();

    let drives = Idc::load_drives(&config).unwrap();
    let mut idc = Idc::new(drives);
    let sound = Idc::load_sound_config(&config).unwrap();
    idc.apply_sound_config(&sound);

    let status = idc.sound_status();
    assert!(status[0].is_none());
    assert_eq!(
        status[1].clone().map(|(internal, _)| internal).as_deref(),
        Some("e2e_drive")
    );

    let mut left = vec![0i16; 480];
    let mut right = vec![0i16; 480];
    idc.render_audio(48_000, &mut left, &mut right);
    assert!(left.iter().any(|&s| s != 0));
    assert!(right.iter().any(|&s| s != 0));

    // Unknown profile names warn and stay muted instead of failing.
    let mut unknown = IdcSoundConfig::muted();
    unknown.asset_roots = vec![dir.clone()];
    unknown.slots[1] = Some("ghost_drive".to_string());
    idc.apply_sound_config(&unknown);
    assert!(idc.sound_status()[1].is_none());
    fs::remove_dir_all(dir).unwrap();
}
