// Serial port emulation for the PeC (Peripheral Controller)
use crate::bus::Device;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
pub struct UART {
    pub tx_buffer: Vec<u8>, // Transmit buffer for outgoing data
    pub rx_buffer: Vec<u8>, // Receive buffer for incoming data
    pub tx_ready: bool, // Flag indicating if the UART is ready to transmit
    pub rx_ready: bool, // Flag indicating if the UART has received data
    pub enable: bool, // Flag indicating if the UART is enabled
    pub int_enable: bool, // Flag indicating if UART interrupts are enabled
    pub baud_rate: u32, // Baud rate for UART communication
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
        print!("{}", data as char); // Output the character to the console
        std::io::stdout().flush().unwrap();
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
    fn read(&mut self, addr: u32) -> u8 {
        let mut uart = self.uart.lock().unwrap();
        match addr {
            0x0 => { // DATA_RW (RW)
                uart.read().unwrap_or(0)
            } 
            0x1 => { // STATUS (R)
                let mut status: u8 = 0;
                if uart.tx_ready {
                    status |= 0x01; // TX_READY
                }
                if uart.rx_ready {
                    status |= 0x02; // RX_READY
                }
                status
            }
            0x2 => { // BAUD_RATE (RW)
                uart.baud_rate as u8
            }
            0x3 => { // CONTROL (RW)
                let mut control: u8 = 0;
                if !uart.enable { // Reversed logic: 0 means enabled
                    control |= 0x01; // /ENABLE
                }
                if uart.int_enable {
                    control |= 0x02; // INT_ENABLE
                }
                control
            }
            0x4 => { // RX_BYTES (R)
                uart.rx_buffer.len() as u8
            }
            0x5 => { // TX_BYTES (R)
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
            0x0 => { // DATA_RW (RW)
                uart.write(data);
            }
            0x2 => { // BAUD_RATE (RW)
                uart.baud_rate = data as u32; // Set baud rate (assuming 8-bit value for simplicity)
            }
            0x3 => { // CONTROL (RW)
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
    bus.add_device(UART_BASE_ADDR, Box::new(UARTDevice::new(with_stdin)));
}