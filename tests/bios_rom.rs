//! BIOS ROM loader: config parsing, window mapping, boot entry.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use cpt32::bus::Bus;
use cpt32::cpu::Cpu;
use cpt32::devices::bios::{BIOS_BASE, BIOS_SIZE, load_bios};
use cpt32::devices::ram::connect_ram;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_dir() -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cpt32-bios-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_config(dir: &PathBuf, body: &str) -> PathBuf {
    let path = dir.join("config.yaml");
    fs::write(&path, body).unwrap();
    path
}

fn finish(dir: &PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}

/// Minimal WAX v2 executable with one or more addressable payload blocks.
fn wax_blocks(base: u32, blocks: &[(u32, &[u8])]) -> Vec<u8> {
    let table_offset = 64usize;
    let mut file_size = table_offset + blocks.len() * 12;
    let mut records = Vec::new();
    for &(address, payload) in blocks {
        file_size = (file_size + 3) & !3;
        records.push((address, file_size, payload));
        file_size += payload.len();
    }
    let mut out = vec![0u8; file_size];
    out[..4].copy_from_slice(&cpt32::wax::MAGIC);
    out[4..6].copy_from_slice(&cpt32::wax::VERSION.to_be_bytes());
    out[6..8].copy_from_slice(&cpt32::wax::TYPE_WAX.to_be_bytes());
    out[8..10].copy_from_slice(&cpt32::wax::ARCH_SRC32.to_be_bytes());
    out[12..16].copy_from_slice(&64u32.to_be_bytes());
    out[16..20].copy_from_slice(&base.to_be_bytes());
    out[24..28].copy_from_slice(&(table_offset as u32).to_be_bytes());
    out[28..32].copy_from_slice(&(blocks.len() as u32).to_be_bytes());
    out[32..36].copy_from_slice(&12u32.to_be_bytes());
    out[44..48].copy_from_slice(&(file_size as u32).to_be_bytes());
    for (index, (address, file_offset, payload)) in records.into_iter().enumerate() {
        let record = table_offset + index * 12;
        out[record..record + 4].copy_from_slice(&address.to_be_bytes());
        out[record + 4..record + 8].copy_from_slice(&(file_offset as u32).to_be_bytes());
        out[record + 8..record + 12].copy_from_slice(&(payload.len() as u32).to_be_bytes());
        out[file_offset..file_offset + payload.len()].copy_from_slice(payload);
    }
    out
}

fn wax_bytes(base: u32, text: &[u8]) -> Vec<u8> {
    wax_blocks(base, &[(base, text)])
}

#[test]
fn missing_config_file_means_no_bios() {
    let dir = temp_dir();
    let mut bus = Bus::new();
    assert_eq!(load_bios(&dir.join("config.yaml"), &mut bus), Ok(None));
    finish(&dir);
}

#[test]
fn missing_or_null_bios_key_means_no_bios() {
    for body in ["pcg: {}\n", "bios:\n"] {
        let dir = temp_dir();
        let config = write_config(&dir, body);
        let mut bus = Bus::new();
        assert_eq!(load_bios(&config, &mut bus), Ok(None));
        finish(&dir);
    }
}

#[test]
fn raw_image_maps_read_only_and_zero_padded() {
    let dir = temp_dir();
    let image = [0xDE, 0xAD, 0xBE, 0xEF];
    fs::write(dir.join("bios.rom"), image).unwrap();
    let config = write_config(&dir, "bios:\n  image: \"bios.rom\"\n");
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    assert_eq!(load_bios(&config, &mut bus), Ok(Some(BIOS_BASE)));
    for (offset, byte) in image.iter().enumerate() {
        assert_eq!(bus.read_u8(BIOS_BASE + offset as u32), *byte);
    }
    assert_eq!(bus.read_u8(BIOS_BASE + image.len() as u32), 0);
    assert_eq!(bus.read_u8(BIOS_BASE + BIOS_SIZE as u32 - 1), 0);
    bus.write_u8(BIOS_BASE, 0x00);
    bus.write_u8(BIOS_BASE + BIOS_SIZE as u32 - 1, 0xFF);
    assert_eq!(bus.read_u8(BIOS_BASE), 0xDE);
    assert_eq!(bus.read_u8(BIOS_BASE + BIOS_SIZE as u32 - 1), 0);
    assert!(!bus.take_bus_error());
    let region = bus
        .memory_map()
        .find(|region| region.start == BIOS_BASE)
        .expect("BIOS window is mapped");
    assert_eq!(region.size, BIOS_SIZE as u32);
    assert_eq!(region.name, "BIOS ROM");
    finish(&dir);
}

#[test]
fn absolute_image_paths_resolve() {
    let dir = temp_dir();
    let image = [0x01, 0x02];
    let image_path = dir.join("fw.rom");
    fs::write(&image_path, image).unwrap();
    let config = write_config(
        &dir,
        &format!("bios:\n  image: '{}'\n", image_path.display()),
    );
    let mut bus = Bus::new();
    assert_eq!(load_bios(&config, &mut bus), Ok(Some(BIOS_BASE)));
    assert_eq!(bus.read_u8(BIOS_BASE + 1), 0x02);
    finish(&dir);
}

#[test]
fn invalid_configs_are_rejected() {
    let bodies = [
        "bios: []\n",
        "bios: {}\n",
        "bios:\n  image: \"\"\n",
        "bios:\n  image: \"missing.rom\"\n",
    ];
    for body in bodies {
        let dir = temp_dir();
        let config = write_config(&dir, body);
        let mut bus = Bus::new();
        assert!(load_bios(&config, &mut bus).is_err(), "{body}");
        finish(&dir);
    }
    let dir = temp_dir();
    fs::write(dir.join("empty.rom"), []).unwrap();
    fs::write(dir.join("big.rom"), vec![0xFF; BIOS_SIZE + 1]).unwrap();
    fs::write(
        dir.join("ram_base.wax"),
        wax_bytes(0, &[0xFC, 0x00, 0x00, 0x00]),
    )
    .unwrap();
    for image in ["empty.rom", "big.rom", "ram_base.wax"] {
        let config = write_config(&dir, &format!("bios:\n  image: \"{image}\"\n"));
        let mut bus = Bus::new();
        assert!(load_bios(&config, &mut bus).is_err(), "{image}");
    }
    finish(&dir);
}

#[test]
fn wax_inside_window_loads_at_offset_with_entry() {
    let dir = temp_dir();
    // Big-endian HALT word (opcode 0x3F).
    let base = BIOS_BASE + 0x100;
    fs::write(
        dir.join("bios.wax"),
        wax_bytes(base, &[0xFC, 0x00, 0x00, 0x00]),
    )
    .unwrap();
    let config = write_config(&dir, "bios:\n  image: \"bios.wax\"\n");
    let mut bus = Bus::new();
    assert_eq!(load_bios(&config, &mut bus), Ok(Some(base)));
    assert_eq!(bus.read_u8(base), 0xFC);
    assert_eq!(bus.read_u8(BIOS_BASE), 0);
    finish(&dir);
}

#[test]
fn sparse_wax_bios_blocks_map_at_their_addresses() {
    let dir = temp_dir();
    let base = BIOS_BASE + 0x100;
    let second = BIOS_BASE + 0x1000;
    fs::write(
        dir.join("sparse.wax"),
        wax_blocks(base, &[(base, &[0xFC, 0, 0, 0]), (second, &[0x12, 0x34])]),
    )
    .unwrap();
    let config = write_config(&dir, "bios:\n  image: \"sparse.wax\"\n");
    let mut bus = Bus::new();
    assert_eq!(load_bios(&config, &mut bus), Ok(Some(base)));
    assert_eq!(bus.read_u8(base), 0xFC);
    assert_eq!(bus.read_u8(base + 4), 0);
    assert_eq!(bus.read_u8(second), 0x12);
    assert_eq!(bus.read_u8(second + 1), 0x34);
    finish(&dir);
}

#[test]
fn cpu_boots_from_rom_entry() {
    let dir = temp_dir();
    fs::write(dir.join("bios.rom"), [0xFC, 0x00, 0x00, 0x00]).unwrap();
    let config = write_config(&dir, "bios:\n  image: \"bios.rom\"\n");
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let entry = load_bios(&config, &mut bus).expect("BIOS loads");
    let mut cpu = Cpu::new(bus);
    cpu.set_pc(entry.expect("boot entry"));
    cpu.run(64);
    assert!(!cpu.is_running());
    finish(&dir);
}
