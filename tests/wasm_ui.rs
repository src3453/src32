//! GUI panel extension of the WASM card ABI (`doc/spec_WASM_expansion_card.md` §3.4).

use cpt32::bus::Bus;
use cpt32::devices::expansion::{
    EXPANSION_BASE, ExpansionCallKind, ExpansionUiEvent, ExpansionUiItem, load_expansion_cards,
};
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_directory() -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("cpt32-ui-{}-{id}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

/// Load `module` into slot 0 with the given manifest capabilities.
fn load(dir: &Path, module: &str, capabilities: &str) -> Result<Bus, String> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    archive.start_file("manifest.yaml", options).unwrap();
    write!(
        archive,
        "abi_version: 1\nmodule: card.wasm\ncapabilities: [{capabilities}]\n"
    )
    .unwrap();
    archive.start_file("card.wasm", options).unwrap();
    archive.write_all(&wat::parse_str(module).unwrap()).unwrap();
    fs::write(
        dir.join("card.cpt32card"),
        archive.finish().unwrap().into_inner(),
    )
    .unwrap();
    let config = dir.join("config.yaml");
    fs::write(
        &config,
        "expansion_card:\n  - slot: 0\n    package: card.cpt32card\n",
    )
    .unwrap();
    let mut bus = Bus::new();
    load_expansion_cards(&config, &mut bus)?;
    Ok(bus)
}

/// MMIO read 0 = clicks, 1 = checkbox, 2 = int (low byte), 3 = last text
/// result (low byte), 0x80.. = text buffer. Write 0 sets a misbehaviour mode;
/// write 1 calls `ui_text` outside `cpt32_ui`.
const PANEL: &str = r#"
(module
  (import "cpt32_host" "ui_text" (func $text (param i32 i32)))
  (import "cpt32_host" "ui_text_colored" (func $colored (param i32 i32 i32)))
  (import "cpt32_host" "ui_separator" (func $sep))
  (import "cpt32_host" "ui_same_line" (func $same))
  (import "cpt32_host" "ui_button" (func $button (param i32 i32 i32) (result i32)))
  (import "cpt32_host" "ui_checkbox" (func $checkbox (param i32 i32 i32 i32) (result i32)))
  (import "cpt32_host" "ui_input_int" (func $int (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "cpt32_host" "ui_input_text" (func $input (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "cpt32_host" "irq_set" (func $irq (param i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "Hello")
  (data (i32.const 16) "Go")
  (data (i32.const 32) "Chk")
  (data (i32.const 48) "Num")
  (data (i32.const 64) "Txt")
  (data (i32.const 128) "ab")
  (global $mode (mut i32) (i32.const 0))
  (global $clicks (mut i32) (i32.const 0))
  (global $chk (mut i32) (i32.const 0))
  (global $num (mut i32) (i32.const 50))
  (global $tlen (mut i32) (i32.const 2))
  (global $tres (mut i32) (i32.const 0))
  (func (export "cpt32_abi_version") (result i32) i32.const 1)
  (func (export "cpt32_reset"))
  (func (export "cpt32_tick"))
  (func (export "cpt32_mmio_read") (param $o i32) (result i32)
    (if (i32.ge_u (local.get $o) (i32.const 0x80))
      (then (return (i32.load8_u (local.get $o)))))
    (if (i32.eq (local.get $o) (i32.const 0)) (then (return (global.get $clicks))))
    (if (i32.eq (local.get $o) (i32.const 1)) (then (return (global.get $chk))))
    (if (i32.eq (local.get $o) (i32.const 2))
      (then (return (i32.and (global.get $num) (i32.const 255)))))
    (i32.and (global.get $tres) (i32.const 255)))
  (func (export "cpt32_mmio_write") (param $o i32) (param $v i32)
    (if (i32.eq (local.get $o) (i32.const 0)) (then (global.set $mode (local.get $v))))
    (if (i32.eq (local.get $o) (i32.const 1)) (then (call $text (i32.const 0) (i32.const 5)))))
  (func (export "cpt32_ui")
    (local $r i32) (local $i i32)
    (call $text (i32.const 0) (i32.const 5))
    (call $colored (i32.const 0xFF0000FF) (i32.const 0) (i32.const 5))
    (call $sep)
    (call $same)
    (if (call $button (i32.const 1) (i32.const 16) (i32.const 2))
      (then
        (global.set $clicks (i32.add (global.get $clicks) (i32.const 1)))
        (call $irq (i32.const 1))))
    (global.set $chk
      (call $checkbox (i32.const 2) (i32.const 32) (i32.const 3) (global.get $chk)))
    (global.set $num
      (call $int (i32.const 3) (i32.const 48) (i32.const 3) (global.get $num)
        (i32.const -5) (i32.const 10)))
    (local.set $r
      (call $input (i32.const 4) (i32.const 64) (i32.const 3) (i32.const 128) (i32.const 8)
        (global.get $tlen)))
    (if (i32.ge_s (local.get $r) (i32.const 0)) (then (global.set $tlen (local.get $r))))
    (global.set $tres (local.get $r))
    (if (i32.eq (global.get $mode) (i32.const 1))
      (then (drop (call $button (i32.const 1) (i32.const 16) (i32.const 2)))))
    (if (i32.eq (global.get $mode) (i32.const 2))
      (then (drop (call $checkbox (i32.const 9) (i32.const 32) (i32.const 3) (i32.const 2)))))
    (if (i32.eq (global.get $mode) (i32.const 3))
      (then (call $text (i32.const 0) (i32.const 300))))
    (if (i32.eq (global.get $mode) (i32.const 4))
      (then
        (loop $more
          (call $sep)
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br_if $more (i32.lt_u (local.get $i) (i32.const 600))))))
    (if (i32.eq (global.get $mode) (i32.const 5))
      (then (drop (call $int (i32.const 9) (i32.const 48) (i32.const 3) (i32.const 0)
        (i32.const 1) (i32.const 0))))))
)
"#;

fn ui(bus: &mut Bus) -> Vec<ExpansionUiItem> {
    bus.expansion_card_ui(0).expect("panel")
}

#[test]
fn panel_widgets_are_recorded_in_order_and_input_returns_next_call() {
    let dir = temporary_directory();
    let mut bus = load(&dir, PANEL, "gui").unwrap();
    let info = &bus.expansion_card_debug_info()[0];
    assert!(info.gui_capable);
    assert!(info.features().contains(&"GUI panel"));

    let items = ui(&mut bus);
    assert_eq!(
        items,
        [
            ExpansionUiItem::Text {
                text: "Hello".into(),
                color: None
            },
            ExpansionUiItem::Text {
                text: "Hello".into(),
                color: Some([1.0, 0.0, 0.0, 1.0])
            },
            ExpansionUiItem::Separator,
            ExpansionUiItem::SameLine,
            ExpansionUiItem::Button {
                id: 1,
                label: "Go".into()
            },
            ExpansionUiItem::Checkbox {
                id: 2,
                label: "Chk".into(),
                value: false
            },
            ExpansionUiItem::InputInt {
                id: 3,
                label: "Num".into(),
                value: 10,
                min: -5,
                max: 10
            },
            ExpansionUiItem::InputText {
                id: 4,
                label: "Txt".into(),
                value: "ab".into(),
                capacity: 8
            },
        ]
    );
    assert_eq!(bus.read_u8(EXPANSION_BASE + 2), 10, "value is clamped");
    assert_eq!(bus.read_u8(EXPANSION_BASE + 3), 0xFF, "no text submitted");

    bus.expansion_card_ui_event(0, 1, ExpansionUiEvent::Clicked);
    bus.expansion_card_ui_event(0, 2, ExpansionUiEvent::Checkbox(true));
    bus.expansion_card_ui_event(0, 3, ExpansionUiEvent::Int(-100));
    bus.expansion_card_ui_event(0, 4, ExpansionUiEvent::Text("hello world".into()));
    assert_eq!(
        bus.read_u8(EXPANSION_BASE),
        0,
        "nothing is delivered before cpt32_ui"
    );
    let items = ui(&mut bus);
    assert_eq!(bus.read_u8(EXPANSION_BASE), 1);
    assert_eq!(bus.read_u8(EXPANSION_BASE + 1), 1);
    assert_eq!(bus.read_u8(EXPANSION_BASE + 2), (-5i32 as u8));
    assert_eq!(
        bus.read_u8(EXPANSION_BASE + 3),
        8,
        "submitted text cut to cap"
    );
    let buffer: Vec<u8> = (0..8)
        .map(|i| bus.read_u8(EXPANSION_BASE + 0x80 + i))
        .collect();
    assert_eq!(buffer, b"hello wo");
    assert!(items.contains(&ExpansionUiItem::InputText {
        id: 4,
        label: "Txt".into(),
        value: "hello wo".into(),
        capacity: 8
    }));
    assert!(
        bus.expansion_card_debug_info()[0].irq_asserted,
        "cpt32_ui may change card state"
    );

    // Events are consumed once; wrong-kind and unknown-ID events are dropped.
    bus.expansion_card_ui_event(0, 1, ExpansionUiEvent::Int(3));
    bus.expansion_card_ui_event(0, 99, ExpansionUiEvent::Clicked);
    ui(&mut bus);
    ui(&mut bus);
    assert_eq!(bus.read_u8(EXPANSION_BASE), 1);
    assert_eq!(bus.read_u8(EXPANSION_BASE + 3), 0xFF);

    // Reset drops undelivered input.
    bus.expansion_card_ui_event(0, 1, ExpansionUiEvent::Clicked);
    bus.reset_expansion_cards();
    ui(&mut bus);
    assert_eq!(bus.read_u8(EXPANSION_BASE), 1);

    let calls = bus.expansion_card_debug_info()[0]
        .call(ExpansionCallKind::Ui)
        .calls;
    assert_eq!(calls, 5);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn panel_abi_violations_fault_the_card() {
    for (offset, mode, reason) in [
        // The MMIO fault reason shows only the top-level trap, not the host message.
        (1, 1, "cpt32_mmio_write trapped"),
        (0, 1, "duplicate ui widget id 1"),
        (0, 2, "ui_checkbox value must be 0 or 1"),
        (0, 3, "ui text length out of range"),
        (0, 4, "too many ui widgets"),
        (0, 5, "min is greater than max"),
    ] {
        let dir = temporary_directory();
        let mut bus = load(&dir, PANEL, "gui").unwrap();
        bus.write_u8(EXPANSION_BASE + offset, mode);
        if offset == 0 {
            assert!(bus.expansion_card_ui(0).is_none(), "mode {mode}");
        }
        let info = &bus.expansion_card_debug_info()[0];
        assert!(info.faulted, "mode {mode}");
        let fault = info.fault_reason.as_deref().unwrap();
        assert!(fault.contains(reason), "mode {mode}: {fault}");
        assert!(bus.expansion_card_ui(0).is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn gui_capability_gates_imports_and_requires_the_export_and_memory() {
    let dir = temporary_directory();
    let error = load(&dir, PANEL, "").err().unwrap();
    assert!(error.contains("requires the \"gui\" capability"), "{error}");

    let no_export = PANEL.replace("(func (export \"cpt32_ui\")", "(func $ui");
    let error = load(&dir, &no_export, "gui").err().unwrap();
    assert!(error.contains("requires export cpt32_ui"), "{error}");

    let no_memory = PANEL.replace("(memory (export \"memory\") 1)", "(memory 1)");
    let error = load(&dir, &no_memory, "gui").err().unwrap();
    assert!(error.contains("requires exported memory"), "{error}");

    let error = load(&dir, PANEL, "gui, gui").err().unwrap();
    assert!(error.contains("duplicate capability"), "{error}");

    // A card without the capability may export cpt32_ui; it is never called.
    let plain = r#"(module
      (memory (export "memory") 1)
      (func (export "cpt32_abi_version") (result i32) i32.const 1)
      (func (export "cpt32_reset")) (func (export "cpt32_tick"))
      (func (export "cpt32_ui") unreachable)
      (func (export "cpt32_mmio_read") (param i32) (result i32) i32.const 0)
      (func (export "cpt32_mmio_write") (param i32 i32)))"#;
    let mut bus = load(&dir, plain, "").unwrap();
    assert!(bus.expansion_card_ui(0).is_none());
    let info = &bus.expansion_card_debug_info()[0];
    assert!(!info.gui_capable && !info.faulted);

    let wrong = plain.replace(
        "(func (export \"cpt32_ui\") unreachable)",
        "(func (export \"cpt32_ui\") (param i32))",
    );
    let error = load(&dir, &wrong, "").err().unwrap();
    assert!(error.contains("wrong type for export cpt32_ui"), "{error}");
    fs::remove_dir_all(dir).unwrap();
}
