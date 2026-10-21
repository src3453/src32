#![no_std]

use cpt32_wasm_expansion_card::Card;
use psg::{ChipType, PSG};

const CLOCK_HZ: f64 = 1_789_772.5;
const INTERNAL_RATE: u32 = 48_000;
const ID: &[u8; 8] = b"PSG-RS01";
const MASKS: [u8; 16] = [
    255, 15, 255, 15, 255, 15, 31, 255, 31, 31, 31, 255, 255, 15, 255, 255,
];

/// Byte-addressed PSG adapter. Readback is a passive register latch snapshot.
pub struct PsgCard {
    psg: PSG,
    registers: [u8; 16],
    selected: u8,
    model: u8,
    phase: u64,
    last_rate: u32,
    sample: [i16; 2],
    primed: bool,
}

impl Default for PsgCard {
    fn default() -> Self {
        let mut psg = PSG::new(CLOCK_HZ, INTERNAL_RATE).expect("fixed clock/rate are valid");
        let mut registers = [0; 16];
        registers[7] = 0x3f;
        for (index, &value) in registers.iter().enumerate() {
            psg.set_register(index as u8, value);
        }
        Self {
            psg,
            registers,
            selected: 0,
            model: 0,
            phase: 0,
            last_rate: 0,
            sample: [0; 2],
            primed: false,
        }
    }
}

impl PsgCard {
    fn write_register(&mut self, index: u8, value: u8) {
        let value = value & MASKS[index as usize];
        self.registers[index as usize] = value;
        // Reassemble tone periods from the latches: upstream clamps each byte
        // setter's zero period to 1, which otherwise makes byte order observable.
        if index < 6 {
            let base = (index & !1) as usize;
            let period = u16::from_le_bytes([self.registers[base], self.registers[base + 1]]);
            self.psg.set_tone_period(index / 2, period);
        } else if index == 11 || index == 12 {
            self.psg
                .set_envelope_period(u16::from_le_bytes([self.registers[11], self.registers[12]]));
        } else {
            // Includes repeated R13 writes: these must retrigger the envelope.
            self.psg.set_register(index, value);
        }
    }

    fn snapshot(&self, offset: u32) -> Option<u8> {
        match offset {
            0x00..=0x0f => Some(self.registers[offset as usize]),
            0x10 => Some(self.selected),
            0x11 => Some(self.registers[self.selected as usize]),
            0x12 => Some(self.model),
            0xf0..=0xf7 => Some(ID[(offset - 0xf0) as usize]),
            _ => None,
        }
    }

    fn next_sample(&mut self) -> [i16; 2] {
        let (left, right) = self.psg.render();
        // Default upstream pan is 0.5 per side per channel. Leave headroom for
        // three channels plus FIR overshoot; float-to-int casts saturate.
        [(left * 16_384.0) as i16, (right * 16_384.0) as i16]
    }
}

impl Card for PsgCard {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn mmio_read(&mut self, offset: u32) -> u8 {
        self.snapshot(offset).unwrap_or(0)
    }

    fn debug_read(&self, offset: u32) -> Option<u8> {
        self.snapshot(offset)
    }

    fn mmio_write(&mut self, offset: u32, value: u8) {
        match offset {
            0x00..=0x0f => self.write_register(offset as u8, value),
            0x10 => self.selected = value & 15,
            0x11 => self.write_register(self.selected, value),
            0x12 => {
                self.model = value & 1;
                self.psg.set_chip_type(if self.model == 0 {
                    ChipType::YM
                } else {
                    ChipType::AY
                });
            }
            _ => {}
        }
    }

    fn tick(&mut self) {}

    fn render_audio(&mut self, rate: u32, output: &mut [[i16; 2]]) {
        // Bound work for pathological rates; normal host rates are >= 8 kHz.
        if rate < 8_000 {
            output.fill([0; 2]);
            return;
        }
        if self.last_rate != 0 && self.last_rate != rate {
            self.phase = self.phase * u64::from(rate) / u64::from(self.last_rate);
        }
        self.last_rate = rate;
        for frame in output {
            if !self.primed {
                self.sample = self.next_sample();
                self.primed = true;
            }
            while self.phase >= u64::from(rate) {
                self.phase -= u64::from(rate);
                self.sample = self.next_sample();
            }
            *frame = self.sample;
            self.phase += u64::from(INTERNAL_RATE);
        }
    }
}

// psg 1.0.1 links Rust std (including its panic implementation). The adapter
// itself uses core only; do not add a duplicate #[panic_handler] here.
#[cfg(target_arch = "wasm32")]
cpt32_wasm_expansion_card::export_card!(PsgCard);
