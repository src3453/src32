//! End-to-end smoke test for `dos.sol` (the CPT32 DOS shell).
//!
//! The guest binary is built from `dos.sol` (gitignored, like `tinybasic.bin`
//! for the TinyBASIC tests):
//!
//! ```text
//! python3 tools/solc/solc.py compile dos.sol -o dos.a
//! python3 tools/asm/asm.py dos.a -o dos.bin
//! ```
//!
//! The tests craft DSFS images in Rust (no host tooling at test time), boot
//! `dos.bin` from RAM, drive the shell by pushing PeC keyboard events (the
//! shell HALT s and is woken by IRQC source 3), and check both the PCG text
//! plane and the resulting on-disk image bytes:
//!
//! * `dos_shell_manages_files_on_two_drives` walks DIR, NEW, REN, DEL,
//!   FORMAT, COPY, HEX, EDIT, FREE and EXIT.
//! * `dos_reports_missing_media` boots with no drive attached.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu::CYCLES_PER_FRAME;
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::pec::idc::idc::{Idc, connect_idc};
use cpt32::devices::pec::pec::{HostInputEvent, connect_pec_input};
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::vdp::vdp::{VDP_VRAM_BASE, Vdp};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tokio::sync::mpsc::UnboundedSender;

const SECTOR: usize = 512;
const TOTAL: usize = 2880;
const ENTRY_SIZE: usize = 32;
const DIR_OFFSET: usize = 3 * SECTOR;
const BITMAP_OFFSET: usize = 2 * SECTOR;

fn be32(image: &mut [u8], offset: usize, value: u32) {
    image[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn set_bit(image: &mut [u8], block: u32) {
    let byte = BITMAP_OFFSET + (block / 8) as usize;
    image[byte] |= 0x80 >> (block % 8);
}

/// Build a valid 1.44 MiB DSFS image holding `files` (first-fit from block 19).
fn make_dsfs(files: &[(&[u8; 11], &[u8])]) -> Vec<u8> {
    let mut image = vec![0u8; TOTAL * SECTOR];
    image[510] = 0x55;
    image[511] = 0xAA;
    let sb = SECTOR;
    image[sb..sb + 4].copy_from_slice(b"DSFS");
    image[sb + 4..sb + 6].copy_from_slice(&1u16.to_be_bytes());
    be32(&mut image, sb + 0x08, TOTAL as u32);
    be32(&mut image, sb + 0x0C, 2); // bitmap_start
    be32(&mut image, sb + 0x10, 1); // bitmap_blocks
    be32(&mut image, sb + 0x14, 3); // dir_start
    be32(&mut image, sb + 0x18, 16); // dir_blocks
    be32(&mut image, sb + 0x1C, 19); // data_start
    be32(&mut image, sb + 0x20, 1_700_000_000);
    // Metadata blocks 0..18 are used.
    image[BITMAP_OFFSET] = 0xFF;
    image[BITMAP_OFFSET + 1] = 0xFF;
    image[BITMAP_OFFSET + 2] = 0xE0;
    let mut next = 19u32;
    for (index, (name, data)) in files.iter().enumerate() {
        let entry = DIR_OFFSET + index * ENTRY_SIZE;
        image[entry] = 1;
        image[entry + 1..entry + 12].copy_from_slice(*name);
        let blocks = data.len().div_ceil(SECTOR) as u32;
        assert!(next + blocks <= TOTAL as u32, "test files exceed DSFS capacity");
        be32(&mut image, entry + 16, if data.is_empty() { 0 } else { next });
        be32(&mut image, entry + 20, data.len() as u32);
        be32(&mut image, entry + 24, 1_700_000_000);
        be32(&mut image, entry + 28, 1_700_000_000);
        if !data.is_empty() {
            let offset = next as usize * SECTOR;
            image[offset..offset + data.len()].copy_from_slice(data);
            for block in next..next + blocks {
                set_bit(&mut image, block);
            }
        }
        next += blocks;
    }
    image
}

/// Build a minimal WAX v2 executable with address-sorted sparse blocks.
fn make_wax(blocks: &[(u32, &[u8])], entry: u32) -> Vec<u8> {
    assert!(!blocks.is_empty());
    let table_offset = 64usize;
    let mut payload_offset = (table_offset + blocks.len() * 12 + 3) & !3;
    let records: Vec<_> = blocks
        .iter()
        .map(|(address, payload)| {
            let record = (*address, payload_offset, payload.len());
            payload_offset = (payload_offset + payload.len() + 3) & !3;
            record
        })
        .collect();
    let file_size = records
        .last()
        .map(|(_, offset, size)| offset + size)
        .unwrap();
    let mut image = vec![0u8; file_size];
    image[0..4].copy_from_slice(b"WAX\0");
    image[4..6].copy_from_slice(&2u16.to_be_bytes());
    image[6..8].copy_from_slice(&1u16.to_be_bytes());
    image[8..10].copy_from_slice(&1u16.to_be_bytes());
    be32(&mut image, 12, 64);
    be32(&mut image, 16, blocks[0].0);
    be32(&mut image, 20, entry - blocks[0].0);
    be32(&mut image, 24, table_offset as u32);
    be32(&mut image, 28, blocks.len() as u32);
    be32(&mut image, 32, 12);
    be32(&mut image, 44, file_size as u32);
    for (index, ((address, payload), (_, offset, size))) in
        blocks.iter().zip(records.iter()).enumerate()
    {
        let record = table_offset + index * 12;
        be32(&mut image, record, *address);
        be32(&mut image, record + 4, *offset as u32);
        be32(&mut image, record + 8, *size as u32);
        image[*offset..*offset + payload.len()].copy_from_slice(payload);
    }
    image
}

fn entry(image: &[u8], index: usize) -> &[u8] {
    let offset = DIR_OFFSET + index * ENTRY_SIZE;
    &image[offset..offset + ENTRY_SIZE]
}

fn entry_field(image: &[u8], index: usize, offset: usize) -> u32 {
    let entry = entry(image, index);
    u32::from_be_bytes(entry[offset..offset + 4].try_into().unwrap())
}

fn write_config(root: &Path, slots: &[(&str, bool)]) {
    let mut text = String::new();
    if !slots.is_empty() {
        text.push_str("disk:\n");
    }
    for (index, (image, read_only)) in slots.iter().enumerate() {
        text.push_str(&format!(
            "  - slot: {index}\n    type: fdd\n    image: {image}\n    read_only: {read_only}\n    \
             transfer_rate_bytes_per_second: 48000000\n    access_latency_ms: 0\n"
        ));
    }
    std::fs::write(root.join("config.yaml"), text).unwrap();
}

struct Harness {
    cpu: AnyCpu,
    vdp: Rc<RefCell<Vdp>>,
    input: UnboundedSender<HostInputEvent>,
    root: PathBuf,
    cycles_per_frame: usize,
}

impl Harness {
    fn boot(root: &Path) -> Harness {
        Self::boot_with_kind(root, CpuKind::Scalar)
    }

    fn boot_with_kind(root: &Path, cpu_kind: CpuKind) -> Harness {
        let drives = Idc::load_drives(&root.join("config.yaml")).unwrap();
        let mut bus = Bus::new();
        let input = connect_pec_input(&mut bus);
        connect_rng(&mut bus);
        let vdp = connect_devices_with_vdp(&mut bus);
        connect_idc(&mut bus, drives);
        let mut cpu = AnyCpu::new(cpu_kind, bus);
        let guest = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/dos.bin")).expect(
            "dos.bin is missing; build it from dos.sol (see the module comment)",
        );
        cpu.load_program(0, &guest);
        Harness {
            cpu,
            vdp,
            input,
            root: root.to_path_buf(),
            cycles_per_frame: CYCLES_PER_FRAME as usize,
        }
    }

    fn image(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.root.join(name)).unwrap()
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            self.vdp.borrow_mut().tick();
            self.cpu.set_irq_source(0, true);
            self.cpu.set_irq_source(0, false);
            self.cpu.run(self.cycles_per_frame);
        }
    }

    /// The visible 80x30 PCG character plane, non-printable cells blanked.
    fn screen(&mut self) -> String {
        self.screen_width(80)
    }

    /// PCG rows are packed using the active text width (40 or 80 columns).
    fn screen_width(&mut self, width: usize) -> String {
        let mut text = String::new();
        for cell in 0..width * 30 {
            let byte = self.cpu.read_mem_u8(VDP_VRAM_BASE + cell as u32);
            text.push(if byte.is_ascii_graphic() || byte == b' ' {
                byte as char
            } else {
                ' '
            });
        }
        text
    }

    fn key(&self, usage: u8, down: bool) {
        self.input
            .send(HostInputEvent::Key { usage, down })
            .unwrap();
    }

    fn type_text(&self, text: &str) {
        for ch in text.chars() {
            let (usage, shifted) = key_for(ch);
            if shifted {
                self.key(225, true);
            }
            self.key(usage, true);
            self.key(usage, false);
            if shifted {
                self.key(225, false);
            }
        }
    }

    /// Run frames until `needle` shows up on the text plane.
    fn wait_text(&mut self, needle: &str, max_frames: usize) -> String {
        for _ in 0..max_frames {
            self.frames(1);
            let text = self.screen();
            if text.contains(needle) {
                return text;
            }
        }
        let pc = self.cpu.pc();
        let running = self.cpu.is_running();
        let cycles = self.cpu.cycles();
        let irq_pending = (u16::from(self.cpu.read_mem_u8(0xFFFF_0040)) << 8)
            | u16::from(self.cpu.read_mem_u8(0xFFFF_0041));
        let irq_enable = (u16::from(self.cpu.read_mem_u8(0xFFFF_0044)) << 8)
            | u16::from(self.cpu.read_mem_u8(0xFFFF_0045));
        panic!(
            "timeout waiting for {needle:?} (running={running}, pc=0x{pc:08X}, cycles={cycles}, IRQC pending=0x{irq_pending:04X}, enable=0x{irq_enable:04X}); screen:\n{}\n",
            wrap(&self.screen())
        );
    }

    fn command(&mut self, text: &str, expect: &str, max_frames: usize) {
        self.type_text(text);
        self.type_text("\r");
        self.wait_text(expect, max_frames);
    }
}

/// US HID usage id for a character typed by the shell test (a=4 ... 0=39).
fn key_for(ch: char) -> (u8, bool) {
    match ch {
        'a'..='z' => (4 + (ch as u8 - b'a'), false),
        'A'..='Z' => (4 + (ch as u8 - b'A'), true),
        '1'..='9' => (30 + (ch as u8 - b'1'), false),
        '0' => (39, false),
        ' ' => (44, false),
        '.' => (55, false),
        ':' => (51, true),
        '-' => (45, false),
        '_' => (45, true),
        '\r' | '\n' => (40, false),
        other => panic!("no HID usage mapping for {other:?}"),
    }
}

/// Split a packed PCG text plane into rows at the selected cell width.
fn wrap_width(text: &str, width: usize) -> String {
    text.as_bytes()
        .chunks(width)
        .map(|row| String::from_utf8_lossy(row).into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

fn wrap(text: &str) -> String {
    wrap_width(text, 80)
}

fn assert_dir_columns(text: &str, width: usize, name: &str, size: &str, blocks: &str) {
    let rendered = wrap_width(text, width);
    let header = rendered
        .lines()
        .find(|line| line.contains("#   NAME"))
        .unwrap();
    let row = rendered.lines().find(|line| line.contains(name)).unwrap();
    assert_eq!(&header[23..27], "SIZE", "DIR size heading:\n{rendered}");
    assert_eq!(&header[31..37], "BLOCKS", "DIR blocks heading:\n{rendered}");
    assert_eq!(row[23..31].trim(), size, "DIR size column:\n{rendered}");
    assert_eq!(row[31..36].trim(), blocks, "DIR blocks column:\n{rendered}");
}

fn assert_hex_columns(text: &str) {
    let rendered = wrap(text);
    let labels = rendered
        .lines()
        .find(|line| line.contains("00 01 02 03 04 05 06 07"))
        .unwrap();
    let first_row = rendered
        .lines()
        .find(|line| line.starts_with("0000:"))
        .unwrap();
    assert_eq!(&labels[6..8], "00", "HEX first column label:\n{rendered}");
    assert_eq!(&first_row[6..8], "30", "HEX first byte:\n{rendered}");
    assert_eq!(&first_row[54..55], "|", "HEX ASCII separator:\n{rendered}");
}

fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("dos-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn hex_returns_to_shell_at_budget(
    cycles_per_frame: usize,
    cpu_kind: CpuKind,
    tag: &str,
) {
    let root = temp_root(tag);
    let contents = b"0123456789ABCD";
    std::fs::write(
        root.join("disk0.img"),
        make_dsfs(&[(b"TEST    TXT", contents.as_slice())]),
    )
    .unwrap();
    write_config(&root, &[("disk0.img", true)]);

    let mut h = Harness::boot_with_kind(&root, cpu_kind);
    h.wait_text("CPT32 DOS", 200);
    h.wait_text("0> ", 200);
    h.cycles_per_frame = cycles_per_frame;

    let render_frames = 16_000_000usize.div_ceil(cycles_per_frame);
    let command_frames = 4_000_000usize.div_ceil(cycles_per_frame);
    h.command("HEX TEST.TXT", "HEX EDITOR", 2_000);
    h.frames(render_frames);
    let text = h.screen();
    assert_hex_columns(&text);
    assert!(text.contains("00F0:"), "HEX rendering did not finish:\n{text}");
    h.command("Q", "0>", command_frames);
    h.command("HELP", "COPY SRC DST", render_frames);
    h.command("EXIT", "BYE", command_frames);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn dos_hex_returns_to_shell_at_800_cycles_per_frame() {
    hex_returns_to_shell_at_budget(800, CpuKind::Scalar, "hex-slow");
}

#[test]
fn dos_hex_returns_to_shell_at_default_cycles_per_frame() {
    hex_returns_to_shell_at_budget(CYCLES_PER_FRAME as usize, CpuKind::Scalar, "hex-default");
}

#[test]
fn dos_pipeline_hex_returns_to_shell_at_800_cycles_per_frame() {
    hex_returns_to_shell_at_budget(800, CpuKind::Pipelined, "hex-pipe-slow");
}

#[test]
fn dos_pipeline_hex_returns_to_shell_at_default_cycles_per_frame() {
    hex_returns_to_shell_at_budget(
        CYCLES_PER_FRAME as usize,
        CpuKind::Pipelined,
        "hex-pipe-default",
    );
}

#[test]
fn dos_dir_columns_align_in_40_and_80_column_modes() {
    let root = temp_root("dir-columns");
    let hello = b"HELLO FROM DSFS\n";
    let tinybas = vec![0u8; 110_228];
    let image = make_dsfs(&[
        (b"HELLO   TXT", hello.as_slice()),
        (b"TINYBAS WAX", tinybas.as_slice()),
    ]);
    std::fs::write(root.join("disk0.img"), image).unwrap();
    write_config(&root, &[("disk0.img", false)]);

    let mut h = Harness::boot(&root);
    h.wait_text("CPT32 DOS", 200);
    h.wait_text("0> ", 200);
    h.command("MODE 40", "40-COLUMN MODE", 200);
    h.command("DIR", "HELLO.TXT", 200);
    assert_dir_columns(&h.screen_width(40), 40, "HELLO.TXT", "16", "1");
    assert_dir_columns(&h.screen_width(40), 40, "TINYBAS.WAX", "110228", "216");
    h.command("MODE 80", "80-COLUMN MODE", 200);
    h.command("DIR", "HELLO.TXT", 200);
    assert_dir_columns(&h.screen(), 80, "HELLO.TXT", "16", "1");
    assert_dir_columns(&h.screen(), 80, "TINYBAS.WAX", "110228", "216");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn dos_runs_sparse_wax_and_rejects_invalid_images() {
    let root = temp_root("wax-run");
    let app = make_wax(
        &[
            (0x0000_0000, &[0x00, 0x00, 0x00, 0x00]),
            (0x0001_0000, &[0xFC, 0x00, 0x00, 0x00]),
            (0x0031_0000, &[0xA5, 0x5A, 0xC3, 0x3C]),
        ],
        0x0001_0000,
    );
    let reserved = make_wax(&[(0x00FE_0000, &[0xFC, 0x00, 0x00, 0x00])], 0x00FE_0000);
    let overlap = make_wax(
        &[
            (0x0002_0000, &[0x00, 0x00, 0x00, 0x00]),
            (0x0002_0000, &[0xFC, 0x00, 0x00, 0x00]),
        ],
        0x0002_0000,
    );
    let mut shared = make_wax(
        &[
            (0x0003_0000, &[0x00, 0x00, 0x00, 0x00]),
            (0x0004_0000, &[0xFC, 0x00, 0x00, 0x00]),
        ],
        0x0003_0000,
    );
    be32(&mut shared, 64 + 12 + 4, 88);
    let image = make_dsfs(&[
        (b"BROKEN  WAX", b"not WAX".as_slice()),
        (b"RESERVE WAX", reserved.as_slice()),
        (b"OVERLAP WAX", overlap.as_slice()),
        (b"SHARED  WAX", shared.as_slice()),
        (b"APP     WAX", app.as_slice()),
    ]);
    std::fs::write(root.join("disk0.img"), image).unwrap();
    write_config(&root, &[("disk0.img", false)]);

    let mut h = Harness::boot(&root);
    h.wait_text("CPT32 DOS", 200);
    h.wait_text("0> ", 200);
    h.command("RUN BROKEN.WAX", "INVALID WAX HEADER", 200);
    h.command("RUN RESERVE.WAX", "WAX USES RESERVED RAM", 200);
    h.command("RUN OVERLAP.WAX", "INVALID WAX LAYOUT", 200);
    h.command("RUN SHARED.WAX", "OVERLAPPING WAX DATA", 200);

    h.type_text("RUN APP.WAX");
    h.type_text("\r");
    let mut launched = false;
    for _ in 0..400 {
        h.frames(1);
        if !h.cpu.is_running()
            && h.cpu.read_mem_u8(0x0000_0000) == 0x00
            && h.cpu.read_mem_u8(0x0001_0000) == 0xFC
            && h.cpu.read_mem_u8(0x0031_0000) == 0xA5
        {
            launched = true;
            break;
        }
    }
    assert!(
        launched,
        "DOS did not load and execute WAX: running={}, pc={:#010X}, screen:\n{}",
        h.cpu.is_running(),
        h.cpu.pc(),
        wrap(&h.screen())
    );
    assert_eq!(h.cpu.read_mem_u8(0x0001_0001), 0x00);
    assert_eq!(h.cpu.read_mem_u8(0x0080_0000), 0, "sparse hole was modified");
    assert!(!h.cpu.is_running(), "WAX HALT should stop the guest CPU");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn dos_shell_manages_files_on_two_drives() {
    let root = temp_root("shell");
    let hello = b"HELLO FROM DSFS\n";
    std::fs::write(
        root.join("disk0.img"),
        make_dsfs(&[(b"HELLO   TXT", hello.as_slice())]),
    )
    .unwrap();
    std::fs::write(root.join("disk1.img"), vec![0u8; TOTAL * SECTOR]).unwrap();
    write_config(&root, &[("disk0.img", false), ("disk1.img", false)]);

    let mut h = Harness::boot(&root);
    h.wait_text("CPT32 DOS", 200);
    h.wait_text("0> ", 200);
    h.command("MODE 40", "40-COLUMN MODE", 200);
    h.command("DIR", "HELLO.TXT", 200);
    assert!(h.screen().contains("0> "), "40-column prompt missing");
    h.command("MODE 80", "80-COLUMN MODE", 200);
    assert!(h.screen().contains("0> "), "80-column prompt missing");

    // DIR lists the preexisting file with its size.
    h.command("DIR", "HELLO.TXT", 200);
    let text = h.screen();
    assert!(text.contains("DRIVE 0"), "drive header missing:\n{text}");
    assert!(text.contains("1 FILE(S)"), "file count missing:\n{text}");
    assert!(text.contains("16"), "file size missing:\n{text}");

    // NEW creates an empty entry in the first free slot.
    h.command("NEW EMPTY.TXT", "CREATED EMPTY.TXT", 200);
    let image = h.image("disk0.img");
    assert_eq!(entry(&image, 1)[0], 1, "EMPTY.TXT entry not used");
    assert_eq!(&entry(&image, 1)[1..12], b"EMPTY   TXT");
    assert_eq!(entry_field(&image, 1, 16), 0, "empty file must not use blocks");
    assert_eq!(entry_field(&image, 1, 20), 0);

    // REN changes only the name; timestamps and allocation are untouched.
    let before = h.image("disk0.img");
    let created = entry_field(&before, 1, 24);
    let modified = entry_field(&before, 1, 28);
    h.command("REN EMPTY.TXT RENAMED.TXT", "RENAMED EMPTY.TXT", 200);
    let image = h.image("disk0.img");
    assert_eq!(&entry(&image, 1)[1..12], b"RENAMED TXT");
    assert_eq!(entry_field(&image, 1, 24), created, "created kept");
    assert_eq!(entry_field(&image, 1, 28), modified, "modified kept");
    assert_eq!(entry_field(&image, 1, 16), 0, "start_block kept");
    assert_eq!(entry_field(&image, 1, 20), 0, "size kept");

    // DEL clears the entry.
    h.command("DEL RENAMED.TXT", "DELETED RENAMED.TXT", 200);
    let image = h.image("disk0.img");
    assert!(
        entry(&image, 1).iter().all(|&b| b == 0),
        "deleted entry not zeroed"
    );

    // FORMAT builds a fresh DSFS volume on drive 1 (guest-side format).
    h.command("FORMAT 1", "FORMATTED 2880 BLOCKS", 400);
    let image = h.image("disk1.img");
    assert_eq!(image[510..512], [0x55, 0xAA], "boot signature");
    assert_eq!(&image[SECTOR..SECTOR + 4], b"DSFS");
    assert_eq!(image[SECTOR + 4..SECTOR + 6], [0, 1], "version");
    assert_eq!(image[SECTOR + 6..SECTOR + 8], [0, 0], "flags");
    assert_eq!(
        (
            u32::from_be_bytes(image[SECTOR + 0x08..SECTOR + 0x0C].try_into().unwrap()),
            u32::from_be_bytes(image[SECTOR + 0x0C..SECTOR + 0x10].try_into().unwrap()),
            u32::from_be_bytes(image[SECTOR + 0x10..SECTOR + 0x14].try_into().unwrap()),
            u32::from_be_bytes(image[SECTOR + 0x14..SECTOR + 0x18].try_into().unwrap()),
            u32::from_be_bytes(image[SECTOR + 0x18..SECTOR + 0x1C].try_into().unwrap()),
            u32::from_be_bytes(image[SECTOR + 0x1C..SECTOR + 0x20].try_into().unwrap()),
        ),
        (2880, 2, 1, 3, 16, 19),
        "superblock geometry"
    );
    assert_eq!(
        image[BITMAP_OFFSET..BITMAP_OFFSET + 4],
        [0xFF, 0xFF, 0xE0, 0x00],
        "metadata bits only"
    );
    assert!(
        image[BITMAP_OFFSET + 4..BITMAP_OFFSET + SECTOR].iter().all(|&b| b == 0),
        "padding bits must stay clear"
    );
    assert!(
        image[DIR_OFFSET..DIR_OFFSET + 16 * SECTOR].iter().all(|&b| b == 0),
        "directory must be empty"
    );

    // COPY moves a file between drives.
    h.command("COPY 0:HELLO.TXT 1:HELLO.TXT", "COPIED 16 BYTES", 400);
    let image = h.image("disk1.img");
    assert_eq!(entry(&image, 0)[0], 1);
    assert_eq!(&entry(&image, 0)[1..12], b"HELLO   TXT");
    assert_eq!(entry_field(&image, 0, 16), 19, "first-fit start block");
    assert_eq!(entry_field(&image, 0, 20), 16);
    assert_eq!(
        entry_field(&image, 0, 24),
        entry_field(&image, 0, 28),
        "created/modified use the same RTC timestamp"
    );
    assert_eq!(&image[19 * SECTOR..19 * SECTOR + hello.len()], hello);
    assert!(image[19 * SECTOR + hello.len()..20 * SECTOR].iter().all(|&b| b == 0));
    assert_eq!(image[BITMAP_OFFSET + 2], 0xF0, "copied block marked used");

    // HEX edits bytes in place and saves them.
    h.command("DRV 0", "DRIVE 0 READY", 200);
    h.command("HEX HELLO.TXT", "HEX EDITOR", 200);
    h.command("E 0 58 58", "|XX", 200);
    h.command("W", "SAVED 16 BYTES", 200);
    h.command("Q", "0>", 200);
    h.command("TYPE HELLO.TXT", "XXLLO FROM DSFS", 200);
    let image = h.image("disk0.img");
    assert_eq!(&image[19 * SECTOR..19 * SECTOR + 2], b"XX");
    assert_eq!(entry_field(&image, 0, 20), 16, "hex editor keeps the size");

    // EDIT: append, insert, delete and replace lines, then save as text.
    h.command("EDIT NOTE.TXT", "LINE EDITOR", 200);
    h.command("A FIRST LINE", "1 LINE(S)", 200);
    h.command("A SECOND LINE", "2 LINE(S)", 200);
    h.command("A THIRD LINE", "3 LINE(S)", 200);
    h.command("I 2 INSERTED", "4 LINE(S)", 200);
    h.command("D 4", "3 LINE(S)", 200);
    h.command("3 REPLACED LINE", "3  REPLACED LINE", 200);
    h.command("W", "SAVED 34 BYTES", 200);
    h.command("Q", "EDITOR CLOSED", 200);
    h.command("TYPE NOTE.TXT", "REPLACED LINE", 200);
    let image = h.image("disk0.img");
    assert_eq!(&entry(&image, 1)[1..12], b"NOTE    TXT");
    assert_eq!(entry_field(&image, 1, 20), 34);
    assert_eq!(entry_field(&image, 1, 16), 20, "NOTE.TXT first-fit block");
    let note = &image[20 * SECTOR..20 * SECTOR + 34];
    assert_eq!(note, b"FIRST LINE\nINSERTED\nREPLACED LINE\n");
    assert!(image[20 * SECTOR + 34..21 * SECTOR].iter().all(|&b| b == 0));

    // FREE reports the volume geometry.
    h.command("FREE", "FREE BLOCKS", 200);

    // EXIT stops the CPU.
    h.command("EXIT", "BYE", 200);
    let pc = h.cpu.pc();
    h.frames(5);
    assert_eq!(h.cpu.pc(), pc, "guest kept running after EXIT");

    std::fs::remove_dir_all(&root).unwrap();
}

/// A write-protected volume must reject writes instead of reporting success.
#[test]
fn dos_reports_write_protected_media() {
    let root = temp_root("wprot");
    let hello = b"HELLO FROM DSFS\n";
    std::fs::write(
        root.join("disk0.img"),
        make_dsfs(&[(b"HELLO   TXT", hello.as_slice())]),
    )
    .unwrap();
    write_config(&root, &[("disk0.img", true)]);

    let mut h = Harness::boot(&root);
    h.wait_text("CPT32 DOS", 200);
    h.command("DIR", "HELLO.TXT", 200);
    h.command("NEW NEW.TXT", "CREATE FAILED", 200);
    let image = h.image("disk0.img");
    assert_eq!(entry(&image, 1)[0], 0, "read-only volume gained an entry");
    h.command("EXIT", "BYE", 200);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn dos_reports_missing_media() {
    let root = temp_root("nodisk");
    write_config(&root, &[]);
    let mut h = Harness::boot(&root);
    h.wait_text("CPT32 DOS", 200);
    h.wait_text("IS NOT MOUNTED", 200);
    h.command("DIR", "DRIVE NOT MOUNTED", 200);
    h.command("FORMAT 0", "NO MEDIA IN DRIVE", 200);
    h.command("HELP", "COPY SRC DST", 200);
    std::fs::remove_dir_all(&root).unwrap();
}
