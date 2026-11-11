//! Bare-metal CPT32 support for Rust (`no_std`).
//!
//! Build with `tools/llvm-src32/cargo_src32.py`; the program is a library crate
//! whose `#[unsafe(no_mangle)] pub extern "C" fn main() -> i32` is the entry
//! (its return value is left in R1 and the CPU halts, as with C).
//!
//! - [`cpu`]: the HALT / Extension P intrinsics of `<cpt32/cpu.h>`.
//! - [`mmio`]: volatile access to device registers.
//! - [`pcg`]: the PCG text screen (a Rust port of `sdk/cpt32/src/pcg.c`).
//! - [`uart`]: the debug UART (`sdk/cpt32/src/print.c`).
#![no_std]

pub mod cpu;
pub mod mmio;
pub mod pcg;
pub mod uart;

/// Writes `value` in decimal through `out` (no `core::fmt`, which is large on SRC32).
pub fn write_decimal(value: u32, mut out: impl FnMut(u8)) {
    let mut digits = [0u8; 10];
    let mut n = value;
    let mut len = 0;
    loop {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    while len > 0 {
        len -= 1;
        out(digits[len]);
    }
}

#[cfg(feature = "panic-handler")]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    uart::write_str("panic");
    if let Some(location) = info.location() {
        uart::write_str(" at ");
        uart::write_str(location.file());
        uart::putc(b':');
        write_decimal(location.line(), uart::putc);
    }
    uart::write_str("\r\n");
    loop {
        cpu::halt();
    }
}
