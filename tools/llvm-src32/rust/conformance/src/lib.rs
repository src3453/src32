//! Lowerings rustc relies on (tools/llvm-src32/test_rust.py runs this with
//! opt-level 0 and 2): saturating float-to-int `as` casts (llvm.fpto[su]i.sat),
//! three-way comparisons in every width (llvm.[su]cmp), the black_box barrier
//! (an empty inline asm), overflow-checked arithmetic (llvm.*.with.overflow) and
//! panics. `main` returns 42, or the failing check.
#![no_std]

use core::cmp::Ordering;
use core::hint::black_box;

// Link cpt32 for its #[panic_handler] even though nothing else is used.
use cpt32 as _;

fn casts() -> i32 {
    let nan = black_box(f32::NAN);
    let big = black_box(3.0e9f32);
    let neg = black_box(-1.0e20f64);
    let frac = black_box(-7.9f64);
    if nan as i32 != 0 || nan as u8 != 0 || (black_box(f64::NAN) as u64) != 0 {
        return 1;
    }
    if big as i32 != i32::MAX
        || big as u32 != 3_000_000_000
        || big as i16 != i16::MAX
        || big as u8 != 255
    {
        return 2;
    }
    if neg as i32 != i32::MIN
        || neg as u32 != 0
        || neg as i8 != i8::MIN
        || neg as i64 != i64::MIN
        || neg as u64 != 0
    {
        return 3;
    }
    if frac as i32 != -7
        || frac as i8 != -7
        || frac as u16 != 0
        || (black_box(300.5f32) as u8) != 255
    {
        return 4;
    }
    if black_box(f64::INFINITY) as i64 != i64::MAX || black_box(1.0e30f32) as u64 != u64::MAX {
        return 5;
    }
    if black_box(-129.0f32) as i8 != -128
        || black_box(127.9f64) as i8 != 127
        || black_box(65535.9f32) as u16 != 65535
    {
        return 6;
    }
    if black_box(9.0e15f64) as i64 != 9_000_000_000_000_000
        || black_box(4294967295.5f64) as u32 != u32::MAX
    {
        return 7;
    }
    0
}

fn compares() -> i32 {
    let (a, b) = (black_box(-3i8), black_box(5i8));
    if a.cmp(&b) != Ordering::Less || (a as u8).cmp(&(b as u8)) != Ordering::Greater {
        return 11;
    }
    let (c, d) = (black_box(-70000i32), black_box(70000i32));
    if c.cmp(&d) != Ordering::Less
        || (c as u32).cmp(&(d as u32)) != Ordering::Greater
        || c.cmp(&c) != Ordering::Equal
    {
        return 12;
    }
    let (e, f) = (black_box(-1i64), black_box(0x1_0000_0000i64));
    if e.cmp(&f) != Ordering::Less || (e as u64).cmp(&(f as u64)) != Ordering::Greater {
        return 13;
    }
    // Equal high words: the low words decide, unsigned.
    let (g, h) = (black_box(0x7_8000_0000u64), black_box(0x7_0000_0001u64));
    if g.cmp(&h) != Ordering::Greater
        || (g as i64).cmp(&(h as i64)) != Ordering::Greater
        || g.cmp(&g) != Ordering::Equal
    {
        return 14;
    }
    let (i, j) = (black_box(-2i16), black_box(-2i16));
    if i.cmp(&j) != Ordering::Equal || black_box(true).cmp(&black_box(false)) != Ordering::Greater {
        return 15;
    }
    let mut values = black_box([5u64, 1 << 40, 3, u64::MAX, 0]);
    values.sort_unstable();
    if values != [0, 3, 5, 1 << 40, u64::MAX] {
        return 16;
    }
    0
}

fn overflow() -> i32 {
    let (a8, b8) = (black_box(100i8), black_box(27i8));
    if a8.checked_add(b8) != Some(127)
        || a8.checked_add(b8 + 1).is_some()
        || (-a8).checked_sub(b8 + 2).is_some()
    {
        return 21;
    }
    if black_box(200u8).checked_add(55) != Some(255)
        || black_box(200u8).checked_add(56).is_some()
        || black_box(3u8).checked_sub(4).is_some()
        || black_box(16u8).checked_mul(15) != Some(240)
        || black_box(16u8).checked_mul(16).is_some()
    {
        return 22;
    }
    if black_box(-128i8).checked_mul(-1).is_some()
        || black_box(-64i8).checked_mul(2) != Some(-128)
        || black_box(181i16).checked_mul(181) != Some(32761)
        || black_box(182i16).checked_mul(181).is_some()
    {
        return 23;
    }
    if black_box(65535u32).checked_mul(65537) != Some(u32::MAX)
        || black_box(65536u32).checked_mul(65536).is_some()
        || black_box(i32::MIN).checked_sub(1).is_some()
        || black_box(i32::MAX).checked_add(1).is_some()
        || black_box(-46341i32).checked_mul(46341).is_some()
        || black_box(-46340i32).checked_mul(46340) != Some(-2147395600)
        || black_box(u32::MAX).checked_add(1).is_some()
        || black_box(0u32).checked_sub(1).is_some()
    {
        return 24;
    }
    if black_box(u64::MAX - 1).checked_add(1) != Some(u64::MAX)
        || black_box(u64::MAX).checked_add(1).is_some()
        || black_box(0xFFFF_FFFFu64).checked_add(1) != Some(0x1_0000_0000)
        || black_box(0x1_0000_0000u64).checked_sub(1) != Some(0xFFFF_FFFF)
        || black_box(5u64).checked_sub(6).is_some()
    {
        return 25;
    }
    if black_box(i64::MAX).checked_add(1).is_some()
        || black_box(i64::MIN).checked_sub(1).is_some()
        || black_box(-1i64).checked_add(-0x7FFF_FFFF_FFFF_FFFF) != Some(i64::MIN)
        || black_box(i64::MIN + 1).checked_sub(1) != Some(i64::MIN)
    {
        return 26;
    }
    // Debug builds check plain operators the same way.
    let (x, y) = (black_box(40000u16), black_box(20000u16));
    if x - y != 20000 || x.wrapping_add(y) != 60000 || x.overflowing_add(y + 10000) != (4464, true)
    {
        return 27;
    }
    0
}

static mut PANIC_INDEX: usize = 7;

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    // With the `panic` feature: index out of bounds, reported by cpt32's
    // handler on the UART ("panic at <file>:<line>") before it halts.
    if cfg!(feature = "panic") {
        let table = black_box([1u8, 2, 3]);
        let index = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(PANIC_INDEX)) };
        return table[index] as i32;
    }
    let result = casts();
    if result != 0 {
        return result;
    }
    let result = compares();
    if result != 0 {
        return result;
    }
    let result = overflow();
    if result != 0 {
        return result;
    }
    42
}
