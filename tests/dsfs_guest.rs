//! Guest-side DSFS regression: sol/cpt32/idc.sol + sol/cpt32/dsfs.sol on a
//! real IDC+DMAC image.
//!
//! The guest binary is built from `tools/dsfs/dsfs_smoke.sol` (gitignored,
//! like `tinybasic.bin` for the TinyBASIC tests):
//!
//! ```text
//! python3 tools/solc/solc.py compile tools/dsfs/dsfs_smoke.sol -o dsfs_guest.a
//! python3 tools/asm/asm.py dsfs_guest.a -o dsfs_guest.bin
//! ```
//!
//! The test crafts a DSFS image in Rust (no host tooling at test time):
//! slot 0 holds "GUEST.TXT" ("Hello DSFS"). The guest mounts the volume,
//! reads that file, writes "NEWFILE.DAT" (600 bytes of (i*7+3)&0xFF at
//! timestamp 1700000123), reads it back, then deletes "GUEST.TXT". R1 must
//! be 0 and the resulting image must satisfy the doc/spec_DSFS.md layout:
//! exactly the two new data blocks marked used, no leaks.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu::{CYCLES_PER_FRAME, Cpu};
use cpt32::devices::pec::idc::idc::{Idc, connect_idc};

const SECTOR: usize = 512;
const TOTAL: usize = 2880;
const GUEST_NAME: &[u8; 11] = b"GUEST   TXT";
const NEWFILE_NAME: &[u8; 11] = b"NEWFILE DAT";

fn be32(image: &mut [u8], offset: usize, value: u32) {
    image[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn make_image() -> Vec<u8> {
    let mut image = vec![0u8; TOTAL * SECTOR];
    image[510] = 0x55;
    image[511] = 0xAA;
    // Superblock at LBA 1 (doc/spec_DSFS.md §4).
    let super_offset = SECTOR;
    image[super_offset..super_offset + 4].copy_from_slice(b"DSFS");
    image[super_offset + 4..super_offset + 6].copy_from_slice(&1u16.to_be_bytes());
    be32(&mut image, super_offset + 0x08, TOTAL as u32);
    be32(&mut image, super_offset + 0x0C, 2); // bitmap_start
    be32(&mut image, super_offset + 0x10, 1); // bitmap_blocks
    be32(&mut image, super_offset + 0x14, 3); // dir_start
    be32(&mut image, super_offset + 0x18, 16); // dir_blocks
    be32(&mut image, super_offset + 0x1C, 19); // data_start
    be32(&mut image, super_offset + 0x20, 1_700_000_000);
    // Bitmap at LBA 2: metadata blocks 0..18 plus GUEST's block 19.
    let bitmap = 2 * SECTOR;
    image[bitmap] = 0xFF;
    image[bitmap + 1] = 0xFF;
    image[bitmap + 2] = 0xF0;
    // Directory at LBA 3: one used entry, "GUEST.TXT" at block 19.
    let dir = 3 * SECTOR;
    image[dir] = 1;
    image[dir + 1..dir + 12].copy_from_slice(GUEST_NAME);
    be32(&mut image, dir + 16, 19); // start_block
    be32(&mut image, dir + 20, 10); // size_bytes
    be32(&mut image, dir + 24, 1_700_000_000);
    be32(&mut image, dir + 28, 1_700_000_000);
    image[19 * SECTOR..19 * SECTOR + 10].copy_from_slice(b"Hello DSFS");
    image
}

#[test]
fn dsfs_guest_reads_writes_and_deletes() {
    let root = std::env::temp_dir().join(format!("dsfs-guest-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let image_path = root.join("disk0.img");
    std::fs::write(&image_path, make_image()).unwrap();
    std::fs::write(
        root.join("config.yaml"),
        "disk:\n  - slot: 0\n    type: fdd\n    image: disk0.img\n    read_only: false\n    transfer_rate_bytes_per_second: 48000000\n    access_latency_ms: 0\n",
    )
    .unwrap();

    let mut bus = Bus::new();
    let vdp = connect_devices_with_vdp(&mut bus);
    let drives = Idc::load_drives(&root.join("config.yaml")).unwrap();
    connect_idc(&mut bus, drives);
    let mut cpu = Cpu::new(bus);
    let guest = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/dsfs_guest.bin")).expect(
        "dsfs_guest.bin is missing; build it from tools/dsfs/dsfs_smoke.sol \
         (see the module comment)",
    );
    cpu.load_program(0, &guest);

    for _ in 0..64 {
        if !cpu.is_running() {
            break;
        }
        vdp.borrow_mut().tick();
        cpu.run(CYCLES_PER_FRAME as usize);
    }
    assert!(!cpu.is_running(), "guest did not halt");
    assert_eq!(cpu.read_reg(1), 0, "guest reported a failing step");

    let image = std::fs::read(&image_path).unwrap();
    let dir = 3 * SECTOR;
    // Entry 0 was GUEST.TXT and must be cleared by the guest delete.
    assert!(
        image[dir..dir + ENTRY_SIZE].iter().all(|&b| b == 0),
        "deleted entry was not zeroed"
    );
    // Entry 1 must hold the guest-written file.
    let entry = &image[dir + ENTRY_SIZE..dir + 2 * ENTRY_SIZE];
    assert_eq!(entry[0], 1, "NEWFILE entry not used");
    assert_eq!(&entry[1..12], NEWFILE_NAME);
    let start = u32::from_be_bytes(entry[16..20].try_into().unwrap());
    let size = u32::from_be_bytes(entry[20..24].try_into().unwrap());
    let created = u32::from_be_bytes(entry[24..28].try_into().unwrap());
    let modified = u32::from_be_bytes(entry[28..32].try_into().unwrap());
    assert_eq!((start, size), (20, 600), "NEWFILE geometry");
    assert_eq!((created, modified), (1_700_000_123, 1_700_000_123));
    // Bitmap: metadata + the two NEWFILE blocks used; freed blocks clear.
    let bitmap = 2 * SECTOR;
    assert_eq!(image[bitmap..bitmap + 3], [0xFF, 0xFF, 0xEC], "bitmap");
    assert_eq!(image[bitmap + 3], 0, "bitmap byte 3");
    // Data: GUEST payload survives its delete; NEWFILE holds the pattern
    // with a zero-filled tail in its final sector.
    assert_eq!(&image[19 * SECTOR..19 * SECTOR + 10], b"Hello DSFS");
    let data = &image[20 * SECTOR..22 * SECTOR];
    let expected: Vec<u8> = (0..600).map(|i| (i as u32 * 7 + 3) as u8).collect();
    assert_eq!(&data[..600], &expected[..], "NEWFILE content");
    assert!(data[600..].iter().all(|&b| b == 0), "final-block tail");
    std::fs::remove_dir_all(&root).unwrap();
}

const ENTRY_SIZE: usize = 32;
