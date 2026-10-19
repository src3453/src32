// Algorithms retain the original operator routing and feedback injection points.
pub fn synth(
    mode: u8,
    phase: [u64; 8],
    volume: [f32; 8],
    wave: [u8; 8],
    tables: &[[i8; 256]; 16],
    feedback: f32,
) -> f32 {
    let op = |n: usize, shift: i64, direct: bool| -> f32 {
        let p = (if direct { 0 } else { phase[n] as i128 }) + shift as i128;
        tables[wave[n] as usize][((p / (1i128 << 24)) & 255) as usize] as f32 * volume[n]
    };
    // Signed phase displacement in Q32 turns. Integer division intentionally
    // truncates negative table indices toward zero, as in the legacy core.
    // Audio amplitudes remain floating point; no floating phase is accumulated.
    let pm = |x: f32| (x as f64 * 4.0 * 4294967296.0) as i64;
    let a = |n| op(n, 0, false);
    let sum = |lo, hi| (lo..hi).map(a).sum::<f32>();
    let value = match mode {
        0 => sum(0, 8) + feedback,
        1 | 10 => {
            let p = pm(sum(4, 8) + feedback);
            (0..4).map(|n| op(n, p, mode == 10)).sum()
        }
        2 => sum(0, 4) * sum(4, 8) + feedback,
        3 | 11 => {
            let direct = mode == 11;
            let p = pm(a(6) + a(7));
            let p = pm(op(4, p, direct) + op(5, p, direct));
            let p = pm(op(2, p, direct) + op(3, p, direct) + feedback);
            op(0, p, direct) + op(1, p, direct)
        }
        4 | 12 => {
            let direct = mode == 12;
            let mut p = pm(a(7));
            for n in (1..7).rev() {
                p = pm(op(n, p, direct) + if n == 1 { feedback } else { 0.0 });
            }
            op(0, p, direct)
        }
        5 | 7 => {
            let p = pm(a(7));
            let p = pm(op(6, p, false));
            let p = pm(op(5, p, false));
            let upper = op(4, p, false);
            let p = pm(a(3));
            let p = pm(op(2, p, false));
            let p = pm(op(1, p, false) + feedback);
            let lower = op(0, p, false);
            if mode == 5 {
                upper + lower
            } else {
                upper * lower
            }
        }
        6 | 9 => {
            let pairs = [
                op(0, pm(a(1) + feedback), false),
                op(6, pm(a(7)), false),
                op(4, pm(a(5)), false),
                op(2, pm(a(3)), false),
            ];
            if mode == 6 {
                pairs.iter().sum()
            } else {
                pairs.iter().product()
            }
        }
        8 => (a(0) + a(1)) * (a(2) + a(3)) * (a(4) + a(5)) * (a(6) + a(7)) + feedback,
        _ => 0.0,
    };
    value * 255.0 * 127.0
}
