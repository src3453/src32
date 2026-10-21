//! WAX v2 sparse block container (spec doc/spec_WAX.md): parse + load.

pub const MAGIC: [u8; 4] = [0x57, 0x41, 0x58, 0x00];
pub const VERSION: u16 = 2;
pub const ARCH_SRC32: u16 = 1;
pub const TYPE_WAX: u16 = 1;
pub const TYPE_WAL: u16 = 2;
pub const HEADER_SIZE: usize = 64;
pub const FLAG_DEBUG: u16 = 1 << 0;
/// The executable runs in Extension P user mode (doc/spec_WAX.md).
pub const FLAG_PROTECTED: u16 = 1 << 1;
pub const BLOCK_ENTRY_SIZE: u32 = 12;
pub const PROTECTED_BLOCK_ENTRY_SIZE: u32 = 16;
/// Region 0 holds the user stack, so a protected image has at most 7 blocks.
pub const MAX_PROTECTED_BLOCKS: usize = 7;
/// Block permission bits, equal to Extension P region FLAGS.
pub const PERM_R: u32 = 1;
pub const PERM_W: u32 = 2;
pub const PERM_X: u32 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaxBlock<'a> {
    pub load_address: u32,
    pub data: &'a [u8],
    /// R/W/X bits from a protected image; 0 in an unprotected image.
    pub perm: u32,
}

/// User-mode start state requested by a protected `.wax`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaxProtection {
    pub stack_top: u32,
    pub stack_size: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaxImage<'a> {
    pub is_lib: bool,
    pub base: u32,
    pub entry: u32,
    pub blocks: Vec<WaxBlock<'a>>,
    pub debug: &'a [u8],
    pub protection: Option<WaxProtection>,
}

/// One Extension P protection region (`RGNn_START`, `RGNn_END`, `RGNn_FLAGS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserRegion {
    pub start: u32,
    pub end: u32,
    pub flags: u32,
}

impl WaxImage<'_> {
    /// Regions a loader programs before entering user mode: region 0 is the
    /// stack (R/W), regions 1.. are the blocks in table order. `None` for an
    /// unprotected image.
    pub fn user_regions(&self) -> Option<Vec<UserRegion>> {
        let protection = self.protection?;
        let stack = UserRegion {
            start: protection.stack_top - protection.stack_size,
            end: protection.stack_top,
            flags: PERM_R | PERM_W,
        };
        let blocks = self.blocks.iter().map(|block| UserRegion {
            start: block.load_address,
            end: block.load_address + block.data.len() as u32,
            flags: block.perm,
        });
        Some(std::iter::once(stack).chain(blocks).collect())
    }
}

fn be16(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([d[o], d[o + 1]])
}
fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

pub fn is_wax(data: &[u8]) -> bool {
    data.len() >= 4 && data[..4] == MAGIC
}

pub fn parse(data: &[u8]) -> Result<WaxImage<'_>, String> {
    if data.len() < HEADER_SIZE {
        return Err("too small for WAX header".into());
    }
    if data[..4] != MAGIC {
        return Err("bad magic (not WAX)".into());
    }
    let (ver, ftype, arch, flags) = (be16(data, 4), be16(data, 6), be16(data, 8), be16(data, 10));
    let hsz = be32(data, 12);
    let base = be32(data, 16);
    let entry_offset = be32(data, 20);
    let table_offset = be32(data, 24);
    let block_count = be32(data, 28);
    let block_entry_size = be32(data, 32);
    let debug_offset = be32(data, 36);
    let debug_size = be32(data, 40);
    let file_size = be32(data, 44);
    if ver != VERSION {
        return Err(format!("unsupported version {ver}"));
    }
    if ftype != TYPE_WAX && ftype != TYPE_WAL {
        return Err(format!("bad type {ftype}"));
    }
    if arch != ARCH_SRC32 {
        return Err(format!("bad arch {arch}"));
    }
    if hsz as usize != HEADER_SIZE {
        return Err(format!("bad header_size {hsz}"));
    }
    if flags & !(FLAG_DEBUG | FLAG_PROTECTED) != 0 {
        return Err(format!("bad flags {flags:#x}"));
    }
    let protected = flags & FLAG_PROTECTED != 0;
    if protected && ftype != TYPE_WAX {
        return Err("PROTECTED is only valid for .wax executables".into());
    }
    // Protected images use 48..56 for the user stack; the rest is reserved.
    let reserved_start = if protected { 56 } else { 48 };
    if data[reserved_start..HEADER_SIZE]
        .iter()
        .any(|byte| *byte != 0)
    {
        return Err("reserved header bytes must be 0".into());
    }
    if u64::from(file_size) != data.len() as u64 {
        return Err("file_size mismatch".into());
    }
    if block_count == 0 {
        return Err("empty block table".into());
    }
    let expected_entry_size = if protected {
        PROTECTED_BLOCK_ENTRY_SIZE
    } else {
        BLOCK_ENTRY_SIZE
    };
    if block_entry_size != expected_entry_size {
        return Err(format!("bad block_entry_size {block_entry_size}"));
    }
    if protected && block_count as usize > MAX_PROTECTED_BLOCKS {
        return Err(format!(
            "protected WAX allows at most {MAX_PROTECTED_BLOCKS} blocks"
        ));
    }
    if table_offset % 4 != 0 || (table_offset as usize) < HEADER_SIZE {
        return Err("block table misaligned or inside header".into());
    }
    let table_start = u64::from(table_offset);
    let table_end = table_start + u64::from(block_count) * u64::from(block_entry_size);
    if table_end > u64::from(file_size) {
        return Err("block table out of range".into());
    }

    let mut blocks = Vec::with_capacity(block_count as usize);
    let mut file_spans = Vec::with_capacity(block_count as usize + 2);
    file_spans.push((table_start, table_end));
    let mut previous_load_end = None;
    for index in 0..block_count as usize {
        let offset = table_offset as usize + index * block_entry_size as usize;
        let load_address = be32(data, offset);
        let file_offset = be32(data, offset + 4);
        let size = be32(data, offset + 8);
        if size == 0 {
            return Err("empty data block".into());
        }
        let perm = if protected {
            be32(data, offset + 12)
        } else {
            0
        };
        if protected && (perm == 0 || perm & !(PERM_R | PERM_W | PERM_X) != 0) {
            return Err("protected block permission must be a nonzero R/W/X mask".into());
        }
        let load_end = u64::from(load_address) + u64::from(size);
        if load_end > 0x1_0000_0000 {
            return Err("block address range wraps u32".into());
        }
        if previous_load_end.is_some_and(|end| u64::from(load_address) < end) {
            return Err("blocks overlap or are not address-sorted".into());
        }
        previous_load_end = Some(load_end);
        if file_offset % 4 != 0 {
            return Err("block payload misaligned".into());
        }
        let file_start = u64::from(file_offset);
        let file_end = file_start + u64::from(size);
        if file_start < table_end || file_end > u64::from(file_size) {
            return Err("block payload out of range".into());
        }
        file_spans.push((file_start, file_end));
        blocks.push(WaxBlock {
            load_address,
            data: &data[file_offset as usize..file_end as usize],
            perm,
        });
    }

    if (debug_size != 0) != (flags & FLAG_DEBUG != 0) {
        return Err("debug flag/size mismatch".into());
    }
    let debug = if debug_size == 0 {
        if debug_offset != 0 {
            return Err("debug_offset must be 0 when debug is absent".into());
        }
        &data[0..0]
    } else {
        if debug_offset % 4 != 0 || u64::from(debug_offset) < table_end {
            return Err("debug block misaligned or overlaps metadata".into());
        }
        let debug_end = u64::from(debug_offset) + u64::from(debug_size);
        if debug_end > u64::from(file_size) {
            return Err("debug block out of range".into());
        }
        file_spans.push((u64::from(debug_offset), debug_end));
        &data[debug_offset as usize..debug_end as usize]
    };
    file_spans.sort_unstable();
    if file_spans.windows(2).any(|spans| spans[1].0 < spans[0].1) {
        return Err("overlapping file blocks".into());
    }

    let is_lib = ftype == TYPE_WAL;
    if is_lib && (base != 0 || entry_offset != 0) {
        return Err(".wal base/entry must be 0".into());
    }
    if !is_lib && base != blocks[0].load_address {
        return Err("base must equal the lowest WAX block address".into());
    }
    let entry = u64::from(base) + u64::from(entry_offset);
    if entry > u64::from(u32::MAX) {
        return Err("entry address wraps u32".into());
    }
    if !is_lib
        && !blocks.iter().any(|block| {
            u64::from(block.load_address) <= entry
                && entry < u64::from(block.load_address) + block.data.len() as u64
        })
    {
        return Err("entry outside image blocks".into());
    }
    let protection = if protected {
        let protection = WaxProtection {
            stack_top: be32(data, 48),
            stack_size: be32(data, 52),
        };
        validate_protection(protection, &blocks, entry as u32)?;
        Some(protection)
    } else {
        None
    };

    Ok(WaxImage {
        is_lib,
        base,
        entry: entry as u32,
        blocks,
        debug,
        protection,
    })
}

fn validate_protection(
    protection: WaxProtection,
    blocks: &[WaxBlock<'_>],
    entry: u32,
) -> Result<(), String> {
    let WaxProtection {
        stack_top,
        stack_size,
    } = protection;
    if stack_size == 0 || stack_size % 4 != 0 || stack_top % 4 != 0 {
        return Err("protected stack top/size must be nonzero multiples of 4".into());
    }
    if stack_top > crate::cpu_privilege::CPU_RESERVED_BASE || stack_size > stack_top {
        return Err("protected stack lies outside user-addressable memory".into());
    }
    let stack_start = stack_top - stack_size;
    let overlaps = |block: &WaxBlock<'_>| {
        let end = u64::from(block.load_address) + block.data.len() as u64;
        u64::from(stack_start) < end && u64::from(block.load_address) < u64::from(stack_top)
    };
    if blocks.iter().any(overlaps) {
        return Err("protected stack overlaps an image block".into());
    }
    if blocks.iter().any(|block| {
        u64::from(block.load_address) + block.data.len() as u64
            > u64::from(crate::cpu_privilege::CPU_RESERVED_BASE)
    }) {
        return Err("protected block reaches the CPU-reserved area".into());
    }
    let entry_block = blocks.iter().find(|block| {
        block.load_address <= entry
            && u64::from(entry) < u64::from(block.load_address) + block.data.len() as u64
    });
    if entry_block.is_none_or(|block| block.perm & PERM_X == 0) {
        return Err("protected entry must lie in an executable block".into());
    }
    Ok(())
}

/// Host loaders start programs in supervisor mode. Entering user mode is the
/// BIOS WAX loader's job (doc/spec_WAX.md), so a protected image is refused
/// here instead of silently running unprotected.
pub fn reject_protected_direct_load(image: &WaxImage<'_>) -> Result<(), String> {
    if image.protection.is_some() {
        return Err(
            "protected WAX must be started by the BIOS WAX loader; direct host loading runs in supervisor mode only"
                .into(),
        );
    }
    Ok(())
}

/// Load data (WAX v2 or legacy raw) into bus. Returns PC.
pub fn load_bytes(bus: &mut crate::bus::Bus, data: &[u8]) -> Result<u32, String> {
    if !is_wax(data) {
        for (i, b) in data.iter().enumerate() {
            bus.write_u8(i as u32, *b);
        }
        return Ok(0);
    }
    let img = parse(data)?;
    if img.is_lib {
        return Err("cannot execute .wal library".into());
    }
    reject_protected_direct_load(&img)?;
    for block in &img.blocks {
        for (index, byte) in block.data.iter().enumerate() {
            bus.write_u8(block.load_address.wrapping_add(index as u32), *byte);
        }
    }
    Ok(img.entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;
    use crate::devices::ram::connect_ram;

    fn pack(base: u32, entry_offset: u32, blocks: &[(u32, &[u8])]) -> Vec<u8> {
        let table_end = HEADER_SIZE + blocks.len() * 12;
        let mut file_size = table_end;
        let mut records = Vec::new();
        for &(address, payload) in blocks {
            file_size = (file_size + 3) & !3;
            records.push((address, file_size, payload));
            file_size += payload.len();
        }
        let mut bytes = vec![0; file_size];
        bytes[..4].copy_from_slice(&MAGIC);
        bytes[4..6].copy_from_slice(&VERSION.to_be_bytes());
        bytes[6..8].copy_from_slice(&TYPE_WAX.to_be_bytes());
        bytes[8..10].copy_from_slice(&ARCH_SRC32.to_be_bytes());
        bytes[12..16].copy_from_slice(&(HEADER_SIZE as u32).to_be_bytes());
        bytes[16..20].copy_from_slice(&base.to_be_bytes());
        bytes[20..24].copy_from_slice(&entry_offset.to_be_bytes());
        bytes[24..28].copy_from_slice(&(HEADER_SIZE as u32).to_be_bytes());
        bytes[28..32].copy_from_slice(&(blocks.len() as u32).to_be_bytes());
        bytes[32..36].copy_from_slice(&12u32.to_be_bytes());
        bytes[44..48].copy_from_slice(&(file_size as u32).to_be_bytes());
        for (index, (address, file_offset, payload)) in records.into_iter().enumerate() {
            let record = HEADER_SIZE + index * 12;
            bytes[record..record + 4].copy_from_slice(&address.to_be_bytes());
            bytes[record + 4..record + 8].copy_from_slice(&(file_offset as u32).to_be_bytes());
            bytes[record + 8..record + 12].copy_from_slice(&(payload.len() as u32).to_be_bytes());
            bytes[file_offset..file_offset + payload.len()].copy_from_slice(payload);
        }
        bytes
    }

    #[test]
    fn header_fields_are_big_endian_and_blocks_parse() {
        let w = pack(
            0x1234_5678,
            0,
            &[
                (0x1234_5678, &[0, 0, 0, 0]),
                (0x1235_0000, &[0xfc, 0, 0, 0]),
            ],
        );
        assert_eq!(&w[4..6], &[0, 2]);
        assert_eq!(&w[12..16], &[0, 0, 0, 64]);
        assert_eq!(&w[16..20], &[0x12, 0x34, 0x56, 0x78]);
        assert_eq!(&w[24..28], &[0, 0, 0, 64]);
        assert_eq!(&w[28..32], &[0, 0, 0, 2]);
        let image = parse(&w).unwrap();
        assert_eq!(image.entry, 0x1234_5678);
        assert_eq!(image.blocks.len(), 2);
        assert_eq!(image.blocks[1].load_address, 0x1235_0000);
        assert_eq!(image.blocks[1].data, &[0xfc, 0, 0, 0]);
    }

    #[test]
    fn load_bytes_writes_sparse_blocks_and_preserves_gaps() {
        let w = pack(0x100, 0, &[(0x100, &[1, 2, 3, 4]), (0x1000, &[5, 6, 7, 8])]);
        let mut bus = Bus::new();
        connect_ram(&mut bus);
        bus.write_u8(0x800, 0xaa);
        assert_eq!(load_bytes(&mut bus, &w).unwrap(), 0x100);
        assert_eq!(bus.read_u8(0x100), 1);
        assert_eq!(bus.read_u8(0x103), 4);
        assert_eq!(bus.read_u8(0x1000), 5);
        assert_eq!(bus.read_u8(0x1003), 8);
        assert_eq!(bus.read_u8(0x800), 0xaa);
    }

    #[test]
    fn rejects_entry_in_sparse_gap() {
        let w = pack(0x100, 0x800, &[(0x100, &[0; 4]), (0x1000, &[0; 4])]);
        assert!(parse(&w).is_err());
    }

    #[test]
    fn rejects_overlapping_or_out_of_range_blocks() {
        let mut overlapping = pack(0x100, 0, &[(0x100, &[1, 2, 3, 4]), (0x200, &[5, 6, 7, 8])]);
        overlapping[HEADER_SIZE + 12..HEADER_SIZE + 16].copy_from_slice(&0x102u32.to_be_bytes());
        assert!(parse(&overlapping).is_err());

        let mut out_of_range = pack(0x100, 0, &[(0x100, &[1, 2, 3, 4])]);
        out_of_range[HEADER_SIZE + 4..HEADER_SIZE + 8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse(&out_of_range).is_err());
    }

    /// Protected image: 16-byte block entries with permissions, stack in 48..56.
    fn pack_protected(
        entry_offset: u32,
        blocks: &[(u32, &[u8], u32)],
        stack: (u32, u32),
    ) -> Vec<u8> {
        let table_end = HEADER_SIZE + blocks.len() * 16;
        let mut file_size = table_end;
        let mut records = Vec::new();
        for &(address, payload, perm) in blocks {
            file_size = (file_size + 3) & !3;
            records.push((address, file_size, payload, perm));
            file_size += payload.len();
        }
        let mut bytes = vec![0; file_size];
        bytes[..4].copy_from_slice(&MAGIC);
        bytes[4..6].copy_from_slice(&VERSION.to_be_bytes());
        bytes[6..8].copy_from_slice(&TYPE_WAX.to_be_bytes());
        bytes[8..10].copy_from_slice(&ARCH_SRC32.to_be_bytes());
        bytes[10..12].copy_from_slice(&FLAG_PROTECTED.to_be_bytes());
        bytes[12..16].copy_from_slice(&(HEADER_SIZE as u32).to_be_bytes());
        bytes[16..20].copy_from_slice(&blocks[0].0.to_be_bytes());
        bytes[20..24].copy_from_slice(&entry_offset.to_be_bytes());
        bytes[24..28].copy_from_slice(&(HEADER_SIZE as u32).to_be_bytes());
        bytes[28..32].copy_from_slice(&(blocks.len() as u32).to_be_bytes());
        bytes[32..36].copy_from_slice(&16u32.to_be_bytes());
        bytes[44..48].copy_from_slice(&(file_size as u32).to_be_bytes());
        bytes[48..52].copy_from_slice(&stack.0.to_be_bytes());
        bytes[52..56].copy_from_slice(&stack.1.to_be_bytes());
        for (index, (address, file_offset, payload, perm)) in records.into_iter().enumerate() {
            let record = HEADER_SIZE + index * 16;
            bytes[record..record + 4].copy_from_slice(&address.to_be_bytes());
            bytes[record + 4..record + 8].copy_from_slice(&(file_offset as u32).to_be_bytes());
            bytes[record + 8..record + 12].copy_from_slice(&(payload.len() as u32).to_be_bytes());
            bytes[record + 12..record + 16].copy_from_slice(&perm.to_be_bytes());
            bytes[file_offset..file_offset + payload.len()].copy_from_slice(payload);
        }
        bytes
    }

    #[test]
    fn protected_image_describes_user_regions() {
        let w = pack_protected(
            0,
            &[
                (0x10_0000, &[0; 8], PERM_R | PERM_X),
                (0x11_0000, &[0; 4], PERM_R | PERM_W),
            ],
            (0x20_0000, 0x1000),
        );
        let image = parse(&w).unwrap();
        assert_eq!(
            image.protection,
            Some(WaxProtection {
                stack_top: 0x20_0000,
                stack_size: 0x1000
            })
        );
        assert_eq!(
            image.user_regions().unwrap(),
            vec![
                UserRegion {
                    start: 0x1F_F000,
                    end: 0x20_0000,
                    flags: PERM_R | PERM_W
                },
                UserRegion {
                    start: 0x10_0000,
                    end: 0x10_0008,
                    flags: PERM_R | PERM_X
                },
                UserRegion {
                    start: 0x11_0000,
                    end: 0x11_0004,
                    flags: PERM_R | PERM_W
                },
            ]
        );
        let mut bus = Bus::new();
        connect_ram(&mut bus);
        assert!(
            load_bytes(&mut bus, &w)
                .unwrap_err()
                .contains("BIOS WAX loader")
        );
    }

    #[test]
    fn protected_image_rejects_bad_permissions_stack_and_entry() {
        let stack = (0x20_0000, 0x1000);
        let code: &[u8] = &[0; 8];
        // Entry block without X.
        assert!(parse(&pack_protected(0, &[(0x10_0000, code, PERM_R)], stack)).is_err());
        // Permission outside R/W/X, or zero.
        assert!(parse(&pack_protected(0, &[(0x10_0000, code, 8 | PERM_X)], stack)).is_err());
        assert!(parse(&pack_protected(0, &[(0x10_0000, code, 0)], stack)).is_err());
        // Stack overlapping code, misaligned, empty, or in the CPU-reserved area.
        assert!(
            parse(&pack_protected(
                0,
                &[(0x10_0000, code, PERM_X)],
                (0x10_0004, 4)
            ))
            .is_err()
        );
        assert!(
            parse(&pack_protected(
                0,
                &[(0x10_0000, code, PERM_X)],
                (0x20_0002, 4)
            ))
            .is_err()
        );
        assert!(
            parse(&pack_protected(
                0,
                &[(0x10_0000, code, PERM_X)],
                (0x20_0000, 0)
            ))
            .is_err()
        );
        assert!(
            parse(&pack_protected(
                0,
                &[(0x10_0000, code, PERM_X)],
                (0xFFFF_1000, 4)
            ))
            .is_err()
        );
        // Eight blocks exceed the region budget.
        let blocks: Vec<(u32, &[u8], u32)> = (0..8)
            .map(|index| (0x10_0000 + index * 0x100, code, PERM_X))
            .collect();
        assert!(parse(&pack_protected(0, &blocks, stack)).is_err());
        // Unprotected entries keep the 12-byte table.
        let mut wrong_size = pack_protected(0, &[(0x10_0000, code, PERM_X)], stack);
        wrong_size[10..12].fill(0);
        assert!(parse(&wrong_size).is_err());
    }

    #[test]
    fn rejects_old_version_and_raw_data() {
        let mut old_version = pack(0x100, 0, &[(0x100, &[0, 0, 0, 0])]);
        old_version[4..6].copy_from_slice(&1u16.to_be_bytes());
        assert!(parse(&old_version).is_err());
        let mut reserved = pack(0x100, 0, &[(0x100, &[0, 0, 0, 0])]);
        reserved[48..64].fill(0);
        reserved[63] = 1;
        assert!(parse(&reserved).is_err());
        assert!(parse(&[0u8; 64]).is_err());
    }
}
