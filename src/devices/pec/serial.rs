// Serial port emulation for the PeC (Peripheral Controller)

pub struct UART {
    pub tx_buffer: Vec<u8>, // Transmit buffer for outgoing data
    pub rx_buffer: Vec<u8>, // Receive buffer for incoming data
    pub tx_ready: bool, // Flag indicating if the UART is ready to transmit
    pub rx_ready: bool, // Flag indicating if the UART has received data
}

impl UART {
    pub fn new() -> Self {
        Self {
            tx_buffer: Vec::new(),
            rx_buffer: Vec::new(),
            tx_ready: true,
            rx_ready: false,
        }
    }

    pub fn write(&mut self, data: u8) {
        self.tx_buffer.push(data);
        self.tx_ready = false; // Not ready to transmit until the buffer is processed
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