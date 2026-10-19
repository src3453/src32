use cpt32_3ws88pwn4::chip::{Chip, PCM_SIZE, REG_BASE};
fn write(chip: &mut Chip, offset: u32, value: u8) {
    chip.write(REG_BASE + offset, value);
}
fn setup(scenario: usize) -> (Chip, Vec<u8>) {
    let mut chip = Chip::default();
    let mut pcm = vec![0; PCM_SIZE];
    for ch in 0..8 {
        write(&mut chip, ch * 64, 2);
        write(&mut chip, ch * 64 + 1, 238);
    }
    for ch in 0..4 {
        write(&mut chip, 0x200 + ch * 48, 2);
        write(&mut chip, 0x201 + ch * 48, 238);
    }
    if scenario < 29 {
        for op in 0..8 {
            if op != 0 {
                write(&mut chip, op * 2, (16 * (op + 1)) as u8);
                write(&mut chip, op * 2 + 1, 0);
            }
            write(&mut chip, 16 + op, 80 + (op * 9) as u8);
            write(&mut chip, 32 + op * 4, 0);
            write(&mut chip, 33 + op * 4, 2);
            write(&mut chip, 34 + op * 4, 190);
            write(&mut chip, 35 + op * 4, 2);
        }
        let wave = if scenario < 13 { 0 } else { scenario - 13 };
        for i in 0..4 {
            write(&mut chip, 24 + i, (wave * 17) as u8);
        }
        write(
            &mut chip,
            28,
            if scenario < 13 { scenario as u8 } else { 3 },
        );
        write(&mut chip, 29, 0xa7);
        write(&mut chip, 30, 1);
        write(&mut chip, 31, 128);
        for ch in 0..4 {
            for i in 0..32 {
                write(
                    &mut chip,
                    0x210 + ch * 48 + i,
                    ((i * 7 + ch * 31) % 256) as u8,
                );
            }
        }
    } else {
        let s = scenario - 29;
        let mode = if s < 6 { s } else { 1 };
        write(&mut chip, 0x202, 200);
        write(&mut chip, 0x203, mode as u8);
        write(&mut chip, 0x207, 0xf9);
        for i in 0..32 {
            write(&mut chip, 0x210 + i, (i * 7) as u8);
        }
        if mode == 0 {
            for i in 0..256 {
                pcm[i] = (i * 11 % 256) as u8;
            }
            for (i, value) in [0, 0, 0, 0, 0, 255, 0, 0, 32].iter().enumerate() {
                write(&mut chip, 0x210 + i as u32, *value);
            }
        }
        if mode == 5 {
            for value in [0, 255, 64, 192] {
                write(&mut chip, 0x300, value);
            }
        }
        if (6..10).contains(&s) {
            write(&mut chip, 0x204, (s - 6) as u8);
            write(&mut chip, 0x205, 90);
            write(&mut chip, 0x206, 45);
        }
        if (10..14).contains(&s) {
            write(&mut chip, 0x2c1, 1);
            write(&mut chip, 0x2c5, 24);
            write(&mut chip, 0x2c6, 16);
            write(&mut chip, 0x2c7, 32);
        }
        if (14..18).contains(&s) {
            write(&mut chip, 0x2c0, 1);
            write(&mut chip, 0x2c2, 80);
            write(&mut chip, 0x2c3, 150);
            write(&mut chip, 0x2c4, 32);
        }
        if s == 18 {
            write(&mut chip, 0x2d8, 1);
        }
        if s == 19 {
            write(&mut chip, 0x2d8, 2);
        }
    }
    (chip, pcm)
}

#[test]
fn legacy_reference_comparison() {
    let data = include_bytes!("legacy.pcm");
    assert_eq!(data.len(), 49 * 512 * 4);
    for scenario in 0..49 {
        let (mut chip, pcm) = setup(scenario);
        let mut squares = 0u64;
        let mut max = 0;
        for frame in 0..512 {
            let actual = chip.sample(&pcm);
            for ch in 0..2 {
                let at = (scenario * 512 + frame) * 4 + ch * 2;
                let expected = i16::from_le_bytes([data[at], data[at + 1]]);
                let error = (actual[ch] as i32 - expected as i32).abs();
                max = max.max(error);
                squares += (error as u64).pow(2);
            }
        }
        let rms = (squares as f64 / 1024.0).sqrt();
        println!("scenario {scenario}: max {max}, rms {rms:.3}");
        let tolerance = if (39..47).contains(&scenario) { 1 } else { 0 };
        assert!(
            max <= tolerance,
            "scenario {scenario}: rms {rms}, max {max}"
        );
    }
}

#[test]
fn gate_rewrites_restart_envelopes_and_wt_sync_rewinds_playback() {
    let (mut chip, pcm) = setup(0);
    let first: Vec<_> = (0..1024).map(|_| chip.sample(&pcm)).collect();
    // Feedback is deliberately retained by ResetGate, like the original function.
    write(&mut chip, 31, 128);
    write(&mut chip, 30, 1);
    let second: Vec<_> = (0..512).map(|_| chip.sample(&pcm)).collect();
    write(&mut chip, 30, 1);
    let third: Vec<_> = (0..512).map(|_| chip.sample(&pcm)).collect();
    assert_eq!(second[1..], third[1..]);
    assert!(first.iter().any(|x| *x != [0, 0]));
    write(&mut chip, 30, 0);
    for _ in 0..3000 {
        chip.sample(&pcm);
    }
    assert_eq!(chip.sample(&pcm), [0, 0]);
    let (mut chip, pcm) = setup(33);
    let first: Vec<_> = (0..512).map(|_| chip.sample(&pcm)).collect();
    for _ in 0..311 {
        chip.sample(&pcm);
    }
    write(&mut chip, 0x208, 99);
    let reset: Vec<_> = (0..512).map(|_| chip.sample(&pcm)).collect();
    assert_eq!(first, reset);
    assert_eq!(chip.read(REG_BASE + 0x208), 0);
}

#[test]
fn reset_and_zero_frequency_are_deterministic_and_fifo_is_bounded() {
    let mut chip = Chip::default();
    for _ in 0..100 {
        assert_eq!(chip.sample(&[]), [0, 0]);
    }
    for i in 0..4100 {
        write(&mut chip, 0x300, i as u8);
    }
    assert_eq!(chip.read(REG_BASE + 0x301), 16);
    assert_eq!(chip.read(REG_BASE + 0x302), 0);
    write(&mut chip, 0x203, 5);
    chip.sample(&[]);
    assert_eq!(chip.read(REG_BASE + 0x301), 15);
    assert_eq!(chip.read(REG_BASE + 0x302), 255);
    assert_eq!(chip.read(REG_BASE + 0x400), 0);
    chip.write(u32::MAX, 255);
    chip = Chip::default();
    assert_eq!(chip.read(REG_BASE + 0x301), 0);
}
