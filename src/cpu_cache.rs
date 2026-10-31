//! Extension C: split L1 instruction/data cache model (`doc/spec_CPU_cache.md`).
//!
//! The model is tag-only: data always comes from the bus, so the caches are
//! architecturally invisible and cannot go stale. It tracks which lines the
//! RTL caches would hold so that hit/miss counters match the FPGA geometry,
//! and optionally charges a stall per miss (`miss_penalty`, default 0).

use std::cell::RefCell;
use std::rc::Rc;

use crate::bus::Device;

pub const CACHE_REG_BASE: u32 = 0xFFFF_0300;
pub const CACHE_REG_SIZE: u32 = 0x40;
pub const LINE_SHIFT: u32 = 5;
pub const I_SIZE_SHIFT: u32 = 13;
pub const D_SIZE_SHIFT: u32 = 13;
/// CACHE_CTRL bits.
pub const CTRL_I_EN: u32 = 1 << 0;
pub const CTRL_D_EN: u32 = 1 << 1;
pub const CTRL_INV_I: u32 = 1 << 8;
pub const CTRL_INV_D: u32 = 1 << 9;
pub const CTRL_CLR_STATS: u32 = 1 << 10;
/// CACHE_INFO: [3:0] log2 line bytes, [7:4] log2 ways, [15:8] log2 I bytes, [23:16] log2 D bytes.
pub const CACHE_INFO: u32 = LINE_SHIFT | (0 << 4) | (I_SIZE_SHIFT << 8) | (D_SIZE_SHIFT << 16);

const INVALID: u32 = u32::MAX;

#[derive(Debug)]
struct Bank {
    size_shift: u32,
    tags: Vec<u32>,
    enabled: bool,
    hits: u32,
    misses: u32,
}

impl Bank {
    fn new(size_shift: u32) -> Self {
        Self {
            size_shift,
            tags: vec![INVALID; 1 << (size_shift - LINE_SHIFT)],
            enabled: true,
            hits: 0,
            misses: 0,
        }
    }

    fn index_tag(&self, pa: u32) -> (usize, u32) {
        let index = (pa >> LINE_SHIFT) as usize & (self.tags.len() - 1);
        (index, pa >> self.size_shift)
    }

    fn invalidate_all(&mut self) {
        self.tags.fill(INVALID);
    }

    /// Look up and, on a miss, allocate the line. Returns true on a hit.
    fn access(&mut self, pa: u32) -> bool {
        let (index, tag) = self.index_tag(pa);
        if self.tags[index] == tag {
            self.hits = self.hits.wrapping_add(1);
            true
        } else {
            self.tags[index] = tag;
            self.misses = self.misses.wrapping_add(1);
            false
        }
    }

    fn invalidate_line(&mut self, pa: u32) {
        let (index, tag) = self.index_tag(pa);
        if self.tags[index] == tag {
            self.tags[index] = INVALID;
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.invalidate_all();
        }
        self.enabled = enabled;
    }
}

#[derive(Debug)]
pub struct L1Cache {
    i: Bank,
    d: Bank,
    /// Cacheable physical range `[start, end)` (Main RAM).
    ram_start: u32,
    ram_end: u32,
    /// Stall cycles charged per miss (0 keeps the pre-Extension-C timing).
    pub miss_penalty: u32,
}

impl L1Cache {
    pub fn new(ram_start: u32, ram_size: u32) -> Self {
        Self {
            i: Bank::new(I_SIZE_SHIFT),
            d: Bank::new(D_SIZE_SHIFT),
            ram_start,
            ram_end: ram_start.saturating_add(ram_size),
            miss_penalty: 0,
        }
    }

    fn cacheable(&self, pa: u32) -> bool {
        pa >= self.ram_start && pa < self.ram_end
    }

    pub fn reset(&mut self) {
        self.i.invalidate_all();
        self.d.invalidate_all();
        self.i.enabled = true;
        self.d.enabled = true;
        self.clear_stats();
    }

    fn clear_stats(&mut self) {
        self.i.hits = 0;
        self.i.misses = 0;
        self.d.hits = 0;
        self.d.misses = 0;
    }

    /// Instruction fetch of the word/halfword at `pa`. Returns stall cycles.
    pub fn fetch(&mut self, pa: u32) -> u32 {
        if !self.i.enabled || !self.cacheable(pa) {
            return 0;
        }
        if self.i.access(pa) {
            0
        } else {
            self.miss_penalty
        }
    }

    /// Data load. Returns stall cycles.
    pub fn load(&mut self, pa: u32) -> u32 {
        if !self.d.enabled || !self.cacheable(pa) {
            return 0;
        }
        if self.d.access(pa) {
            0
        } else {
            self.miss_penalty
        }
    }

    /// Write-through store: no allocation, snoops the instruction cache.
    pub fn store(&mut self, pa: u32) {
        if self.cacheable(pa) {
            self.i.invalidate_line(pa);
        }
    }

    /// Another bus master (DMA) wrote `[pa, pa+len)`.
    pub fn external_write(&mut self, pa: u32, len: u32) {
        let mut line = pa & !((1 << LINE_SHIFT) - 1);
        let end = pa.saturating_add(len);
        while line < end {
            self.i.invalidate_line(line);
            self.d.invalidate_line(line);
            line = line.wrapping_add(1 << LINE_SHIFT);
            if line == 0 {
                break;
            }
        }
    }

    pub fn ctrl(&self) -> u32 {
        u32::from(self.i.enabled) * CTRL_I_EN | u32::from(self.d.enabled) * CTRL_D_EN
    }

    pub fn read_reg(&self, offset: u32) -> u32 {
        match offset {
            0x00 => CACHE_INFO,
            0x04 => self.ctrl(),
            0x10 => self.i.hits,
            0x14 => self.i.misses,
            0x18 => self.d.hits,
            0x1C => self.d.misses,
            _ => 0,
        }
    }

    pub fn write_reg(&mut self, offset: u32, value: u32) {
        if offset != 0x04 {
            return;
        }
        self.i.set_enabled(value & CTRL_I_EN != 0);
        self.d.set_enabled(value & CTRL_D_EN != 0);
        if value & CTRL_INV_I != 0 {
            self.i.invalidate_all();
        }
        if value & CTRL_INV_D != 0 {
            self.d.invalidate_all();
        }
        if value & CTRL_CLR_STATS != 0 {
            self.clear_stats();
        }
    }
}

/// CPU-reserved MMIO window for the cache registers (big-endian words).
struct CacheRegisterDevice(Rc<RefCell<L1Cache>>);

impl CacheRegisterDevice {
    fn lane(addr: u32) -> (u32, u32) {
        (addr & !3, 8 * (3 - (addr & 3)))
    }
}

impl Device for CacheRegisterDevice {
    fn debug_read(&mut self, addr: u32) -> Option<u8> {
        let (word, shift) = Self::lane(addr);
        Some((self.0.borrow().read_reg(word) >> shift) as u8)
    }
    fn read(&mut self, addr: u32) -> u8 {
        let (word, shift) = Self::lane(addr);
        (self.0.borrow().read_reg(word) >> shift) as u8
    }
    fn write(&mut self, addr: u32, value: u8) {
        let (word, shift) = Self::lane(addr);
        let mut cache = self.0.borrow_mut();
        // Only CACHE_CTRL is writable; merge the byte into its current value
        // (strobe bits read as zero).
        let old = cache.read_reg(word);
        let merged = (old & !(0xFF << shift)) | (u32::from(value) << shift);
        cache.write_reg(word, merged);
    }
    fn size(&self) -> u32 {
        CACHE_REG_SIZE
    }
}

pub fn connect_cache(bus: &mut crate::bus::Bus) -> Rc<RefCell<L1Cache>> {
    // Without Main RAM the caches have nothing to cache but the registers stay.
    let (start, size) = bus
        .memory_map()
        .find(|region| region.name == "Main RAM")
        .map_or((0, 0), |region| (region.start, region.size));
    let cache = Rc::new(RefCell::new(L1Cache::new(start, size)));
    bus.add_device(
        CACHE_REG_BASE,
        "L1 Cache Registers",
        Box::new(CacheRegisterDevice(Rc::clone(&cache))),
    );
    cache
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> L1Cache {
        L1Cache::new(0, 16 << 20)
    }

    #[test]
    fn sequential_fetch_hits_within_a_line() {
        let mut c = cache();
        for i in 0..8 {
            c.fetch(0x1000 + i * 4);
        }
        assert_eq!((c.i.hits, c.i.misses), (7, 1));
    }

    #[test]
    fn conflicting_lines_evict_each_other() {
        let mut c = cache();
        c.load(0x0000);
        c.load(0x2000); // same index, different tag (8 KiB direct mapped)
        c.load(0x0000);
        assert_eq!((c.d.hits, c.d.misses), (0, 3));
    }

    #[test]
    fn store_invalidates_instruction_line_but_not_data_line() {
        let mut c = cache();
        c.fetch(0x100);
        c.load(0x100);
        c.store(0x104);
        c.fetch(0x100);
        c.load(0x100);
        assert_eq!((c.i.hits, c.i.misses), (0, 2));
        assert_eq!((c.d.hits, c.d.misses), (1, 1));
    }

    #[test]
    fn mmio_and_disabled_caches_are_bypassed() {
        let mut c = cache();
        c.load(0x8004_0000);
        assert_eq!(c.d.misses, 0);
        c.write_reg(0x04, CTRL_I_EN);
        c.load(0x100);
        assert_eq!(c.d.misses, 0);
        c.write_reg(0x04, CTRL_I_EN | CTRL_D_EN);
        c.load(0x100);
        c.load(0x100);
        assert_eq!((c.d.hits, c.d.misses), (1, 1));
    }

    #[test]
    fn external_write_and_invalidate_strobes() {
        let mut c = cache();
        c.load(0x200);
        c.external_write(0x210, 4);
        c.load(0x200);
        assert_eq!(c.d.misses, 2);
        c.write_reg(0x04, CTRL_I_EN | CTRL_D_EN | CTRL_INV_D);
        c.load(0x200);
        assert_eq!(c.d.misses, 3);
        c.write_reg(0x04, CTRL_I_EN | CTRL_D_EN | CTRL_CLR_STATS);
        assert_eq!(c.read_reg(0x18) + c.read_reg(0x1C), 0);
        assert_eq!(c.read_reg(0x04), 3);
    }
}
