use libm::{cosf, powf, sinf, sinhf, sqrtf};

#[derive(Clone, Copy)]
pub struct Biquad {
    a: [f32; 3],
    b: [f32; 3],
    x: [f32; 2],
    y: [f32; 2],
}
impl Default for Biquad {
    fn default() -> Self {
        Self {
            a: [1.0, 0.0, 0.0],
            b: [1.0, 0.0, 0.0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }
}
impl Biquad {
    pub fn process(&mut self, input: f32) -> f32 {
        let value = self.b[0] / self.a[0] * input
            + self.b[1] / self.a[0] * self.x[0]
            + self.b[2] / self.a[0] * self.x[1]
            - self.a[1] / self.a[0] * self.y[0]
            - self.a[2] / self.a[0] * self.y[1];
        self.x = [input, self.x[0]];
        self.y = [value, self.y[0]];
        value
    }
    pub fn channel(&mut self, mode: u8, cutoff: u8, resonance: u8) {
        let omega = 2.0 * 3.14159265 * (cutoff as f32 + 1.0) * 8.0 / 48000.0;
        let c = cosf(omega);
        let s = sinf(omega);
        let alpha = if mode == 2 || mode == 3 {
            s * sinhf(core::f32::consts::LN_2 * (resonance as f32 + 1.0) / 256.0 * omega / s)
        } else {
            s / (1.0 + (resonance as f32 + 1.0) / if mode <= 1 { 16.0 } else { 64.0 })
        };
        self.a = [1.0 + alpha, -2.0 * c, 1.0 - alpha];
        self.b = match mode {
            1 => [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0],
            2 => [alpha, 0.0, -alpha],
            3 => [1.0, -2.0 * c, 1.0],
            _ => [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0],
        };
    }
    pub fn detector(&mut self) {
        let omega = 2.0 * 3.14159265 * 50.0 / 48000.0;
        let c = cosf(omega);
        let alpha = sinf(omega) / 2.0;
        self.a = [1.0 + alpha, -2.0 * c, 1.0 - alpha];
        self.b = [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0];
    }
    pub fn eq(&mut self, band: usize, gain: u8) {
        // Original EQ uses the filter library's 44100 Hz default, even at 48 kHz.
        let omega = 2.0 * 3.14159265 * [400.0, 1000.0, 4000.0][band] / 44100.0;
        let c = cosf(omega);
        let s = sinf(omega);
        let q = 1.0 / sqrtf(2.0);
        let a = powf(10.0, (gain as f32 / 8.0) / 40.0);
        if band == 1 {
            let alpha = s * sinhf(core::f32::consts::LN_2 / 2.0 * q * omega / s);
            self.a = [1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a];
            self.b = [1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a];
        } else {
            let beta = sqrtf(a) / q;
            let sign = if band == 0 { 1.0 } else { -1.0 };
            self.a = [
                (a + 1.0) + sign * (a - 1.0) * c + beta * s,
                -sign * 2.0 * ((a - 1.0) + sign * (a + 1.0) * c),
                (a + 1.0) + sign * (a - 1.0) * c - beta * s,
            ];
            self.b = [
                a * ((a + 1.0) - sign * (a - 1.0) * c + beta * s),
                sign * 2.0 * a * ((a - 1.0) - sign * (a + 1.0) * c),
                a * ((a + 1.0) - sign * (a - 1.0) * c - beta * s),
            ];
        }
    }
}

#[derive(Default)]
pub struct Master {
    eq: [[Biquad; 3]; 2],
    detector: [Biquad; 2],
    gain: [f32; 2],
}
impl Master {
    pub fn configure(&mut self, regs: &[u8]) {
        if regs[1] == 1 {
            for channel in &mut self.eq {
                for (band, filter) in channel.iter_mut().enumerate() {
                    filter.eq(band, regs[5 + band]);
                }
            }
        }
        if regs[0] == 1 {
            for f in &mut self.detector {
                f.detector();
            }
        }
    }
    pub fn process(&mut self, mut input: [f32; 2], regs: &[u8]) -> [i16; 2] {
        for ch in 0..2 {
            if regs[1] == 1 {
                for filter in &mut self.eq[ch] {
                    input[ch] = filter.process(input[ch]);
                }
            }
            if regs[0] == 1 {
                let detected = self.detector[ch].process(input[ch].abs());
                let target = if detected == 0.0 {
                    256.0
                } else {
                    (regs[2] as f32 / 255.0) / detected
                };
                let delta = target - self.gain[ch];
                self.gain[ch] = (self.gain[ch]
                    + if delta > 0.0 {
                        delta.min(regs[3] as f32 / 255.0)
                    } else {
                        -(-delta).min(4.0)
                    })
                .clamp(0.0, 256.0);
                input[ch] *= regs[4] as f32 / 32.0 * self.gain[ch];
            }
        }
        input.map(|x| (x * 32767.0 / 4.0).clamp(-32768.0, 32767.0) as i16)
    }
}
