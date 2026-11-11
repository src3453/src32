//! Volatile device register access (`<cpt32/mmio.h>`). SRC32 is big-endian.

#[inline(always)]
pub fn read8(address: u32) -> u8 {
    unsafe { core::ptr::read_volatile(address as *const u8) }
}

#[inline(always)]
pub fn write8(address: u32, value: u8) {
    unsafe { core::ptr::write_volatile(address as *mut u8, value) }
}

#[inline(always)]
pub fn read16(address: u32) -> u16 {
    unsafe { core::ptr::read_volatile(address as *const u16) }
}

#[inline(always)]
pub fn write16(address: u32, value: u16) {
    unsafe { core::ptr::write_volatile(address as *mut u16, value) }
}

#[inline(always)]
pub fn read32(address: u32) -> u32 {
    unsafe { core::ptr::read_volatile(address as *const u32) }
}

#[inline(always)]
pub fn write32(address: u32, value: u32) {
    unsafe { core::ptr::write_volatile(address as *mut u32, value) }
}
