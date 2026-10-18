use cpt32::bus::{Bus, BusAccessSource};
use cpt32::cpu::Cpu;
use cpt32::devices::bmc::connect_bmc;
use cpt32::devices::dmac::dmac::{DMAC_BASE, connect_dmac};
use cpt32::devices::expansion::{EXPANSION_BASE, EXPANSION_SLOT_SIZE, load_expansion_cards};
use cpt32::devices::irqc::irqc::{IRQC_BASE, connect_irqc};
use cpt32::devices::ram::connect_ram;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

const CARD: &str = r#"
(module
  (import "cpt32_host" "schedule_tick" (func $schedule (param i64)))
  (import "cpt32_host" "irq_set" (func $irq (param i32)))
  (global $count (mut i32) (i32.const 0))
  (func $start
    i64.const 5
    call $schedule)
  (start $start)
  (func (export "cpt32_abi_version") (result i32) i32.const 1)
  (func (export "cpt32_reset")
    i32.const 0
    global.set $count
    i64.const 0
    call $schedule
    i32.const 0
    call $irq)
  (func (export "cpt32_mmio_read") (param i32) (result i32)
    global.get $count)
  (func (export "cpt32_mmio_write") (param i32) (param i32)
    local.get 1
    global.set $count
    local.get 1
    i32.eqz
    if
      i32.const 0
      call $irq
    end)
  (func (export "cpt32_tick")
    global.get $count
    i32.const 1
    i32.add
    global.set $count
    i64.const 5
    call $schedule
    i32.const 1
    call $irq))
"#;

#[test]
fn slot_zero_mmio_and_deadlines_are_driven_by_virtual_cycles() {
    let dir = temporary_directory();
    let mut bus = Bus::new();
    let irq = connect_irqc(&mut bus);
    bus.attach_irq_controller(Rc::clone(&irq));
    install_package(&mut bus, &dir, 0, CARD, &[]);

    let region = bus
        .memory_map()
        .find(|region| region.start == EXPANSION_BASE)
        .unwrap();
    assert_eq!(region.size, EXPANSION_SLOT_SIZE);
    assert_eq!(bus.read_u8(EXPANSION_BASE), 0);
    assert_eq!(bus.cycles_until_next_event(), Some(5));

    bus.advance_system_cycles(4);
    assert_eq!(bus.system_cycles(), 4);
    assert_eq!(irq.borrow().pending() & (1 << 5), 0);
    assert_eq!(bus.read_u8(EXPANSION_BASE), 0);

    bus.advance_system_cycles(1);
    assert_eq!(bus.system_cycles(), 5);
    assert_eq!(bus.read_u8(EXPANSION_BASE), 1);
    assert_ne!(irq.borrow().pending() & (1 << 5), 0);
    assert_eq!(bus.cycles_until_next_event(), Some(5));

    bus.advance_system_cycles(5);
    assert_eq!(bus.system_cycles(), 10);
    assert_eq!(bus.read_u8(EXPANSION_BASE), 2);
    bus.write_u8(EXPANSION_BASE, 0);
    irq.borrow_mut().clear_pending(1 << 5);
    assert_eq!(irq.borrow().pending() & (1 << 5), 0);

    bus.reset_expansion_cards();
    assert_eq!(bus.system_cycles(), 10);
    assert_eq!(bus.read_u8(EXPANSION_BASE), 0);
    assert_eq!(bus.cycles_until_next_event(), None);

    let skip_dir = temporary_directory();
    let mut skip_bus = Bus::new();
    let skip_irq = connect_irqc(&mut skip_bus);
    skip_bus.attach_irq_controller(Rc::clone(&skip_irq));
    install_package(&mut skip_bus, &skip_dir, 0, CARD, &[]);
    skip_bus.advance_system_cycles(12);
    assert_eq!(skip_bus.system_cycles(), 12);
    assert_eq!(skip_bus.read_u8(EXPANSION_BASE), 2);
    assert_eq!(skip_bus.cycles_until_next_event(), Some(3));
    assert_ne!(skip_irq.borrow().pending() & (1 << 5), 0);
    fs::remove_dir_all(skip_dir).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dmac_transfers_bytes_to_and_from_a_card_aperture() {
    let dir = temporary_directory();
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let bmc = connect_bmc(&mut bus);
    connect_dmac(&mut bus, bmc);
    install_package(&mut bus, &dir, 0, CARD, &[]);

    bus.write_u8(0x100, 0xA5);
    bus.write_u32_be(DMAC_BASE + 8, 1);
    bus.write_u32_be(DMAC_BASE + 0x100, 0x100);
    bus.write_u32_be(DMAC_BASE + 0x104, EXPANSION_BASE + 1);
    bus.write_u32_be(DMAC_BASE + 0x108, 1);
    bus.write_u32_be(DMAC_BASE + 0x10C, (1 << 4) | (1 << 8));
    bus.tick_devices();
    bus.tick_devices();
    assert_eq!(bus.read_u8(EXPANSION_BASE + 1), 0xA5);
    assert_eq!(bus.read_u32_be(DMAC_BASE + 0x110) & 2, 2);

    bus.write_u32_be(DMAC_BASE + 0x100, EXPANSION_BASE + 1);
    bus.write_u32_be(DMAC_BASE + 0x104, 0x200);
    bus.write_u32_be(DMAC_BASE + 0x108, 1);
    bus.write_u32_be(DMAC_BASE + 0x10C, (1 << 2) | (1 << 8));
    bus.tick_devices();
    bus.tick_devices();
    assert_eq!(bus.read_u8(0x200), 0xA5);
    assert_eq!(bus.read_u32_be(DMAC_BASE + 0x110) & 2, 2);

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn faulted_cards_raise_cpu_bus_error_and_reject_dma() {
    const FAULTING_CARD: &str = r#"
    (module
      (func (export "cpt32_abi_version") (result i32) i32.const 1)
      (func (export "cpt32_reset"))
      (func (export "cpt32_mmio_read") (param i32) (result i32) i32.const 300)
      (func (export "cpt32_mmio_write") (param i32) (param i32))
      (func (export "cpt32_tick")))
    "#;

    let dir = temporary_directory();
    let mut bus = Bus::new();
    install_package(&mut bus, &dir, 7, FAULTING_CARD, &[]);
    assert_eq!(bus.read_u8(0xAE00_0000), 0);
    assert!(bus.take_bus_error());

    let mut dma_bus = Bus::new();
    connect_ram(&mut dma_bus);
    let bmc = connect_bmc(&mut dma_bus);
    connect_dmac(&mut dma_bus, bmc);
    install_package(&mut dma_bus, &dir, 0, FAULTING_CARD, &[]);
    assert_eq!(dma_bus.read_u8(EXPANSION_BASE), 0);
    assert!(dma_bus.take_bus_error());
    dma_bus.write_u32_be(DMAC_BASE + 8, 1);
    dma_bus.write_u32_be(DMAC_BASE + 0x100, 0x100);
    dma_bus.write_u32_be(DMAC_BASE + 0x104, EXPANSION_BASE);
    dma_bus.write_u32_be(DMAC_BASE + 0x108, 1);
    dma_bus.write_u32_be(DMAC_BASE + 0x10C, 1 << 8);
    dma_bus.tick_devices();
    dma_bus.tick_devices();
    let status = dma_bus.read_u32_be(DMAC_BASE + 0x110);
    assert_ne!(status & 4, 0);
    assert_eq!((status >> 4) & 0xF, 2);

    fs::remove_dir_all(dir).unwrap();
}

const ASSET_CARD: &str = r#"
(module
  (import "cpt32_host" "asset_count" (func $count (result i32)))
  (import "cpt32_host" "asset_size" (func $size (param i32) (result i32)))
  (import "cpt32_host" "asset_read_byte" (func $read (param i32 i32) (result i32)))
  (func (export "cpt32_abi_version") (result i32) i32.const 1)
  (func (export "cpt32_reset"))
  (func (export "cpt32_mmio_read") (param i32) (result i32)
    call $count
    i32.const 1
    i32.ne
    if (result i32)
      i32.const 0
    else
      i32.const 0
      call $size
      i32.const 1
      i32.ne
      if (result i32)
        i32.const 0
      else
        i32.const 0
        i32.const 0
        call $read
      end
    end)
  (func (export "cpt32_mmio_write") (param i32) (param i32))
  (func (export "cpt32_tick")))
"#;

fn temporary_directory() -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("cpt32-card-{}-{id}", std::process::id()));
    fs::create_dir(&path).unwrap();
    path
}

fn write_card_package(path: &Path, module: &str, assets: &[(&str, &[u8])]) {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let wasm = wat::parse_str(module).unwrap();
    let asset_entries = assets
        .iter()
        .enumerate()
        .map(|(id, (path, _))| format!("  - id: {id}\n    path: \"{path}\"\n"))
        .collect::<String>();
    let manifest = format!(
        "abi_version: 1\nmodule: card.wasm\n{}{}",
        if assets.is_empty() { "" } else { "asset:\n" },
        asset_entries
    );
    archive.start_file("manifest.yaml", options).unwrap();
    archive.write_all(manifest.as_bytes()).unwrap();
    archive.start_file("card.wasm", options).unwrap();
    archive.write_all(&wasm).unwrap();
    for (entry, bytes) in assets {
        archive.start_file(*entry, options).unwrap();
        archive.write_all(bytes).unwrap();
    }
    fs::write(path, archive.finish().unwrap().into_inner()).unwrap();
}

fn install_package(bus: &mut Bus, dir: &Path, slot: u8, module: &str, assets: &[(&str, &[u8])]) {
    let package = format!("slot-{slot}.cpt32card");
    write_card_package(&dir.join(&package), module, assets);
    let config_path = dir.join(format!("slot-{slot}.yaml"));
    fs::write(
        &config_path,
        format!("expansion_card:\n  - slot: {slot}\n    package: \"{package}\"\n"),
    )
    .unwrap();
    load_expansion_cards(&config_path, bus).unwrap();
}

#[test]
fn package_assets_load_lazily_and_config_paths_are_relative() {
    let dir = temporary_directory();
    let package_path = dir.join("card.cpt32card");
    write_card_package(&package_path, ASSET_CARD, &[("assets/font.bin", &[0x5A])]);
    let config_path = dir.join("config.yaml");
    fs::write(
        &config_path,
        "expansion_card:\n  - slot: 7\n    package: \"card.cpt32card\"\n",
    )
    .unwrap();

    let mut bus = Bus::new();
    load_expansion_cards(&config_path, &mut bus).unwrap();
    let region = bus
        .memory_map()
        .find(|region| region.start == 0xAE00_0000)
        .unwrap();
    assert_eq!(region.size, EXPANSION_SLOT_SIZE);
    assert_eq!(bus.read_u8(0xAE00_0000), 0x5A);
    fs::remove_dir_all(dir).unwrap();
}

const SIMPLE_CARD: &str = r#"
(module
  (func (export "cpt32_abi_version") (result i32) i32.const 1)
  (func (export "cpt32_reset"))
  (func (export "cpt32_mmio_read") (param i32) (result i32) i32.const 90)
  (func (export "cpt32_mmio_write") (param i32) (param i32))
  (func (export "cpt32_tick")))
"#;

#[test]
fn failed_package_load_does_not_register_earlier_slots() {
    let dir = temporary_directory();
    write_card_package(&dir.join("valid.cpt32card"), SIMPLE_CARD, &[]);
    fs::write(dir.join("broken.cpt32card"), b"not a ZIP archive").unwrap();
    let config = dir.join("config.yaml");
    fs::write(
        &config,
        "expansion_card:\n  - slot: 0\n    package: \"valid.cpt32card\"\n  - slot: 1\n    package: \"broken.cpt32card\"\n",
    )
    .unwrap();

    let mut bus = Bus::new();
    let error = load_expansion_cards(&config, &mut bus).unwrap_err();
    assert!(error.contains("slot 1"));
    assert!(error.contains("broken.cpt32card"));
    assert!(
        !bus.memory_map()
            .any(|region| (EXPANSION_BASE..0xB000_0000).contains(&region.start))
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn monitor_entrypoint_loads_card_package_from_config() {
    let dir = temporary_directory();
    write_card_package(
        &dir.join("card.cpt32card"),
        ASSET_CARD,
        &[("assets/font.bin", &[0x5A])],
    );
    fs::write(
        dir.join("config.yaml"),
        "expansion_card:\n  - slot: 0\n    package: \"card.cpt32card\"\n",
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_cpt32"))
        .args(["--monitor"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"mem 0xA0000000 1\nquit\n")
        .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("0xA0000000: 5A"), "{stdout}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn headless_testbench_entrypoint_loads_card_package_from_config() {
    let dir = temporary_directory();
    write_card_package(&dir.join("card.cpt32card"), SIMPLE_CARD, &[]);
    fs::write(
        dir.join("config.yaml"),
        "expansion_card:\n  - slot: 0\n    package: \"card.cpt32card\"\n",
    )
    .unwrap();
    let image = dir.join("halt.bin");
    fs::write(&image, [0xFC, 0, 0, 0]).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_src32_testbench"))
        .arg(&image)
        .args(["0", "--frames", "1"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(dir).unwrap();
}

const STATE_CARD: &str = r#"
(module
  (global $value (mut i32) (i32.const 0))
  (func (export "cpt32_abi_version") (result i32) i32.const 1)
  (func (export "cpt32_reset") i32.const 0 global.set $value)
  (func (export "cpt32_mmio_read") (param i32) (result i32) global.get $value)
  (func (export "cpt32_mmio_write") (param i32) (param i32)
    local.get 1
    global.set $value)
  (func (export "cpt32_tick")))
"#;

#[test]
fn slot_zero_and_slot_seven_map_full_nonoverlapping_windows() {
    let dir = temporary_directory();
    let mut bus = Bus::new();
    install_package(&mut bus, &dir, 0, STATE_CARD, &[]);
    install_package(&mut bus, &dir, 7, STATE_CARD, &[]);

    let slot0_end = EXPANSION_BASE + EXPANSION_SLOT_SIZE - 1;
    let slot7 = EXPANSION_BASE + 7 * EXPANSION_SLOT_SIZE;
    let slot7_end = slot7 + EXPANSION_SLOT_SIZE - 1;
    for base in [EXPANSION_BASE, slot7] {
        let region = bus
            .memory_map()
            .find(|region| region.start == base)
            .unwrap();
        assert_eq!(region.size, EXPANSION_SLOT_SIZE);
    }
    bus.write_u8(slot0_end, 0xA5);
    bus.write_u8(slot7_end, 0x5A);
    assert_eq!(bus.read_u8(slot0_end), 0xA5);
    assert_eq!(bus.read_u8(slot7_end), 0x5A);
    assert_eq!(slot7, 0xAE00_0000);
    assert_eq!(slot7_end, 0xAFFF_FFFF);

    bus.set_access_source(BusAccessSource::Debugger);
    assert_eq!(bus.read_debug_u8(0xB000_0000), None);
    assert!(!bus.take_bus_error());
    fs::remove_dir_all(dir).unwrap();
}

const IRQ_CARD: &str = r#"
(module
  (import "cpt32_host" "schedule_tick" (func $schedule (param i64)))
  (import "cpt32_host" "irq_set" (func $irq (param i32)))
  (func $start i64.const 5 call $schedule)
  (start $start)
  (func (export "cpt32_abi_version") (result i32) i32.const 1)
  (func (export "cpt32_reset") i64.const 0 call $schedule i32.const 0 call $irq)
  (func (export "cpt32_mmio_read") (param i32) (result i32) i32.const 0)
  (func (export "cpt32_mmio_write") (param i32) (param i32))
  (func (export "cpt32_tick") i32.const 1 call $irq))
"#;

#[test]
fn halted_cpu_wakes_for_card_deadline_irq() {
    let dir = temporary_directory();
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    install_package(&mut bus, &dir, 0, IRQ_CARD, &[]);
    let mut cpu = Cpu::new(bus);
    cpu.load_program(0, &[0xFC, 0, 0, 0]);
    cpu.write_mem_u8(IRQC_BASE + 5, 1 << 5);
    cpu.write_mem_u32_be(0xFFFF_0114, 0x1020_002A);
    cpu.write_mem_u32_be(0xFFFF_0118, 0xFC00_0000);

    cpu.run(20);

    assert_eq!(cpu.read_reg(1), 42);
    assert!(!cpu.is_running());
    assert_eq!(cpu.cycles(), 20);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn relaxed_cached_cpu_observes_expansion_deadline_irq() {
    use cpt32::cpu_select::{AnyCpu, CpuKind};
    let mut kinds = vec![CpuKind::CachedRelaxed];
    if cfg!(feature = "jit") {
        kinds.push(CpuKind::Jit);
    }
    for kind in kinds {
        for halt in [false, true] {
            let dir = temporary_directory();
            let mut bus = Bus::new();
            connect_ram(&mut bus);
            install_package(&mut bus, &dir, 0, IRQ_CARD, &[]);
            let mut cpu = AnyCpu::new(kind, bus);
            // HALT or an active branch loop, followed by the same card IRQ handler.
            cpu.load_program(
                0,
                &if halt { 0xfc00_0000u32 } else { 0x2400_fffcu32 }.to_be_bytes(),
            );
            cpu.write_mem_u8(IRQC_BASE + 5, 1 << 5);
            cpu.write_mem_u32_be(0xffff_0114, 0x1020_002a);
            cpu.write_mem_u32_be(0xffff_0118, 0xfc00_0000);
            cpu.run(20);
            assert_eq!(cpu.read_reg(1), 42);
            assert!(!cpu.is_running());
            assert_eq!(cpu.cycles(), 20);
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

fn generated_card_module(
    abi_version: i32,
    read_result_type: &str,
    read_body: &str,
    include_tick: bool,
    import: &str,
) -> String {
    let tick = if include_tick {
        "(func (export \"cpt32_tick\"))"
    } else {
        ""
    };
    format!(
        "(module {import} (func (export \"cpt32_abi_version\") (result i32) i32.const {abi_version}) (func (export \"cpt32_reset\")) (func (export \"cpt32_mmio_read\") (param i32) (result {read_result_type}) {read_body}) (func (export \"cpt32_mmio_write\") (param i32) (param i32)) {tick})"
    )
}

fn write_package_config(dir: &Path, slot: u8, package: &str) -> PathBuf {
    let config = dir.join("config.yaml");
    fs::write(
        &config,
        format!("expansion_card:\n  - slot: {slot}\n    package: \"{package}\"\n"),
    )
    .unwrap();
    config
}

#[test]
fn abi_version_exports_signatures_and_import_allowlist_are_enforced() {
    let dir = temporary_directory();
    let cases = [
        (
            "wrong-version",
            generated_card_module(2, "i32", "i32.const 0", true, ""),
            "unsupported ABI version",
        ),
        (
            "missing-export",
            generated_card_module(1, "i32", "i32.const 0", false, ""),
            "missing export cpt32_tick",
        ),
        (
            "wrong-signature",
            generated_card_module(1, "i64", "i64.const 0", true, ""),
            "wrong type for export cpt32_mmio_read",
        ),
        (
            "forbidden-import",
            generated_card_module(
                1,
                "i32",
                "i32.const 0",
                true,
                "(import \"env\" \"host_file\" (func))",
            ),
            "unsupported import namespace env",
        ),
    ];
    for (name, module, diagnostic) in cases {
        let package = format!("{name}.cpt32card");
        write_card_package(&dir.join(&package), &module, &[]);
        let config = write_package_config(&dir, 0, &package);
        let mut bus = Bus::new();
        let error = load_expansion_cards(&config, &mut bus).unwrap_err();
        assert!(error.contains("slot 0"), "{error}");
        assert!(error.contains(&package), "{error}");
        assert!(error.contains(diagnostic), "{error}");
        assert!(
            !bus.memory_map()
                .any(|region| region.start == EXPANSION_BASE)
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_slots_duplicate_slots_and_missing_packages_fail_before_registration() {
    let dir = temporary_directory();
    let cases = [
        (
            "duplicate slots",
            "expansion_card:\n  - slot: 0\n    package: \"first.cpt32card\"\n  - slot: 0\n    package: \"second.cpt32card\"\n",
            "duplicate slot",
        ),
        (
            "out-of-range slot",
            "expansion_card:\n  - slot: 8\n    package: \"missing.cpt32card\"\n",
            "slot must be 0..7",
        ),
        (
            "missing package",
            "expansion_card:\n  - slot: 3\n    package: \"missing.cpt32card\"\n",
            "missing.cpt32card",
        ),
        (
            "invalid slot type",
            "expansion_card:\n  - slot: \"0\"\n    package: \"bad-type.cpt32card\"\n",
            "bad-type.cpt32card",
        ),
    ];
    for (label, contents, diagnostic) in cases {
        let config = dir.join("config.yaml");
        fs::write(&config, contents).unwrap();
        let mut bus = Bus::new();
        let error = load_expansion_cards(&config, &mut bus).unwrap_err();
        assert!(error.contains(diagnostic), "{label}: {error}");
        assert!(
            !bus.memory_map()
                .any(|region| (EXPANSION_BASE..0xB000_0000).contains(&region.start))
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unreferenced_corrupt_asset_is_not_decompressed() {
    let dir = temporary_directory();
    let package = dir.join("lazy.cpt32card");
    write_card_package(
        &package,
        SIMPLE_CARD,
        &[("assets/unused.bin", b"unused payload")],
    );
    let mut bytes = fs::read(&package).unwrap();
    let data_start = {
        let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        archive.by_name("assets/unused.bin").unwrap().data_start() as usize
    };
    bytes[data_start] ^= 0xFF;
    fs::write(&package, bytes).unwrap();
    let config = write_package_config(&dir, 0, "lazy.cpt32card");

    let mut bus = Bus::new();
    load_expansion_cards(&config, &mut bus).unwrap();
    assert_eq!(bus.read_u8(EXPANSION_BASE), 90);
    assert!(!bus.take_bus_error());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn runtime_import_trap_and_fuel_exhaustion_fault_the_card() {
    const BAD_IMPORT_ARGUMENT: &str = r#"
    (module
      (import "cpt32_host" "irq_set" (func $irq (param i32)))
      (func (export "cpt32_abi_version") (result i32) i32.const 1)
      (func (export "cpt32_reset"))
      (func (export "cpt32_mmio_read") (param i32) (result i32)
        i32.const 2 call $irq i32.const 0)
      (func (export "cpt32_mmio_write") (param i32) (param i32))
      (func (export "cpt32_tick")))
    "#;
    const FUEL_EXHAUSTION: &str = r#"
    (module
      (func (export "cpt32_abi_version") (result i32) i32.const 1)
      (func (export "cpt32_reset"))
      (func (export "cpt32_mmio_read") (param i32) (result i32)
        (loop $forever br $forever)
        i32.const 0)
      (func (export "cpt32_mmio_write") (param i32) (param i32))
      (func (export "cpt32_tick")))
    "#;

    const MEMORY_LIMIT: &str = r#"
    (module
      (memory 1)
      (func (export "cpt32_abi_version") (result i32) i32.const 1)
      (func (export "cpt32_reset"))
      (func (export "cpt32_mmio_read") (param i32) (result i32)
        i32.const 1024
        memory.grow
        drop
        i32.const 0)
      (func (export "cpt32_mmio_write") (param i32) (param i32))
      (func (export "cpt32_tick")))
    "#;
    let dir = temporary_directory();
    for (slot, name, module) in [
        (0, "bad-import-argument.cpt32card", BAD_IMPORT_ARGUMENT),
        (1, "fuel-exhaustion.cpt32card", FUEL_EXHAUSTION),
        (2, "memory-limit.cpt32card", MEMORY_LIMIT),
    ] {
        write_card_package(&dir.join(name), module, &[]);
        let config = write_package_config(&dir, slot, name);
        let mut bus = Bus::new();
        load_expansion_cards(&config, &mut bus).unwrap();
        let address = EXPANSION_BASE + u32::from(slot) * EXPANSION_SLOT_SIZE;
        assert_eq!(bus.read_u8(address), 0);
        assert!(bus.take_bus_error());
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dma_to_empty_slot_uses_existing_address_error_path() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    let bmc = connect_bmc(&mut bus);
    connect_dmac(&mut bus, bmc);
    bus.write_u8(0x100, 0x5A);
    bus.write_u32_be(DMAC_BASE + 8, 1);
    bus.write_u32_be(DMAC_BASE + 0x100, 0x100);
    bus.write_u32_be(DMAC_BASE + 0x104, EXPANSION_BASE + EXPANSION_SLOT_SIZE);
    bus.write_u32_be(DMAC_BASE + 0x108, 1);
    bus.write_u32_be(DMAC_BASE + 0x10C, (1 << 4) | (1 << 8));
    bus.tick_devices();
    bus.tick_devices();
    let status = bus.read_u32_be(DMAC_BASE + 0x110);
    assert_ne!(status & 4, 0);
    assert_eq!((status >> 4) & 0xF, 2);
}

const RESET_CARD: &str = r#"
(module
  (import "cpt32_host" "current_cycles" (func $cycles (result i64)))
  (import "cpt32_host" "schedule_tick" (func $schedule (param i64)))
  (global $count (mut i32) (i32.const 0))
  (func $start i64.const 5 call $schedule)
  (start $start)
  (func (export "cpt32_abi_version") (result i32) i32.const 1)
  (func (export "cpt32_reset")
    call $cycles
    i32.wrap_i64
    global.set $count
    i64.const 0
    call $schedule)
  (func (export "cpt32_mmio_read") (param i32) (result i32) global.get $count)
  (func (export "cpt32_mmio_write") (param i32) (param i32))
  (func (export "cpt32_tick")
    global.get $count
    i32.const 1
    i32.add
    global.set $count))
"#;

#[test]
fn cpu_reset_clears_card_deadline_without_rewinding_virtual_time() {
    let dir = temporary_directory();
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    install_package(&mut bus, &dir, 0, RESET_CARD, &[]);
    bus.advance_system_cycles(3);
    let mut cpu = Cpu::new(bus);
    cpu.reset(0);
    cpu.run(2);
    assert_eq!(cpu.read_mem_u8(EXPANSION_BASE), 3);
    assert_eq!(cpu.cycles(), 2);
    fs::remove_dir_all(dir).unwrap();
}

const START_IRQ_CARD: &str = r#"
(module
  (import "cpt32_host" "irq_set" (func $irq (param i32)))
  (func $start i32.const 1 call $irq)
  (start $start)
  (func (export "cpt32_abi_version") (result i32) i32.const 1)
  (func (export "cpt32_reset") i32.const 0 call $irq)
  (func (export "cpt32_mmio_read") (param i32) (result i32) i32.const 0)
  (func (export "cpt32_mmio_write") (param i32) (param i32))
  (func (export "cpt32_tick")))
"#;

#[test]
fn irq_asserted_during_module_start_is_latched_when_controller_attaches() {
    let dir = temporary_directory();
    let mut bus = Bus::new();
    install_package(&mut bus, &dir, 0, START_IRQ_CARD, &[]);
    let irq = connect_irqc(&mut bus);
    bus.attach_irq_controller(Rc::clone(&irq));
    assert_ne!(irq.borrow().pending() & (1 << 5), 0);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_or_unconfigured_config_leaves_expansion_slots_empty() {
    let dir = temporary_directory();
    let mut bus = Bus::new();
    load_expansion_cards(&dir.join("missing.yaml"), &mut bus).unwrap();
    fs::write(dir.join("config.yaml"), "pcg: {}\n").unwrap();
    load_expansion_cards(&dir.join("config.yaml"), &mut bus).unwrap();
    assert!(
        !bus.memory_map()
            .any(|region| (EXPANSION_BASE..0xB000_0000).contains(&region.start))
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn shared_wasm_memory_is_rejected() {
    const SHARED_MEMORY_CARD: &str = r#"
    (module
      (memory 1 1 shared)
      (func (export "cpt32_abi_version") (result i32) i32.const 1)
      (func (export "cpt32_reset"))
      (func (export "cpt32_mmio_read") (param i32) (result i32) i32.const 0)
      (func (export "cpt32_mmio_write") (param i32) (param i32))
      (func (export "cpt32_tick")))
    "#;

    let dir = temporary_directory();
    write_card_package(&dir.join("shared.cpt32card"), SHARED_MEMORY_CARD, &[]);
    let config = write_package_config(&dir, 0, "shared.cpt32card");
    let mut bus = Bus::new();
    assert!(load_expansion_cards(&config, &mut bus).is_err());
    assert!(
        !bus.memory_map()
            .any(|region| region.start == EXPANSION_BASE)
    );
    fs::remove_dir_all(dir).unwrap();
}
