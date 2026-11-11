//! Debug UART output (`sdk/cpt32/src/print.c`).

use crate::mmio;

pub const UART_ADDR: u32 = 0x8004_0000;

pub fn putc(byte: u8) {
    mmio::write8(UART_ADDR, byte);
}

pub fn write_str(text: &str) {
    for &byte in text.as_bytes() {
        putc(byte);
    }
}

/// `text` followed by CR LF, like `print_prnln`.
pub fn write_line(text: &str) {
    write_str(text);
    putc(0x0d);
    putc(0x0a);
}

/// `core::fmt::Write` for the UART (`\n` is sent as is).
pub struct Uart;

impl core::fmt::Write for Uart {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        write_str(text);
        Ok(())
    }
}
