//! Bare-metal Rust on CPT32: PCG text output, `core::fmt`, slices, 64-bit and
//! floating-point arithmetic and trait objects. `main` returns 42 when every
//! check passes (otherwise the number of the first failing check), which
//! `tools/llvm-src32/test_rust.py` runs on src32_testbench.
#![no_std]

use core::fmt::Write;
use cpt32::pcg::Pcg;

trait Shape {
    fn area(&self) -> u32;
}

struct Rect(u32, u32);
struct Square(u32);

impl Shape for Rect {
    fn area(&self) -> u32 {
        self.0 * self.1
    }
}

impl Shape for Square {
    fn area(&self) -> u32 {
        self.0 * self.0
    }
}

/// `write!` into a fixed buffer, to check the formatted text.
struct Buffer {
    bytes: [u8; 64],
    len: usize,
}

impl Write for Buffer {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let end = self.len + text.len();
        if end > self.bytes.len() {
            return Err(core::fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

static mut DATA: [i32; 8] = [5, -3, 12, 0, 7, -8, 1, 9];

fn checks() -> i32 {
    // Slices and sorting.
    let data = unsafe { &mut *core::ptr::addr_of_mut!(DATA) };
    data.sort_unstable();
    if data != &[-8, -3, 0, 1, 5, 7, 9, 12] {
        return 1;
    }
    // Iterators and closures.
    let sum: i32 = data.iter().filter(|&&x| x > 0).map(|&x| x * 2).sum();
    if sum != 68 {
        return 2;
    }
    // 64-bit arithmetic.
    let big = 0x1234_5678_9abc_def0u64;
    if big / 0x1_0000 != 0x1234_5678_9abc || (big.rotate_left(8) & 0xff) != 0x12 {
        return 3;
    }
    // Floating point (SoftFloat).
    let x = core::hint::black_box(1.5f32);
    if x * 4.0 + 0.25 != 6.25 || (core::hint::black_box(10.0f64) / 4.0) as i32 != 2 {
        return 4;
    }
    // Trait objects (indirect calls).
    let shapes: [&dyn Shape; 2] = [&Rect(3, 4), &Square(5)];
    if shapes.iter().map(|s| s.area()).sum::<u32>() != 37 {
        return 5;
    }
    // core::fmt.
    let mut buffer = Buffer {
        bytes: [0; 64],
        len: 0,
    };
    if write!(buffer, "{}+{:04x}={:?}", -42, 0xbeefu32, Some("ok")).is_err() {
        return 6;
    }
    if &buffer.bytes[..buffer.len] != b"-42+beef=Some(\"ok\")" {
        return 7;
    }
    // UTF-8 and Option/Result.
    match core::str::from_utf8(b"CPT32") {
        Ok(text) if text.chars().rev().next() == Some('2') => {}
        _ => return 8,
    }
    42
}

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    let mut pcg = Pcg::init();
    pcg.cls(0);
    let result = checks();
    let _ = writeln!(pcg, "Hello from Rust on CPT32!");
    let _ = writeln!(
        pcg,
        "checks: {}",
        if result == 42 { "ok" } else { "failed" }
    );
    result
}
