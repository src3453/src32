// Random number generator emulation for the PeC.
//
// The PRNG uses a 32-bit GFSR with the conventional (55, 24) taps. The
// TRNG is backed by the host operating system's random source through rand.
use crate::bus::{Bus, Device};
use rand::random;

const RNG_BASE_ADDR: u32 = 0x8004_0010;
const DEFAULT_SEED: u32 = 0x5555_5555;

struct Gfsr {
    state: [u32; 55],
    index: usize,
}

impl Gfsr {
    fn new(seed: u32) -> Self {
        let mut state = [0u32; 55];
        state[0] = seed;
        for i in 1..55 {
            state[i] = state[i - 1]
                .wrapping_mul(1_812_433_253)
                .wrapping_add(i as u32);
        }
        Self { state, index: 0 }
    }

    fn next(&mut self) -> u32 {
        let current = self.index;
        let lag_24 = (current + 55 - 24) % 55;
        let value = self.state[current] ^ self.state[lag_24];
        self.state[current] = value;
        self.index = (current + 1) % 55;
        value
    }
}

pub struct RngDevice {
    prng: Gfsr,
    seed: u32,
    prng_data: u32,
    trng_data: u32,
    trng_enabled: bool,
}

impl RngDevice {
    pub fn new() -> Self {
        Self {
            prng: Gfsr::new(DEFAULT_SEED),
            seed: DEFAULT_SEED,
            prng_data: 0,
            trng_data: 0,
            trng_enabled: true,
        }
    }

    fn byte(value: u32, offset: u32) -> u8 {
        value.to_be_bytes()[(offset & 3) as usize]
    }

    fn set_seed(&mut self, seed: u32) {
        self.seed = seed;
        self.prng = Gfsr::new(seed);
    }
}

impl Device for RngDevice {
    fn read(&mut self, addr: u32) -> u8 {
        match addr {
            0x00..=0x03 => {
                if addr == 0 {
                    self.prng_data = self.prng.next();
                }
                Self::byte(self.prng_data, addr)
            }
            0x04..=0x07 => Self::byte(self.seed, addr - 4),
            0x08..=0x0B => {
                if addr == 8 && self.trng_enabled {
                    self.trng_data = random();
                }
                Self::byte(self.trng_data, addr - 8)
            }
            0x0C => u8::from(self.trng_enabled),
            0x0D => u8::from(!self.trng_enabled),
            _ => 0,
        }
    }

    fn write(&mut self, addr: u32, value: u8) {
        match addr {
            0x04..=0x07 => {
                let shift = (3 - (addr - 4)) * 8;
                self.seed = (self.seed & !(0xFF << shift)) | ((value as u32) << shift);
                if addr == 0x07 {
                    self.set_seed(self.seed);
                }
            }
            0x0D => self.trng_enabled = (value & 1) == 0,
            _ => {}
        }
    }

    fn size(&self) -> u32 {
        // 0x00..=0x0D: PRNG_DATA, PRNG_SEED, TRNG_DATA, STATUS, CONTROL.
        0x0E
    }
}

pub fn connect_rng(bus: &mut Bus) {
    bus.add_device(RNG_BASE_ADDR, Box::new(RngDevice::new()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_write_restarts_sequence() {
        let mut rng = RngDevice::new();
        let first = [rng.read(0), rng.read(1), rng.read(2), rng.read(3)];
        for (offset, byte) in (0x04..=0x07).zip(0x1234_5678u32.to_be_bytes()) {
            rng.write(offset, byte);
        }
        let seeded = [rng.read(0), rng.read(1), rng.read(2), rng.read(3)];
        for (offset, byte) in (0x04..=0x07).zip(0x1234_5678u32.to_be_bytes()) {
            rng.write(offset, byte);
        }
        let repeated = [rng.read(0), rng.read(1), rng.read(2), rng.read(3)];
        assert_ne!(first, seeded);
        assert_eq!(seeded, repeated);
    }

    #[test]
    fn trng_can_be_disabled() {
        let mut rng = RngDevice::new();
        rng.write(0x0D, 1);
        assert_eq!(rng.read(0x0C), 0);
        assert_eq!(rng.read(0x0D), 1);
    }
}