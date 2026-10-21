use cpt32::bus::Bus;
use cpt32::devices::expansion::{
    EXPANSION_BASE, ExpansionCardPackage, load_expansion_cards_from_bytes,
};

#[test]
#[ignore = "build/package examples/wasm-psg and set CPT32_PSG_CARD_PACKAGE"]
fn built_psg_card_renders_without_faults_and_resets() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .try_init();
    let path = std::env::var("CPT32_PSG_CARD_PACKAGE").expect("set CPT32_PSG_CARD_PACKAGE");
    let mut bus = Bus::new();
    load_expansion_cards_from_bytes(
        vec![ExpansionCardPackage {
            slot: 0,
            label: path.clone(),
            bytes: std::fs::read(path).unwrap(),
        }],
        &mut bus,
    )
    .unwrap();
    let base = EXPANSION_BASE;
    assert_eq!(bus.read_debug_u8(base + 0xf0), Some(b'P'));
    let mut left = [0; 4096];
    let mut right = left;
    bus.mix_expansion_audio(48000, &mut left, &mut right);
    assert!(left.iter().chain(&right).all(|&x| x == 0));

    // 3 tones + noise + envelope exercise the external crate's complete render
    // path under the real host's import checks, memory limit and fuel budget.
    for model in [0, 1] {
        bus.write_u8(base + 0x12, model);
        for (r, v) in [
            (0, 254),
            (1, 0),
            (2, 190),
            (3, 0),
            (4, 127),
            (5, 0),
            (6, 17),
            (7, 0),
            (8, 15),
            (9, 13),
            (10, 16),
            (11, 32),
            (12, 0),
            (13, 10),
        ] {
            bus.write_u8(base + r, v);
        }
        for rate in [48000, 44100, 8000, 96000] {
            left.fill(0);
            right.fill(0);
            let cycles = bus.system_cycles();
            bus.mix_expansion_audio(rate, &mut left, &mut right);
            assert_eq!(bus.system_cycles(), cycles);
            assert!(left.iter().any(|&x| x != 0), "model={model} rate={rate}");
            assert_eq!(left, right); // upstream default: centered mono
            assert_eq!(bus.read_u8(base), 254, "card faulted during render");
            assert!(!bus.take_bus_error());
        }
    }
    bus.write_u8(base + 0x10, 0xf1);
    bus.write_u8(base + 0x11, 0xff);
    assert_eq!(bus.read_debug_u8(base + 1), Some(15));
    assert_eq!(bus.read_u8(base + 0x11), 15);
    bus.reset_expansion_cards();
    left.fill(0);
    right.fill(0);
    bus.mix_expansion_audio(44100, &mut left, &mut right);
    assert!(left.iter().chain(&right).all(|&x| x == 0));
    assert_eq!(bus.read_u8(base + 7), 0x3f);
    assert_eq!(bus.read_u8(base + 0x12), 0);
    assert!(!bus.take_bus_error());
}
