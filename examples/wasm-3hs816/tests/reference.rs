use cpt32_3hs816::chip::{Chip, ID, ID_BASE};

/// Replays tests/script.txt and compares every sample with the output of the
/// C++ model (tests/reference.pcm, rendered by tests/reference.cpp).
#[test]
fn matches_cpp_reference_bit_exactly() {
    let script = include_str!("script.txt");
    let expected = include_bytes!("reference.pcm");
    let mut chip = Chip::default();
    let mut rendered = Vec::with_capacity(expected.len());
    for line in script.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("w") => {
                let address = u32::from_str_radix(parts.next().unwrap(), 16).unwrap();
                let value = u8::from_str_radix(parts.next().unwrap(), 16).unwrap();
                chip.write(address, value);
            }
            Some("r") => {
                let frames: usize = parts.next().unwrap().parse().unwrap();
                for _ in 0..frames {
                    for sample in chip.sample() {
                        rendered.extend_from_slice(&sample.to_le_bytes());
                    }
                }
            }
            other => panic!("bad script line {other:?}"),
        }
    }
    assert_eq!(rendered.len(), expected.len());
    if let Some(index) = rendered
        .iter()
        .zip(expected.iter())
        .position(|(a, b)| a != b)
    {
        panic!("first mismatch at stereo frame {}", index / 4);
    }
    let loud = expected
        .chunks_exact(2)
        .filter(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs() > 1000)
        .count();
    assert!(loud > expected.len() / 8, "fixture is mostly silent");
}

#[test]
fn id_is_read_only_and_outside_reads_zero() {
    let mut chip = Chip::default();
    for (i, byte) in ID.iter().enumerate() {
        assert_eq!(chip.read(ID_BASE + i as u32), *byte);
        chip.write(ID_BASE + i as u32, 0x55);
        assert_eq!(chip.read(ID_BASE + i as u32), *byte);
    }
    chip.write(0x4A0, 7);
    assert_eq!(chip.read(0x4A0), 7);
    assert_eq!(chip.read(0x500), 0);
    chip.write(0x500, 1);
    assert_eq!(chip.read(0x500), 0);
}

/// src/tables.rs must equal the C++ generated header it was converted from.
#[test]
fn tables_match_cpp_header() {
    let header = include_str!("../../../chips/3hs816/src/generated_tables.hpp");
    let rust = include_str!("../src/tables.rs");
    // Numbers between `open` (after `name`) and the next `close`.
    let numbers = |text: &str, name: &str, open: &str, close: &str| -> Vec<i64> {
        let start = text.find(name).unwrap() + name.len();
        let body = &text[start + text[start..].find(open).unwrap() + open.len()..];
        body[..body.find(close).unwrap()]
            .lines()
            .map(|line| line.split("//").next().unwrap())
            .flat_map(|line| {
                line.split(|c: char| !(c.is_ascii_digit() || c == '-'))
                    .filter(|t| !t.is_empty() && *t != "-")
                    .map(|t| t.parse::<i64>().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let waves = numbers(header, "kWaves[16][256]", "=", "};");
    assert_eq!(waves.len(), 16 * 256);
    assert_eq!(waves, numbers(rust, "WAVES: [[i8; 256]; 16]", "=", "\n];"));
    let exp2 = numbers(header, "kExp2Neg[257]", "=", "};");
    assert_eq!(exp2.len(), 257);
    assert_eq!(exp2, numbers(rust, "EXP2_NEG: [u32; 257]", "=", "\n];"));
}
