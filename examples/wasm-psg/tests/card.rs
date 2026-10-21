use cpt32_psg::PsgCard;
use cpt32_wasm_expansion_card::Card;

fn tone() -> PsgCard {
    let mut card = PsgCard::default();
    for (r, v) in [(0, 254), (1, 0), (7, 0x3e), (8, 15)] {
        card.mmio_write(r, v);
    }
    card
}

#[test]
fn ports_masks_snapshots_and_reset() {
    let mut card = tone();
    card.mmio_write(0x10, 0xf1);
    card.mmio_write(0x11, 0xff);
    assert_eq!(card.debug_read(1), Some(15));
    assert_eq!(card.mmio_read(0x10), 1);
    assert_eq!(card.mmio_read(0x11), 15);
    card.mmio_write(0x12, 0xff);
    assert_eq!(card.debug_read(0x12), Some(1));
    card.mmio_write(0x100, 255);
    assert_eq!(card.debug_read(0x100), None);
    assert_eq!(card.mmio_read(0x100), 0);
    card.reset();
    assert_eq!(card.mmio_read(7), 0x3f);
    assert_eq!(card.mmio_read(8), 0);
    assert_eq!(card.mmio_read(0x12), 0);
    let mut output = [[1; 2]; 512];
    card.render_audio(48000, &mut output);
    assert_eq!(output, [[0; 2]; 512]);
}

#[test]
fn tone_byte_order_does_not_change_zero_period_clamping() {
    let mut card = PsgCard::default();
    card.mmio_write(0, 0);
    card.mmio_write(1, 1);
    card.mmio_write(7, 0x3e);
    card.mmio_write(8, 15);
    let mut reference = psg::PSG::new(1_789_772.5, 48000).unwrap();
    reference.set_tone_period(0, 256);
    reference.set_mixer(0x3e);
    reference.set_amplitude(0, 15);
    let mut output = [[0; 2]; 1024];
    card.render_audio(48000, &mut output);
    for frame in output {
        let (l, r) = reference.render();
        assert_eq!(frame, [(l * 16384.0) as i16, (r * 16384.0) as i16]);
    }
}

#[test]
fn resampling_and_debug_reads_preserve_block_continuity() {
    for rate in [8000, 44100, 48000, 96000] {
        let mut whole = tone();
        let mut split = tone();
        let mut expected = [[0; 2]; 601];
        let mut actual = expected;
        whole.render_audio(rate, &mut expected);
        let mut cursor = 0;
        for count in [1, 7, 128, 3, 462] {
            for offset in 0..256 {
                let _ = split.debug_read(offset);
            }
            split.render_audio(rate, &mut actual[cursor..cursor + count]);
            cursor += count;
        }
        assert_eq!(expected, actual);
        assert!(actual.iter().any(|frame| frame[0] != 0));
    }
}

#[test]
fn repeated_shape_write_retriggers_envelope_in_both_models() {
    for model in [0, 1] {
        let mut card = tone();
        card.mmio_write(0x12, model);
        card.mmio_write(8, 16);
        card.mmio_write(11, 32);
        card.mmio_write(13, 9); // one-shot descending envelope, hold at zero
        let mut output = [[0; 2]; 48000];
        card.render_audio(48000, &mut output);
        assert!(output[..4000].iter().any(|x| x[0] != 0));
        assert!(output[47000..].iter().all(|x| x[0] == 0));
        card.mmio_write(13, 9);
        card.render_audio(48000, &mut output[..4000]);
        assert!(output[..4000].iter().any(|x| x[0] != 0));
    }
}
