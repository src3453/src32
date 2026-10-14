use std::fs;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cpt32::bus::Bus;
use cpt32::cartridge::Cartridge;
use cpt32::cow::{BaseImage, CowOverlay};
use cpt32::devices::bmc::connect_bmc;
use cpt32::devices::dmac::dmac::{DMAC_BASE, connect_dmac};
use cpt32::devices::expansion::{
    EXPANSION_BASE, ExpansionCardPackage, load_expansion_cards_from_bytes,
};
use cpt32::devices::irqc::irqc::connect_irqc;
use cpt32::devices::pec::idc::idc::{IDC_BASE, IDC_DATA_ADDRESS, Idc, connect_idc};
use cpt32::devices::ram::connect_ram;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_path(extension: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "cpt32pkg-loader-{}-{id}.{extension}",
        std::process::id()
    ))
}

fn write_package(path: &PathBuf, manifest: &str, program: &[u8]) {
    let file = fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    zip.start_file("manifest.yaml", options).unwrap();
    zip.write_all(manifest.as_bytes()).unwrap();
    zip.start_file("program.bin", options).unwrap();
    zip.write_all(program).unwrap();
    zip.finish().unwrap();
}

const MANIFEST: &str = "format: cpt32pkg\nformat_version: 1\nid: 00000000-0000-4000-8000-000000000001\ntitle: Package smoke\nsystem:\n  architecture: SRC32\nentrypoint:\n  type: program\n  path: program.bin\n";

#[test]
fn package_loader_reads_program_from_zip() {
    let path = temporary_path("cpt32pkg");
    write_package(&path, MANIFEST, &[0x12, 0x34, 0x56]);

    let cartridge = Cartridge::load(&path).unwrap();

    assert_eq!(cartridge.id, "00000000-0000-4000-8000-000000000001");
    assert_eq!(cartridge.title, "Package smoke");
    assert_eq!(cartridge.program, [0x12, 0x34, 0x56]);
    assert!(cartridge.disks.is_empty());
    assert!(cartridge.plugins.is_empty());
    fs::remove_file(path).unwrap();
}

#[test]
fn package_loader_rejects_unsafe_referenced_paths() {
    let path = temporary_path("cpt32pkg");
    let manifest = MANIFEST.replace("program.bin", "../program.bin");
    write_package(&path, &manifest, &[0x12]);

    assert!(Cartridge::load(&path).is_err());
    fs::remove_file(path).unwrap();
}

fn temporary_directory() -> PathBuf {
    let path = temporary_path("dir");
    fs::create_dir_all(&path).unwrap();
    path
}

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    for (name, bytes) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn zip_with_duplicate_names(entries: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xEDB8_8320 & (0u32.wrapping_sub(crc & 1)));
            }
        }
        !crc
    }
    fn put_u16(output: &mut Vec<u8>, value: u16) {
        output.extend_from_slice(&value.to_le_bytes());
    }
    fn put_u32(output: &mut Vec<u8>, value: u32) {
        output.extend_from_slice(&value.to_le_bytes());
    }

    let mut output = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        let offset = u32::try_from(output.len()).unwrap();
        let size = u32::try_from(data.len()).unwrap();
        let checksum = crc32(data);
        put_u32(&mut output, 0x0403_4B50);
        put_u16(&mut output, 20);
        put_u16(&mut output, 0);
        put_u16(&mut output, 0);
        put_u16(&mut output, 0);
        put_u16(&mut output, 0);
        put_u32(&mut output, checksum);
        put_u32(&mut output, size);
        put_u32(&mut output, size);
        put_u16(&mut output, name_bytes.len() as u16);
        put_u16(&mut output, 0);
        output.extend_from_slice(name_bytes);
        output.extend_from_slice(data);

        put_u32(&mut central, 0x0201_4B50);
        put_u16(&mut central, 20);
        put_u16(&mut central, 20);
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u32(&mut central, checksum);
        put_u32(&mut central, size);
        put_u32(&mut central, size);
        put_u16(&mut central, name_bytes.len() as u16);
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u16(&mut central, 0);
        put_u32(&mut central, 0);
        put_u32(&mut central, offset);
        central.extend_from_slice(name_bytes);
    }
    let central_offset = u32::try_from(output.len()).unwrap();
    let central_size = u32::try_from(central.len()).unwrap();
    output.extend_from_slice(&central);
    put_u32(&mut output, 0x0605_4B50);
    put_u16(&mut output, 0);
    put_u16(&mut output, 0);
    put_u16(&mut output, entries.len() as u16);
    put_u16(&mut output, entries.len() as u16);
    put_u32(&mut output, central_size);
    put_u32(&mut output, central_offset);
    put_u16(&mut output, 0);
    output
}

fn load_fixture_package(manifest: &str, files: &[(&str, &[u8])]) -> Result<Cartridge, String> {
    let mut entries = Vec::with_capacity(files.len() + 1);
    entries.push(("manifest.yaml", manifest.as_bytes()));
    entries.extend_from_slice(files);
    let path = temporary_path("cpt32pkg");
    fs::write(&path, zip_bytes(&entries)).unwrap();
    let result = Cartridge::load(&path);
    fs::remove_file(path).unwrap();
    result
}

fn connect_test_idc(drives: [Option<cpt32::devices::pec::idc::idc::Drive>; 4]) -> Bus {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let bmc = connect_bmc(&mut bus);
    connect_dmac(&mut bus, bmc);
    connect_idc(&mut bus, drives);
    let irq = connect_irqc(&mut bus);
    bus.attach_irq_controller(irq);
    bus
}

fn configure_dma(bus: &mut Bus, read: bool, memory: u32) {
    bus.write_u32_be(DMAC_BASE + 0x008, 1);
    bus.write_u32_be(
        DMAC_BASE + 0x100,
        if read { IDC_DATA_ADDRESS } else { memory },
    );
    bus.write_u32_be(
        DMAC_BASE + 0x104,
        if read { memory } else { IDC_DATA_ADDRESS },
    );
    bus.write_u32_be(DMAC_BASE + 0x108, 512);
    let source_fixed = u32::from(read) << 2;
    let destination_fixed = u32::from(!read) << 4;
    bus.write_u32_be(
        DMAC_BASE + 0x10C,
        source_fixed | destination_fixed | (1 << 8),
    );
}

fn run_package_disk_transfer(bus: &mut Bus, read: bool, memory: u32) {
    bus.write_u32_be(IDC_BASE + 0x008, 2);
    bus.write_u32_be(IDC_BASE + 0x014, 0);
    bus.write_u32_be(IDC_BASE + 0x01C, 0);
    bus.write_u32_be(IDC_BASE + 0x020, 1);
    bus.write_u32_be(IDC_BASE + 0x018, if read { 1 } else { 2 });
    configure_dma(bus, read, memory);
    for _ in 0..500_000 {
        bus.tick_devices();
        if bus.read_u32_be(IDC_BASE + 0x008) & 2 != 0 {
            break;
        }
    }
    assert_ne!(bus.read_u32_be(IDC_BASE + 0x008) & 2, 0);
}

#[test]
fn package_disk_dma_writes_only_to_cow_and_reopens_with_changes() {
    const ID: &str = "00000000-0000-4000-8000-000000000001";
    let manifest = format!(
        "format: cpt32pkg\nformat_version: 1\nid: {ID}\ntitle: Disk smoke\nsystem:\n  architecture: SRC32\nentrypoint:\n  type: program\n  path: program.bin\ndisks:\n  - slot: 0\n    type: hdd\n    path: disk.img\n    mode: read_write\n"
    );
    let archive = zip_bytes(&[
        ("manifest.yaml", manifest.as_bytes()),
        ("program.bin", &[0x12, 0x34]),
        ("disk.img", &[0xA6; 512]),
    ]);
    let package_path = temporary_path("cpt32pkg");
    fs::write(&package_path, &archive).unwrap();
    let cartridge = Cartridge::load(&package_path).unwrap();
    let data_root = temporary_directory();
    let cartridge_root = data_root.join(&cartridge.id);

    let drives = Idc::load_cartridge_drives(&cartridge.disks, &cartridge_root).unwrap();
    assert!(!cartridge_root.join("dd0.cow").exists());
    let mut bus = connect_test_idc(drives);
    run_package_disk_transfer(&mut bus, true, 0x2000);
    for offset in 0..512 {
        assert_eq!(bus.read_u8(0x2000 + offset), 0xA6);
        bus.write_u8(0x4000 + offset, 0x35);
    }
    run_package_disk_transfer(&mut bus, false, 0x4000);
    drop(bus);

    assert_eq!(fs::read(&package_path).unwrap(), archive);
    assert!(cartridge_root.join("dd0.cow").is_file());
    let drives = Idc::load_cartridge_drives(&cartridge.disks, &cartridge_root).unwrap();
    let mut reopened = connect_test_idc(drives);
    run_package_disk_transfer(&mut reopened, true, 0x6000);
    for offset in 0..512 {
        assert_eq!(reopened.read_u8(0x6000 + offset), 0x35);
    }

    drop(reopened);
    fs::remove_file(package_path).unwrap();
    fs::remove_dir_all(data_root).unwrap();
}

#[test]
fn package_read_only_disk_rejects_write_without_sidecar() {
    let manifest = MANIFEST.replace(
        "path: program.bin\n",
        "path: program.bin\ndisks:\n  - slot: 0\n    type: fdd\n    path: disk.img\n    mode: read_only\n",
    );
    let package_path = temporary_path("cpt32pkg");
    fs::write(
        &package_path,
        zip_bytes(&[
            ("manifest.yaml", manifest.as_bytes()),
            ("program.bin", &[0x12]),
            ("disk.img", &[0xA6; 512]),
        ]),
    )
    .unwrap();
    let cartridge = Cartridge::load(&package_path).unwrap();
    let data_root = temporary_directory();
    let mut bus = connect_test_idc(
        Idc::load_cartridge_drives(&cartridge.disks, &data_root.join(&cartridge.id)).unwrap(),
    );
    bus.write_u32_be(IDC_BASE + 0x014, 0);
    bus.write_u32_be(IDC_BASE + 0x01C, 0);
    bus.write_u32_be(IDC_BASE + 0x020, 1);
    bus.write_u32_be(IDC_BASE + 0x018, 2);
    assert_eq!(bus.read_u32_be(IDC_BASE + 0x038), 4);
    assert!(!data_root.join(cartridge.id).join("dd0.cow").exists());
    drop(bus);
    fs::remove_file(package_path).unwrap();
    fs::remove_dir_all(data_root).unwrap();
}

fn valid_card_package() -> Vec<u8> {
    let wasm = wat::parse_str(
        r#"(module
          (func (export "cpt32_abi_version") (result i32) i32.const 1)
          (func (export "cpt32_reset"))
          (func (export "cpt32_mmio_read") (param i32) (result i32) i32.const 0)
          (func (export "cpt32_mmio_write") (param i32) (param i32))
          (func (export "cpt32_tick")))"#,
    )
    .unwrap();
    zip_bytes(&[
        ("manifest.yaml", b"abi_version: 1\nmodule: module.wasm\n"),
        ("module.wasm", &wasm),
    ])
}

#[test]
fn byte_loaded_cards_validate_all_before_installing_any() {
    let mut bus = Bus::new();
    let result = load_expansion_cards_from_bytes(
        vec![
            ExpansionCardPackage {
                slot: 0,
                label: "valid card".to_owned(),
                bytes: valid_card_package(),
            },
            ExpansionCardPackage {
                slot: 1,
                label: "invalid card".to_owned(),
                bytes: b"not a zip archive".to_vec(),
            },
        ],
        &mut bus,
    );
    assert!(result.is_err());
    assert!(
        !bus.memory_map()
            .any(|region| region.start == EXPANSION_BASE)
    );

    load_expansion_cards_from_bytes(
        vec![ExpansionCardPackage {
            slot: 0,
            label: "valid card".to_owned(),
            bytes: valid_card_package(),
        }],
        &mut bus,
    )
    .unwrap();
    assert!(
        bus.memory_map()
            .any(|region| region.start == EXPANSION_BASE)
    );
}

#[test]
fn package_loader_rejects_missing_fields_versions_resources_and_slot_errors() {
    assert!(load_fixture_package(
        "format: cpt32pkg\nformat_version: 1\nid: 00000000-0000-4000-8000-000000000001\ntitle: Test\nsystem:\n  architecture: SRC32\nentrypoint:\n  type: program\n  path: program.bin\nunknown: true\n",
        &[("program.bin", &[1])],
    )
    .is_err());
    assert!(
        load_fixture_package(
            &MANIFEST.replace("format_version: 1\n", ""),
            &[("program.bin", &[1])],
        )
        .is_err()
    );
    assert!(
        load_fixture_package(
            &MANIFEST.replace("format_version: 1", "format_version: 2"),
            &[("program.bin", &[1])],
        )
        .is_err()
    );
    assert!(
        load_fixture_package(
            &MANIFEST.replace("path: program.bin", "path: missing.bin"),
            &[("program.bin", &[1])],
        )
        .is_err()
    );

    let duplicate_role = format!(
        "{MANIFEST}disks:\n  - slot: 0\n    type: hdd\n    path: program.bin\n    mode: read_only\n"
    );
    assert!(load_fixture_package(&duplicate_role, &[("program.bin", &[1])]).is_err());

    let duplicate_disk_slots = format!(
        "{MANIFEST}disks:\n  - slot: 0\n    type: hdd\n    path: disk.img\n    mode: read_only\n  - slot: 0\n    type: fdd\n    path: disk2.img\n    mode: read_only\n"
    );
    assert!(
        load_fixture_package(
            &duplicate_disk_slots,
            &[
                ("program.bin", &[1]),
                ("disk.img", &[0; 512]),
                ("disk2.img", &[0; 512])
            ],
        )
        .is_err()
    );

    let invalid_mode = format!(
        "{MANIFEST}disks:\n  - slot: 0\n    type: hdd\n    path: disk.img\n    mode: writable\n"
    );
    assert!(
        load_fixture_package(
            &invalid_mode,
            &[("program.bin", &[1]), ("disk.img", &[0; 512])],
        )
        .is_err()
    );
    let missing_mode = invalid_mode.replace("    mode: writable\n", "");
    assert!(
        load_fixture_package(
            &missing_mode,
            &[("program.bin", &[1]), ("disk.img", &[0; 512])],
        )
        .is_err()
    );

    let duplicate_plugin_slots = format!(
        "{MANIFEST}plugins:\n  - slot: 1\n    path: first.cpt32card\n  - slot: 1\n    path: second.cpt32card\n"
    );
    assert!(
        load_fixture_package(
            &duplicate_plugin_slots,
            &[
                ("program.bin", &[1]),
                ("first.cpt32card", &[1]),
                ("second.cpt32card", &[2])
            ],
        )
        .is_err()
    );
}

#[test]
fn package_loader_rejects_duplicate_names_unsafe_assets_and_oversized_members() {
    let duplicate = zip_with_duplicate_names(&[
        ("manifest.yaml", MANIFEST.as_bytes()),
        ("program.bin", &[1]),
        ("program.bin", &[2]),
    ]);
    let duplicate_path = temporary_path("cpt32pkg");
    fs::write(&duplicate_path, duplicate).unwrap();
    assert!(Cartridge::load(&duplicate_path).is_err());
    fs::remove_file(duplicate_path).unwrap();
    assert!(
        load_fixture_package(MANIFEST, &[("program.bin", &[1]), ("../escape", &[2])],).is_err()
    );
    assert!(
        load_fixture_package(
            MANIFEST,
            &[("program.bin", &[1]), ("folder/", &[]), ("folder", &[])],
        )
        .is_err()
    );

    let oversized = vec![0u8; 64 * 1024 * 1024 + 1];
    assert!(
        load_fixture_package(
            MANIFEST,
            &[("program.bin", &[1]), ("unused.bin", &oversized)],
        )
        .is_err()
    );
}

fn writable_cartridge(image: &[u8]) -> (PathBuf, Vec<u8>, Cartridge) {
    const ID: &str = "00000000-0000-4000-8000-000000000001";
    let manifest = format!(
        "format: cpt32pkg\nformat_version: 1\nid: {ID}\ntitle: COW validation\nsystem:\n  architecture: SRC32\nentrypoint:\n  type: program\n  path: program.bin\ndisks:\n  - slot: 0\n    type: hdd\n    path: disk.img\n    mode: read_write\n"
    );
    let bytes = zip_bytes(&[
        ("manifest.yaml", manifest.as_bytes()),
        ("program.bin", &[0x12]),
        ("disk.img", image),
    ]);
    let path = temporary_path("cpt32pkg");
    fs::write(&path, &bytes).unwrap();
    let cartridge = Cartridge::load(&path).unwrap();
    (path, bytes, cartridge)
}

fn create_package_sidecar(root: &PathBuf, cartridge: &Cartridge, image: &[u8]) -> PathBuf {
    let cartridge_root = root.join(&cartridge.id);
    let cow_path = cartridge_root.join("dd0.cow");
    let mut cow = CowOverlay::open(
        BaseImage::Bytes(Arc::from(image.to_vec())),
        &cow_path,
        1,
        512,
    )
    .unwrap();
    cow.write_block(0, &[0x5A; 512]).unwrap();
    cow_path
}

fn recalculate_record_header_crc(sidecar: &mut [u8]) {
    sidecar[88..92].fill(0);
    let checksum = crc32c::crc32c(&sidecar[64..96]);
    sidecar[88..92].copy_from_slice(&checksum.to_le_bytes());
}

#[test]
fn package_startup_rejects_corrupt_cow_without_mutating_package_or_sidecar() {
    let base = [0xA6; 512];
    for corruption in ["header", "record_header", "block_data", "sequence", "range"] {
        let (package_path, package_bytes, cartridge) = writable_cartridge(&base);
        let root = temporary_directory();
        let cow_path = create_package_sidecar(&root, &cartridge, &base);
        let mut sidecar = fs::read(&cow_path).unwrap();
        match corruption {
            "header" => sidecar[60] ^= 1,
            "record_header" => sidecar[88] ^= 1,
            "block_data" => sidecar[96] ^= 1,
            "sequence" => {
                sidecar[76..84].copy_from_slice(&2u64.to_le_bytes());
                recalculate_record_header_crc(&mut sidecar);
            }
            "range" => {
                sidecar[68..76].copy_from_slice(&2u64.to_le_bytes());
                recalculate_record_header_crc(&mut sidecar);
            }
            _ => unreachable!(),
        }
        fs::write(&cow_path, &sidecar).unwrap();
        let unchanged_sidecar = fs::read(&cow_path).unwrap();

        assert!(Idc::load_cartridge_drives(&cartridge.disks, &root.join(&cartridge.id)).is_err());
        assert_eq!(fs::read(&package_path).unwrap(), package_bytes);
        assert_eq!(fs::read(&cow_path).unwrap(), unchanged_sidecar);

        fs::remove_file(package_path).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn package_startup_rejects_cow_base_hash_mismatch_without_mutation() {
    let original_image = [0xA6; 512];
    let changed_image = [0xA7; 512];
    let (original_path, original_archive, original) = writable_cartridge(&original_image);
    let (updated_path, updated_archive, updated) = writable_cartridge(&changed_image);
    let root = temporary_directory();
    let cow_path = create_package_sidecar(&root, &original, &original_image);
    let unchanged_sidecar = fs::read(&cow_path).unwrap();

    assert!(Idc::load_cartridge_drives(&updated.disks, &root.join(&updated.id)).is_err());
    assert_eq!(fs::read(&cow_path).unwrap(), unchanged_sidecar);
    assert_eq!(fs::read(&original_path).unwrap(), original_archive);
    assert_eq!(fs::read(&updated_path).unwrap(), updated_archive);

    fs::remove_file(original_path).unwrap();
    fs::remove_file(updated_path).unwrap();
    fs::remove_dir_all(root).unwrap();
}
