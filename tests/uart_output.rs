//! Capture the actual UART stdout path in a child process. Rust's test output
//! capture would otherwise mask byte-to-character re-encoding mistakes.
use cpt32::devices::pec::serial::UART;
use std::process::Command;

const PAYLOAD: &[u8] = b"UART:\0\x1b[31m\xe6\x97\xa5\xe6\x9c\xac\xff\r\n";

#[test]
fn uart_raw_output_child() {
    if std::env::var_os("CPT32_UART_OUTPUT_CHILD").is_none() {
        return;
    }
    let mut uart = UART::new();
    for &byte in PAYLOAD {
        uart.write(byte);
    }
    assert!(uart.tx_buffer.is_empty());
    assert!(uart.tx_ready);
}

#[test]
fn uart_mmio_reports_completed_synchronous_transmission() {
    use cpt32::bus::Device;
    use cpt32::devices::pec::serial::UARTDevice;
    let mut uart = UARTDevice::new(false);
    uart.write(0, b'K');
    assert_eq!(uart.read(5), 0);
    assert_eq!(uart.read(1) & 1, 1);
    assert_eq!(uart.debug_read(5), Some(0));
}

#[test]
fn stdout_preserves_uart_bytes() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "uart_raw_output_child", "--nocapture"])
        .env("CPT32_UART_OUTPUT_CHILD", "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        output
            .stdout
            .windows(PAYLOAD.len())
            .any(|bytes| bytes == PAYLOAD)
    );
}
