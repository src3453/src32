#![cfg_attr(target_arch = "wasm32", no_std)]

//! 3HS816 as a CPT32 WASM expansion card. The slot-relative MMIO window
//! `0x000..0x4FF` is the chip's register map (`doc/spec_3HS816.md`).

pub mod chip;
pub(crate) mod tables;

#[cfg(target_arch = "wasm32")]
mod card {
    use super::chip::{Chip, REGISTER_SIZE, SAMPLE_RATE};
    use cpt32_wasm_expansion_card::Card;

    #[derive(Default)]
    struct SoundCard {
        chip: Chip,
        resample_phase: u64,
        sample: [i16; 2],
        primed: bool,
    }

    impl Card for SoundCard {
        fn reset(&mut self) {
            *self = Self::default();
        }
        fn mmio_read(&mut self, offset: u32) -> u8 {
            self.chip.read(offset)
        }
        fn debug_read(&self, offset: u32) -> Option<u8> {
            (offset < REGISTER_SIZE).then(|| self.chip.read(offset))
        }
        fn mmio_write(&mut self, offset: u32, value: u8) {
            if offset < REGISTER_SIZE {
                self.chip.write(offset, value);
            }
        }
        fn tick(&mut self) {}
        fn render_audio(&mut self, rate: u32, output: &mut [[i16; 2]]) {
            // Fixed 48 kHz chip clock with rational zero-order hold, like the
            // 3WS88PWN4 card: bounded for any host rate and block size.
            for frame in output {
                if !self.primed {
                    self.sample = self.chip.sample();
                    self.primed = true;
                }
                while self.resample_phase >= u64::from(rate) {
                    self.resample_phase -= u64::from(rate);
                    self.sample = self.chip.sample();
                }
                *frame = self.sample;
                self.resample_phase += u64::from(SAMPLE_RATE);
            }
        }
    }

    cpt32_wasm_expansion_card::export_card!(SoundCard);

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
        core::arch::wasm32::unreachable()
    }
}
