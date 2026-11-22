// Serial port emulation for the PeC (Peripheral Controller)
use crate::bus::Device;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
pub struct UART {
    pub tx_buffer: Vec<u8>, // Transmit buffer for outgoing data
    pub rx_buffer: Vec<u8>, // Receive buffer for incoming data
    pub tx_ready: bool,     // Flag indicating if the UART is ready to transmit
    pub rx_ready: bool,     // Flag indicating if the UART has received data
    pub enable: bool,       // Flag indicating if the UART is enabled
    pub int_enable: bool,   // Flag indicating if UART interrupts are enabled
    pub baud_rate: u32,     // Baud rate for UART communication
}

impl UART {
    pub fn new() -> Self {
        Self {
            tx_buffer: Vec::new(),
            rx_buffer: Vec::new(),
            tx_ready: true,
            rx_ready: false,
            enable: true,
            int_enable: false,
            baud_rate: 115200,
        }
    }

    pub fn write(&mut self, data: u8) {
        self.tx_buffer.push(data);
        self.tx_ready = false; // Not ready to transmit until the buffer is processed
        // UART carries bytes. Formatting `data as char` would re-encode every
        // byte >= 0x80, corrupting UTF-8 and binary output.
        write_host_byte(&mut std::io::stdout().lock(), data);
        // Host output is synchronous: this byte has left the transmit queue.
        self.process_tx();
    }

    pub fn read(&mut self) -> Option<u8> {
        if !self.rx_buffer.is_empty() {
            let data = self.rx_buffer.remove(0);
            if self.rx_buffer.is_empty() {
                self.rx_ready = false; // No more data to read
            }
            Some(data)
        } else {
            None
        }
    }

    pub fn process_tx(&mut self) {
        if !self.tx_buffer.is_empty() {
            // Simulate sending data over the serial line
            let _data = self.tx_buffer.remove(0);
            // After processing, set tx_ready to true
            if self.tx_buffer.is_empty() {
                self.tx_ready = true;
            }
        }
    }

    pub fn receive_data(&mut self, data: u8) {
        self.rx_buffer.push(data);
        self.rx_ready = true; // Data is available to read
    }
}

/// Mirrors one guest UART byte to the host terminal. Guest output must never
/// take the emulator down: a Windows console rejects bytes that are not valid
/// UTF-8 (`InvalidData`), so such a byte is shown as U+FFFD, and any other host
/// I/O error (closed pipe etc.) is ignored. The guest still sees the byte sent.
fn write_host_byte<W: Write>(out: &mut W, data: u8) {
    match out.write_all(&[data]) {
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            let _ = out.write_all("\u{FFFD}".as_bytes());
        }
        _ => {}
    }
    let _ = out.flush();
}

const UART_BASE_ADDR: u32 = 0x80040000; // Base address for the UART device

pub struct UARTDevice {
    uart: Arc<Mutex<UART>>,
}

impl UARTDevice {
    pub fn new(with_stdin: bool) -> Self {
        let uart = Arc::new(Mutex::new(UART::new()));
        if with_stdin {
            Self::start_stdin_reader(Arc::clone(&uart));
        }
        Self { uart }
    }

    fn start_stdin_reader(uart: Arc<Mutex<UART>>) {
        thread::spawn(move || {
            let stdin = std::io::stdin();
            let mut handle = stdin.lock();
            let mut buf = [0u8; 1];

            loop {
                match handle.read(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        if let Ok(mut uart) = uart.lock() {
                            uart.receive_data(buf[0]);
                        } else {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }
}

impl Device for UARTDevice {
    /// Host-side byte queues; its interrupt enable bit drives no IRQ source.
    fn is_timing_insensitive(&self) -> bool {
        true
    }

    fn debug_read(&mut self, addr: u32) -> Option<u8> {
        // DATA_RW consumes RX data, so it has no passive snapshot equivalent.
        if addr == 0 {
            return None;
        }
        let uart = self.uart.lock().unwrap();
        Some(match addr {
            1 => u8::from(uart.tx_ready) | (u8::from(uart.rx_ready) << 1),
            2 => uart.baud_rate as u8,
            3 => u8::from(!uart.enable) | (u8::from(uart.int_enable) << 1),
            4 => uart.rx_buffer.len() as u8,
            5 => uart.tx_buffer.len() as u8,
            _ => 0,
        })
    }

    fn read(&mut self, addr: u32) -> u8 {
        let mut uart = self.uart.lock().unwrap();
        match addr {
            0x0 => {
                // DATA_RW (RW)
                uart.read().unwrap_or(0)
            }
            0x1 => {
                // STATUS (R)
                let mut status: u8 = 0;
                if uart.tx_ready {
                    status |= 0x01; // TX_READY
                }
                if uart.rx_ready {
                    status |= 0x02; // RX_READY
                }
                status
            }
            0x2 => {
                // BAUD_RATE (RW)
                uart.baud_rate as u8
            }
            0x3 => {
                // CONTROL (RW)
                let mut control: u8 = 0;
                if !uart.enable {
                    // Reversed logic: 0 means enabled
                    control |= 0x01; // /ENABLE
                }
                if uart.int_enable {
                    control |= 0x02; // INT_ENABLE
                }
                control
            }
            0x4 => {
                // RX_BYTES (R)
                uart.rx_buffer.len() as u8
            }
            0x5 => {
                // TX_BYTES (R)
                uart.tx_buffer.len() as u8
            }
            _ => {
                0 // Default return value for unimplemented registers
            }
        }
        // Implementation for reading from UART device
    }

    fn write(&mut self, addr: u32, data: u8) {
        let mut uart = self.uart.lock().unwrap();
        match addr {
            0x0 => {
                // DATA_RW (RW)
                uart.write(data);
            }
            0x2 => {
                // BAUD_RATE (RW)
                uart.baud_rate = data as u32; // Set baud rate (assuming 8-bit value for simplicity)
            }
            0x3 => {
                // CONTROL (RW)
                uart.enable = (data & 0x01) == 0; // Reversed logic: 0 means enabled
                uart.int_enable = (data & 0x02) != 0; // Set interrupt enable
            }
            _ => {
                // Ignore writes to unimplemented registers
            }
        }
        // Implementation for writing to UART device
    }
    fn size(&self) -> u32 {
        0x10 // Size of the UART device's register space (16 bytes)
    }
}

pub fn connect_uart(bus: &mut crate::bus::Bus) {
    connect_uart_with_stdin(bus, true);
}

pub fn connect_uart_with_stdin(bus: &mut crate::bus::Bus, with_stdin: bool) {
    bus.add_device(
        UART_BASE_ADDR,
        "PeC UART MMIO",
        Box::new(UARTDevice::new(with_stdin)),
    );
}

#[cfg(test)]
mod tests {
    use super::write_host_byte;
    use std::io::{self, Write};

    /// Behaves like a Windows console: rejects a byte that is not ASCII.
    struct StrictConsole(Vec<u8>);

    impl Write for StrictConsole {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf.iter().any(|&b| b >= 0x80) {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8"));
            }
            self.0.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct BrokenPipe;

    impl Write for BrokenPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }

    #[test]
    fn host_write_passes_bytes_through() {
        let mut out = Vec::new();
        for b in [b'A', 0xE3, 0x81, 0x82, 0xFF] {
            write_host_byte(&mut out, b);
        }
        assert_eq!(out, [b'A', 0xE3, 0x81, 0x82, 0xFF]);
    }

    #[test]
    fn rejected_byte_becomes_replacement_char_instead_of_panicking() {
        let mut out = StrictConsole(Vec::new());
        write_host_byte(&mut out, b'A');
        write_host_byte(&mut out, 0xFF);
        write_host_byte(&mut out, b'B');
        // The strict writer also rejects U+FFFD's bytes, so only ASCII survives.
        assert_eq!(out.0, b"AB");
    }

    #[test]
    fn other_host_errors_are_ignored() {
        write_host_byte(&mut BrokenPipe, b'A');
    }
}
