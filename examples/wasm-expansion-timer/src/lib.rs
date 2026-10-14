#![no_std]

use core::panic::PanicInfo;
use cpt32_wasm_expansion_card::{
    Card, asset_count, asset_read_byte, asset_size, irq_set, schedule_tick,
};

const PERIOD_LOW: u32 = 0x00;
const PERIOD_HIGH: u32 = 0x01;
const CONTROL: u32 = 0x02;
const STATUS: u32 = 0x03;
const LABEL_BASE: u32 = 0x10;

#[derive(Default)]
struct TimerCard {
    period: u16,
    enabled: bool,
    pending: bool,
}

impl TimerCard {
    fn active_period(&self) -> u64 {
        u64::from(self.period.max(1))
    }
}

impl Card for TimerCard {
    fn reset(&mut self) {
        self.period = 0;
        self.enabled = false;
        self.pending = false;
        let _ = schedule_tick(0);
        irq_set(false);
    }

    fn mmio_read(&mut self, offset: u32) -> u8 {
        match offset {
            PERIOD_LOW => self.period as u8,
            PERIOD_HIGH => (self.period >> 8) as u8,
            CONTROL => u8::from(self.enabled),
            STATUS => u8::from(self.pending),
            LABEL_BASE.. => {
                let asset_offset = offset - LABEL_BASE;
                if asset_count() == 0 || asset_offset >= asset_size(0) {
                    0
                } else {
                    asset_read_byte(0, asset_offset)
                }
            }
            _ => 0,
        }
    }

    fn mmio_write(&mut self, offset: u32, value: u8) {
        match offset {
            PERIOD_LOW => self.period = (self.period & 0xFF00) | u16::from(value),
            PERIOD_HIGH => self.period = (self.period & 0x00FF) | (u16::from(value) << 8),
            CONTROL if value & 1 != 0 => {
                self.enabled = true;
                let _ = schedule_tick(self.active_period());
            }
            CONTROL => {
                self.enabled = false;
                let _ = schedule_tick(0);
            }
            STATUS if value & 1 != 0 => {
                self.pending = false;
                irq_set(false);
            }
            _ => {}
        }
    }

    fn tick(&mut self) {
        if self.enabled {
            self.pending = true;
            irq_set(true);
            let _ = schedule_tick(self.active_period());
        }
    }
}

cpt32_wasm_expansion_card::export_card!(TimerCard);

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
