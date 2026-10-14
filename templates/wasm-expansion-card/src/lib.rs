#![no_std]

use core::panic::PanicInfo;
use cpt32_wasm_expansion_card::Card;

struct MyCard {
    registers: [u8; 256],
}

impl Default for MyCard {
    fn default() -> Self {
        Self {
            registers: [0; 256],
        }
    }
}

impl Card for MyCard {
    fn reset(&mut self) {
        self.registers = [0; 256];
    }

    fn mmio_read(&mut self, offset: u32) -> u8 {
        self.registers.get(offset as usize).copied().unwrap_or(0)
    }

    fn mmio_write(&mut self, offset: u32, value: u8) {
        if let Some(register) = self.registers.get_mut(offset as usize) {
            *register = value;
        }
    }

    fn tick(&mut self) {}
}

cpt32_wasm_expansion_card::export_card!(MyCard);

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
