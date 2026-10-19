#![cfg_attr(target_arch = "wasm32", no_std)]

pub mod chip;
mod envelope;
mod filter;
mod synthesis;

#[cfg(target_arch = "wasm32")]
mod card {
    use super::chip::{Chip, PCM_SIZE};
    use cpt32_wasm_expansion_card::Card;
    struct Pcm(core::cell::UnsafeCell<[u8; PCM_SIZE]>);
    // All exports run serially; PCM is never borrowed across an export call.
    unsafe impl Sync for Pcm {}
    static PCM: Pcm = Pcm(core::cell::UnsafeCell::new([0; PCM_SIZE]));

    #[derive(Default)]
    struct SoundCard {
        chip: Chip,
        resample_phase: u64,
        sample: [i16; 2],
        primed: bool,
    }
    impl Card for SoundCard {
        fn reset(&mut self) {
            self.chip = Chip::default();
            self.resample_phase = 0;
            self.sample = [0; 2];
            self.primed = false;
            unsafe {
                (&mut *PCM.0.get()).fill(0);
            }
        }
        fn mmio_read(&mut self, offset: u32) -> u8 {
            if (offset as usize) < PCM_SIZE {
                unsafe { (*PCM.0.get())[offset as usize] }
            } else {
                self.chip.read(offset)
            }
        }
        fn debug_read(&self, offset: u32) -> Option<u8> {
            if (offset as usize) < PCM_SIZE {
                Some(unsafe { (*PCM.0.get())[offset as usize] })
            } else if offset < 0x400000 + 0x400 {
                Some(self.chip.read(offset))
            } else {
                None
            }
        }
        fn mmio_write(&mut self, offset: u32, value: u8) {
            if (offset as usize) < PCM_SIZE {
                unsafe {
                    (*PCM.0.get())[offset as usize] = value;
                }
            } else {
                self.chip.write(offset, value);
            }
        }
        fn tick(&mut self) {}
        fn render_audio(&mut self, rate: u32, output: &mut [[i16; 2]]) {
            // Fixed 48 kHz chip clock, rational zero-order hold resampling.
            // Bounded even for very low host rates, and independent of block size.
            for frame in output {
                if !self.primed {
                    self.sample = self.chip.sample(unsafe { &*PCM.0.get() });
                    self.primed = true;
                }
                while self.resample_phase >= rate as u64 {
                    self.resample_phase -= rate as u64;
                    self.sample = self.chip.sample(unsafe { &*PCM.0.get() });
                }
                *frame = self.sample;
                self.resample_phase += 48_000;
            }
        }
    }
    cpt32_wasm_expansion_card::export_card!(SoundCard);
    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
        core::arch::wasm32::unreachable()
    }
}
