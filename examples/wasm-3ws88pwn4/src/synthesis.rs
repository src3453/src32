struct Synth<'a> {
    phase: [u64; 8],
    volume: [f32; 8],
    wave: [u8; 8],
    tables: &'a [[i8; 256]; 16],
    feedback_amount: f32,
    feedback_dest: usize,
    history: &'a mut [[f32; 2]; 8],
}

impl Synth<'_> {
    // Signed phase displacement in Q32 turns, matching the chip's phase domain.
    fn pm(value: f32) -> i64 {
        (value as f64 * 4.0 * 4294967296.0) as i64
    }

    fn op(&mut self, n: usize, shift: i64, direct: bool) -> f32 {
        // Operator outputs are already normalized by the envelope/register
        // volume path (approximately -1..1); average the two delayed samples.
        let fb_signal = (self.history[n][0] + self.history[n][1]) * 0.5;
        let fb_shift = if n == self.feedback_dest {
            Self::pm(fb_signal * self.feedback_amount)
        } else {
            0
        };
        let p = (if direct { 0 } else { self.phase[n] as i128 }) + shift as i128 + fb_shift as i128;
        let output = self.tables[self.wave[n] as usize][((p / (1i128 << 24)) & 255) as usize]
            as f32
            * self.volume[n];
        self.history[n][1] = self.history[n][0];
        self.history[n][0] = output;
        output
    }

    fn sum(&mut self, start: usize, end: usize) -> f32 {
        let mut value = 0.0;
        for n in start..end {
            value += self.op(n, 0, false);
        }
        value
    }
}

pub fn synth(
    mode: u8,
    phase: [u64; 8],
    volume: [f32; 8],
    wave: [u8; 8],
    tables: &[[i8; 256]; 16],
    feedback_amount: f32,
    feedback_dest: u8,
    history: &mut [[f32; 2]; 8],
) -> f32 {
    let mut s = Synth {
        phase,
        volume,
        wave,
        tables,
        feedback_amount,
        feedback_dest: if feedback_dest < 8 {
            feedback_dest as usize
        } else {
            usize::MAX
        },
        history,
    };
    let value = match mode {
        0 => s.sum(0, 8),
        1 | 10 => {
            let p = Synth::pm(s.sum(4, 8));
            let mut output = 0.0;
            for n in 0..4 {
                output += s.op(n, p, mode == 10);
            }
            output
        }
        2 => {
            let a = s.sum(0, 4);
            let b = s.sum(4, 8);
            a * b
        }
        3 | 11 => {
            let direct = mode == 11;
            let a = s.op(6, 0, false);
            let b = s.op(7, 0, false);
            let p = Synth::pm(a + b);
            let a = s.op(4, p, direct);
            let b = s.op(5, p, direct);
            let p = Synth::pm(a + b);
            let a = s.op(2, p, direct);
            let b = s.op(3, p, direct);
            let p = Synth::pm(a + b);
            s.op(0, p, direct) + s.op(1, p, direct)
        }
        4 | 12 => {
            let direct = mode == 12;
            let mut p = Synth::pm(s.op(7, 0, false));
            for n in (1..7).rev() {
                p = Synth::pm(s.op(n, p, direct));
            }
            s.op(0, p, direct)
        }
        5 | 7 => {
            let p = Synth::pm(s.op(7, 0, false));
            let p = Synth::pm(s.op(6, p, false));
            let p = Synth::pm(s.op(5, p, false));
            let upper = s.op(4, p, false);
            let p = Synth::pm(s.op(3, 0, false));
            let p = Synth::pm(s.op(2, p, false));
            let p = Synth::pm(s.op(1, p, false));
            let lower = s.op(0, p, false);
            if mode == 5 {
                upper + lower
            } else {
                upper * lower
            }
        }
        6 | 9 => {
            let mod1 = Synth::pm(s.op(1, 0, false));
            let op1 = s.op(0, mod1, false);
            let mod7 = Synth::pm(s.op(7, 0, false));
            let op7 = s.op(6, mod7, false);
            let mod5 = Synth::pm(s.op(5, 0, false));
            let op5 = s.op(4, mod5, false);
            let mod3 = Synth::pm(s.op(3, 0, false));
            let op3 = s.op(2, mod3, false);
            let pairs = [op1, op7, op5, op3];
            if mode == 6 {
                pairs.iter().sum()
            } else {
                pairs.iter().product()
            }
        }
        8 => {
            let a = s.op(0, 0, false) + s.op(1, 0, false);
            let b = s.op(2, 0, false) + s.op(3, 0, false);
            let c = s.op(4, 0, false) + s.op(5, 0, false);
            let d = s.op(6, 0, false) + s.op(7, 0, false);
            a * b * c * d
        }
        _ => 0.0,
    };
    value * 255.0 * 127.0
}
