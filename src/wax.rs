//! WAX container (spec doc/spec_WAX.md v1): parse + load.

pub const MAGIC: [u8; 4] = [0x57, 0x41, 0x58, 0x00];
pub const VERSION: u16 = 1;
pub const ARCH_SRC32: u16 = 1;
pub const TYPE_WAX: u16 = 1;
pub const TYPE_WAL: u16 = 2;
pub const HEADER_SIZE: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaxImage<'a> {
    pub is_lib: bool,
    pub base: u32,
    pub entry: u32,
    pub text: &'a [u8],
    pub rodata: &'a [u8],
}

fn le16(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn le32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
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
    let (ver, ftype, arch, flags) = (le16(data, 4), le16(data, 6), le16(data, 8), le16(data, 10));
    let (hsz, base, eoff) = (le32(data, 12), le32(data, 16), le32(data, 20));
    let (toff, tsz) = (le32(data, 24), le32(data, 28));
    let (roff, rsz) = (le32(data, 32), le32(data, 36));
    let (doff, dsz) = (le32(data, 40), le32(data, 44));
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
    if flags & !1 != 0 {
        return Err(format!("bad flags {flags:#x}"));
    }
    if tsz == 0 {
        return Err("empty text".into());
    }
    let blobs = [
        ("text", toff, tsz),
        ("rodata", roff, rsz),
        ("debug", doff, dsz),
    ];
    for (name, o, s) in blobs {
        if s == 0 {
            continue;
        }
        if o % 4 != 0 {
            return Err(format!("{name} misaligned"));
        }
        if (o as usize) < HEADER_SIZE || (o as u64) + (s as u64) > data.len() as u64 {
            return Err(format!("{name} out of range"));
        }
    }
    let mut spans: Vec<(u32, u32)> = blobs
        .iter()
        .filter(|&&(_, _, s)| s != 0)
        .map(|&(_, o, s)| (o, o + s))
        .collect();
    spans.sort();
    for w in spans.windows(2) {
        if w[1].0 < w[0].1 {
            return Err("overlapping blobs".into());
        }
    }
    if (dsz != 0) != (flags & 1 != 0) {
        return Err("debug flag/size mismatch".into());
    }
    let is_lib = ftype == TYPE_WAL;
    if is_lib && (base != 0 || eoff != 0) {
        return Err(".wal base/entry must be 0".into());
    }
    let entry = base.wrapping_add(eoff);
    let img_len = (tsz as u64) + (rsz as u64);
    if (base as u64) + img_len > 0x1_0000_0000 {
        return Err("address wrap".into());
    }
    if !is_lib {
        let in_text = entry.wrapping_sub(base) < tsz;
        let in_ro = rsz != 0 && entry.wrapping_sub(base.wrapping_add(tsz)) < rsz;
        if !in_text && !in_ro {
            return Err("entry outside image".into());
        }
    }
    let text = &data[toff as usize..(toff + tsz) as usize];
    let rodata = if rsz == 0 {
        &data[0..0]
    } else {
        &data[roff as usize..(roff + rsz) as usize]
    };
    Ok(WaxImage {
        is_lib,
        base,
        entry,
        text,
        rodata,
    })
}

/// Load data (WAX or legacy raw) into bus. Returns PC.
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
    for (i, b) in img.text.iter().enumerate() {
        bus.write_u8(img.base.wrapping_add(i as u32), *b);
    }
    for (i, b) in img.rodata.iter().enumerate() {
        bus.write_u8(img.base.wrapping_add(img.text.len() as u32 + i as u32), *b);
    }
    Ok(img.entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pack(base: u32, eoff: u32, text: &[u8]) -> Vec<u8> {
        let mut h = vec![0u8; HEADER_SIZE + text.len()];
        h[..4].copy_from_slice(&MAGIC);
        h[4..6].copy_from_slice(&1u16.to_le_bytes());
        h[6..8].copy_from_slice(&1u16.to_le_bytes());
        h[8..10].copy_from_slice(&1u16.to_le_bytes());
        h[12..16].copy_from_slice(&64u32.to_le_bytes());
        h[16..20].copy_from_slice(&base.to_le_bytes());
        h[20..24].copy_from_slice(&eoff.to_le_bytes());
        h[24..28].copy_from_slice(&64u32.to_le_bytes());
        h[28..32].copy_from_slice(&(text.len() as u32).to_le_bytes());
        h[HEADER_SIZE..].copy_from_slice(text);
        h
    }
    #[test]
    fn minimal_loads_at_base() {
        let w = pack(0x100, 0, &[0, 0, 0, 0]);
        let img = parse(&w).unwrap();
        assert_eq!((img.base, img.entry), (0x100, 0x100));
    }
    #[test]
    fn rejects_entry_outside() {
        let w = pack(0x1000, 0x100, &[0; 16]);
        assert!(parse(&w).is_err());
    }
    #[test]
    fn rejects_raw_as_wax() {
        assert!(parse(&[0u8; 64]).is_err());
    }
}
