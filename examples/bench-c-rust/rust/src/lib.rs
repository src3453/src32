//! The benchmark kernels in Rust, the same algorithms as `../c/kernels.c`
//! (see `../README.md`). The entry points have the C signature of
//! `../harness/bench.h`; the shared C harness calls them. Inside, the code is
//! ordinary safe Rust over slices (with bounds checks): only turning the raw
//! buffer into a slice is `unsafe`. Each kernel lives in a `bench_<kernel>`
//! module so that run.py can attribute its code size.
//!
//! The feature `unchecked` (run.py's "Rust unchecked" variant) turns every
//! slice index of the kernels, which go through [`at`], [`set`] and [`swap`],
//! into an unchecked access, to measure what the bounds checks cost. Its
//! soundness then rests on the algorithms staying in bounds; it is for
//! measurement only.
#![no_std]

use cpt32 as _; // panic handler

/// The words of the harness buffer that a kernel may use (`BUFFER_WORDS`).
const BUFFER_WORDS: usize = 8192;

fn words<'a>(buffer: *mut u32, count: usize) -> &'a mut [u32] {
    assert!(count <= BUFFER_WORDS);
    // SAFETY: the harness passes its static buffer of BUFFER_WORDS words.
    unsafe { core::slice::from_raw_parts_mut(buffer, count) }
}

fn bytes<'a>(buffer: *mut u32, count: usize) -> &'a mut [u8] {
    assert!(count <= BUFFER_WORDS * 4);
    // SAFETY: as in `words`; u8 has no alignment requirement.
    unsafe { core::slice::from_raw_parts_mut(buffer.cast::<u8>(), count) }
}

/// `slice[index]`.
#[inline(always)]
fn at<T: Copy>(slice: &[T], index: usize) -> T {
    #[cfg(feature = "unchecked")]
    // SAFETY: measurement build only; every kernel index is in bounds.
    return unsafe { *slice.get_unchecked(index) };
    #[cfg(not(feature = "unchecked"))]
    slice[index]
}

/// `slice[index] = value`.
#[inline(always)]
fn set<T>(slice: &mut [T], index: usize, value: T) {
    #[cfg(feature = "unchecked")]
    // SAFETY: as in `at`.
    unsafe {
        *slice.get_unchecked_mut(index) = value
    }
    #[cfg(not(feature = "unchecked"))]
    {
        slice[index] = value
    }
}

/// `slice.swap(a, b)`.
#[inline(always)]
fn swap<T>(slice: &mut [T], a: usize, b: usize) {
    #[cfg(feature = "unchecked")]
    // SAFETY: as in `at`; `ptr::swap` allows a == b.
    unsafe {
        let base = slice.as_mut_ptr();
        core::ptr::swap(base.add(a), base.add(b))
    }
    #[cfg(not(feature = "unchecked"))]
    slice.swap(a, b)
}

mod bench_crc32 {
    use super::{at, set};

    pub fn run(data: &[u8]) -> u32 {
        let mut table = [0u32; 256];
        for i in 0..256 {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            set(&mut table, i, c);
        }
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in data {
            crc = at(&table, ((crc ^ byte as u32) & 0xFF) as usize) ^ (crc >> 8);
        }
        !crc
    }
}

mod bench_sieve {
    use super::{at, set};

    pub fn run(flags: &mut [u8]) -> u32 {
        let n = flags.len();
        flags.fill(1);
        set(flags, 0, 0);
        set(flags, 1, 0);
        let mut i = 2;
        while i * i < n {
            if at(flags, i) != 0 {
                let mut j = i * i;
                while j < n {
                    set(flags, j, 0);
                    j += i;
                }
            }
            i += 1;
        }
        flags.iter().map(|&flag| flag as u32).sum()
    }
}

mod bench_sort {
    use super::{at, set, swap};

    /// Hoare partition of `a[lo..=hi]` around the middle element.
    fn partition(a: &mut [u32], lo: usize, hi: usize) -> usize {
        let pivot = at(a, lo + (hi - lo) / 2);
        let mut i = lo;
        let mut j = hi;
        loop {
            while at(a, i) < pivot {
                i += 1;
            }
            while at(a, j) > pivot {
                j -= 1;
            }
            if i >= j {
                return j;
            }
            swap(a, i, j);
            i += 1;
            j -= 1;
        }
    }

    fn insertion(a: &mut [u32], lo: usize, hi: usize) {
        for i in lo + 1..=hi {
            let value = at(a, i);
            let mut j = i;
            while j > lo && at(a, j - 1) > value {
                set(a, j, at(a, j - 1));
                j -= 1;
            }
            set(a, j, value);
        }
    }

    /// Recurses into the smaller part and loops on the larger one.
    fn range(a: &mut [u32], mut lo: usize, mut hi: usize) {
        while hi - lo >= 16 {
            let middle = partition(a, lo, hi);
            if middle - lo < hi - middle {
                range(a, lo, middle);
                lo = middle + 1;
            } else {
                range(a, middle + 1, hi);
                hi = middle;
            }
        }
        insertion(a, lo, hi);
    }

    pub fn run(a: &mut [u32]) -> u32 {
        if a.len() > 1 {
            range(a, 0, a.len() - 1);
        }
        let mut sum = 0u32;
        for (i, &value) in a.iter().enumerate() {
            if i > 0 && at(a, i - 1) > value {
                return 0xFFFF_FFFF;
            }
            sum = sum.wrapping_add(value.wrapping_mul(i as u32 + 1));
        }
        sum
    }
}

mod bench_matmul {
    use super::{at, set};

    pub fn run(buffer: &mut [u32], n: usize) -> u32 {
        let (a, rest) = buffer.split_at_mut(n * n);
        let (b, c) = rest.split_at_mut(n * n);
        let c = &mut c[..n * n];
        for i in 0..n {
            for j in 0..n {
                let mut sum = 0u32; // the int32 values, wrapping
                for k in 0..n {
                    sum = sum.wrapping_add(at(a, i * n + k).wrapping_mul(at(b, k * n + j)));
                }
                set(c, i * n + j, sum);
            }
        }
        c.iter().fold(0u32, |hash, &value| hash.wrapping_mul(31).wrapping_add(value))
    }
}

mod bench_nqueens {
    fn solve(columns: u32, left: u32, right: u32, all: u32) -> u32 {
        if columns == all {
            return 1;
        }
        let mut count = 0;
        let mut free = all & !(columns | left | right);
        while free != 0 {
            let bit = free & free.wrapping_neg();
            free -= bit;
            count += solve(columns | bit, (left | bit) << 1, (right | bit) >> 1, all);
        }
        count
    }

    pub fn run(n: u32) -> u32 {
        solve(0, 0, 0, (1 << n) - 1)
    }
}

mod bench_mandel {
    pub fn run(n: u32) -> u32 {
        const ONE: i32 = 4096; // Q12
        let width = n as i32;
        let height = width * 2 / 3;
        let mut total = 0u32;
        for y in 0..height {
            let ci = -ONE + 2 * ONE * y / height;
            for x in 0..width {
                let cr = -10 * ONE / 4 + 14 * ONE / 4 * x / width;
                let (mut zr, mut zi) = (0i32, 0i32);
                let mut iteration = 0u32;
                while iteration < 32 {
                    let zr2 = (zr * zr) >> 12;
                    let zi2 = (zi * zi) >> 12;
                    if zr2 + zi2 > 4 * ONE {
                        break;
                    }
                    zi = ((zr * zi) >> 11) + ci;
                    zr = zr2 - zi2 + cr;
                    iteration += 1;
                }
                total += iteration;
            }
        }
        total
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn bench_crc32(buffer: *mut u32, n: u32) -> u32 {
    bench_crc32::run(bytes(buffer, n as usize))
}

#[unsafe(no_mangle)]
pub extern "C" fn bench_sieve(buffer: *mut u32, n: u32) -> u32 {
    bench_sieve::run(bytes(buffer, n as usize))
}

#[unsafe(no_mangle)]
pub extern "C" fn bench_sort(buffer: *mut u32, n: u32) -> u32 {
    bench_sort::run(words(buffer, n as usize))
}

#[unsafe(no_mangle)]
pub extern "C" fn bench_matmul(buffer: *mut u32, n: u32) -> u32 {
    let n = n as usize;
    bench_matmul::run(words(buffer, 3 * n * n), n)
}

#[unsafe(no_mangle)]
pub extern "C" fn bench_nqueens(_buffer: *mut u32, n: u32) -> u32 {
    bench_nqueens::run(n)
}

#[unsafe(no_mangle)]
pub extern "C" fn bench_mandel(_buffer: *mut u32, n: u32) -> u32 {
    bench_mandel::run(n)
}
