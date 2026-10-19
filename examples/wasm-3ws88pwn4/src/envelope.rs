#[derive(Clone, Copy, Default)]
pub struct Envelope {
    pub stage: u8, // attack, decay, sustain, release
    elapsed: f64,
    pub level: f64,
    last: f64,
}
impl Envelope {
    pub fn trigger(&mut self) {
        self.stage = 0;
        self.elapsed = 0.0;
    }
    pub fn release(&mut self) {
        if self.stage != 3 {
            self.elapsed = 0.0;
            self.stage = 3;
        }
        self.last = self.level;
    }
    pub fn advance(&mut self, adsr: &[u8]) {
        let a = adsr[0] as f32 / 64.0 * 0.33;
        let d = if adsr[1] == 0 {
            0.5 / 64.0 * 1.5
        } else {
            adsr[1] as f32 / 64.0 * 1.5
        };
        let s = (adsr[2] as f32 / 255.0) as f64;
        let r = adsr[3] as f32 / 64.0 * 1.5;
        if self.stage == 0 {
            if self.elapsed < a as f64 {
                self.level = self.elapsed / a as f64;
                self.elapsed += (1.0f32 / 48000.0) as f64;
                return;
            }
            self.elapsed -= a as f64;
            self.stage = 1;
        }
        if self.stage == 1 {
            if self.elapsed < d as f64 {
                self.level = 1.0
                    + (s - 1.0)
                        * (1.0 - libm::powf(2.0, (-10.0 * self.elapsed / d as f64) as f32) as f64);
            } else {
                self.elapsed -= d as f64;
                self.stage = 2;
                self.level = s;
            }
        } else if self.stage == 2 {
            self.level = s;
        } else if self.stage == 3 {
            self.level = if self.elapsed < r as f64 {
                self.last * libm::powf(2.0, (-10.0 * self.elapsed / r as f64) as f32) as f64
            } else {
                0.0
            };
        }
        self.elapsed += (1.0f32 / 48000.0) as f64;
    }
}
