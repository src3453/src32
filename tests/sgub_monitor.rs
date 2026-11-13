use cpt32::devices::sgu::s3w2::{PCM_RAM_SIZE, S3w2Sound, SguVersion, saturate_output};
use cpt32::music::StreamPlayer;
use cpt32::music::monitor::{Engine, FRAME_SAMPLES, Transport};
use std::cell::RefCell;
use std::rc::Rc;

const V2_STREAM: &[u8] = include_bytes!("fixtures/furnace_minimal_v2.sgub");
const V1_STREAM: &[u8] = include_bytes!("fixtures/furnace_pitch.sgub");

fn reference_cores() -> [Rc<RefCell<S3w2Sound>>; 2] {
    let pcm = Rc::new(RefCell::new(vec![0; PCM_RAM_SIZE]));
    std::array::from_fn(|_| {
        Rc::new(RefCell::new(S3w2Sound::with_version(
            pcm.clone(),
            SguVersion::V2_1,
        )))
    })
}

/// The monitor renders channels separately for meters and mutes; the unmuted
/// sum must still match the editor/emulator mixer bit for bit.
#[test]
fn per_channel_mix_matches_reference_mixer() {
    for stream in [V1_STREAM, V2_STREAM] {
        let mut engine = Engine::new(stream.to_vec(), SguVersion::V2_1).unwrap();
        let cores = reference_cores();
        let mut player = StreamPlayer::from_bytes(stream).unwrap();
        for _ in 0..30 {
            let (left, right) = engine.tick(FRAME_SAMPLES);
            player.advance_vsync(&cores).unwrap();
            let mut ref_l = vec![0i32; FRAME_SAMPLES];
            let mut ref_r = vec![0i32; FRAME_SAMPLES];
            for core in &cores {
                core.borrow_mut()
                    .clock_mixed_into_i32(&mut ref_l, &mut ref_r);
            }
            let ref_l: Vec<i16> = ref_l.into_iter().map(saturate_output).collect();
            let ref_r: Vec<i16> = ref_r.into_iter().map(saturate_output).collect();
            assert_eq!(left, ref_l);
            assert_eq!(right, ref_r);
        }
    }
}

#[test]
fn muting_a_voice_removes_it_and_pause_freezes_the_stream() {
    let mut engine = Engine::new(V2_STREAM.to_vec(), SguVersion::V2_1).unwrap();
    engine.muted = [true; 16];
    let (left, _) = engine.tick(FRAME_SAMPLES);
    assert!(left.iter().all(|&s| s == 0));
    assert!(engine.channel_meters[0].level > 0.0, "meters ignore mutes");
    engine.muted = [false; 16];
    engine.tick(FRAME_SAMPLES);
    assert!(engine.history(0).iter().any(|&s| s != 0));
    engine.transport = Transport::Pause;
    let frame = engine.frame_index();
    engine.tick(FRAME_SAMPLES);
    assert_eq!(engine.frame_index(), frame);
}

#[test]
fn seek_reaches_the_same_register_state_as_playing() {
    // 20 frames: past the end of the 8-frame non-repeating pitch fixture.
    // 200 frames: inside the 384-frame v2 fixture.
    for (stream, frames) in [(V1_STREAM, 20), (V2_STREAM, 200)] {
        check_seek(stream, frames);
    }
}

fn check_seek(stream: &[u8], frames: u64) {
    let mut played = Engine::new(stream.to_vec(), SguVersion::V2_1).unwrap();
    for _ in 0..frames {
        played.tick(FRAME_SAMPLES);
    }
    let mut seeked = Engine::new(stream.to_vec(), SguVersion::V2_1).unwrap();
    seeked.seek(frames);
    assert_eq!(seeked.finished(), played.finished());
    assert_eq!(seeked.elapsed_frames, played.elapsed_frames);
    assert_eq!(seeked.frame_index(), played.frame_index());
    for voice in 0..16 {
        let (a, b) = (seeked.channel(voice), played.channel(voice));
        assert_eq!(a.frequency, b.frequency, "voice {voice}");
        assert_eq!(a.volume, b.volume, "voice {voice}");
        assert_eq!(a.wavetable, b.wavetable, "voice {voice}");
    }
    seeked.stop();
    assert_eq!(seeked.transport, Transport::Stop);
    assert_eq!(seeked.elapsed_frames, 0);
}

// ---------------------------------------------------------------------------
// SGUB v4: frame period, 34SI22 and expansion cards.

use cpt32::music::monitor::{CardKind, CardStatus};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

const V4_MIX: &[u8] = include_bytes!("fixtures/furnace_v4_mix.sgub");

/// A stand-in for the 3HS816 card: registers are linear memory at the same
/// slot-relative addresses, `id` sits at 0x4F8, and the audio output is
/// +1000 on the left while channel 0's Gate (0x1E) is 1.
fn fake_card(id: &[u8]) -> String {
    let id: String = id.iter().map(|b| format!("\\{b:02x}")).collect();
    format!(
        r#"(module
  (memory (export "memory") 1)
  (data (i32.const 0x4f8) "{id}")
  (func (export "cpt32_abi_version") (result i32) i32.const 1)
  (func (export "cpt32_reset"))
  (func (export "cpt32_mmio_read") (param $a i32) (result i32)
    (if (result i32) (i32.lt_u (local.get $a) (i32.const 0x500))
      (then (i32.load8_u (local.get $a))) (else (i32.const 0))))
  (func (export "cpt32_mmio_write") (param $a i32) (param $v i32)
    (if (i32.lt_u (local.get $a) (i32.const 0x4f8))
      (then (i32.store8 (local.get $a) (local.get $v)))))
  (func (export "cpt32_tick"))
  (func (export "cpt32_audio_render") (param $n i32) (param $rate i32) (result i32)
    (local $i i32) (local $v i32)
    (local.set $v (select (i32.const 1000) (i32.const 0)
      (i32.eq (i32.load8_u (i32.const 0x1e)) (i32.const 1))))
    (block $done (loop $again
      (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
      (i32.store16 (i32.add (i32.const 0x1000) (i32.shl (local.get $i) (i32.const 2))) (local.get $v))
      (i32.store16 (i32.add (i32.const 0x1002) (i32.shl (local.get $i) (i32.const 2))) (i32.const 0))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $again)))
    (i32.const 0x1000)))"#
    )
}

fn write_package(path: &Path, module: &str) {
    use zip::write::SimpleFileOptions;
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    archive.start_file("manifest.yaml", options).unwrap();
    archive
        .write_all(b"abi_version: 1\nmodule: card.wasm\n")
        .unwrap();
    archive.start_file("card.wasm", options).unwrap();
    archive.write_all(&wat::parse_str(module).unwrap()).unwrap();
    std::fs::write(path, archive.finish().unwrap().into_inner()).unwrap();
}

fn temp_package(name: &str, module: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sgub_monitor_{}_{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    write_package(&path, module);
    path
}

#[test]
fn v4_mix_plays_card_and_si22_through_their_devices() {
    let package = temp_package("3hs816.cpt32card", &fake_card(b"3HS816\x01\x00"));
    let mut engine = Engine::with_cards(V4_MIX.to_vec(), SguVersion::V3_1, &mut |req, kind| {
        assert_eq!((req.slot, kind), (0, CardKind::Hs816));
        Ok(Some(package.clone()))
    })
    .unwrap();
    assert_eq!(engine.cards.len(), 1);
    assert_eq!(engine.cards[0].status, CardStatus::Ready);
    assert_eq!(engine.cards[0].kind, CardKind::Hs816);
    assert!(engine.uses_si22());
    let mut gate_seen = false;
    let mut card_audio = false;
    let mut si22_played = false;
    for _ in 0..40 {
        let (left, _) = engine.tick(FRAME_SAMPLES);
        let card = &engine.cards[0];
        gate_seen |= card.hs(0).gate;
        card_audio |= card.meters[0].level > 0.0 && left.iter().any(|&s| s != 0);
        si22_played |= engine.si22().play.iter().any(|p| p.status == 1);
    }
    let card = &engine.cards[0];
    assert!(card.writes > 0 && card.dropped == 0);
    assert_eq!(card.hs(0).feedback, 0x80, "frame 0 writes neutral feedback");
    assert!(gate_seen && card_audio && si22_played);
    assert_eq!(engine.stray_writes, 0);

    // Seeking resets the card and replays its writes.
    engine.seek(10);
    assert_eq!(engine.cards[0].hs(0).feedback, 0x80);
    std::fs::remove_dir_all(package.parent().unwrap()).unwrap();
}

/// Mixer gains scale each device before the shared saturation; unity keeps
/// the reference mix (checked by `per_channel_mix_matches_reference_mixer`).
#[test]
fn mixer_gains_scale_each_chip_and_card() {
    // SGU: 0 % silences a core, 200 % doubles every channel exactly.
    let mut unity = Engine::new(V2_STREAM.to_vec(), SguVersion::V2_1).unwrap();
    let mut doubled = Engine::new(V2_STREAM.to_vec(), SguVersion::V2_1).unwrap();
    let mut silent = Engine::new(V2_STREAM.to_vec(), SguVersion::V2_1).unwrap();
    doubled.sgu_gains = [200; 2];
    silent.sgu_gains = [0; 2];
    let mut heard = false;
    for _ in 0..60 {
        let (l1, r1) = unity.tick(FRAME_SAMPLES);
        let (l2, r2) = doubled.tick(FRAME_SAMPLES);
        let (l0, r0) = silent.tick(FRAME_SAMPLES);
        assert!(l1.iter().chain(&r1).all(|s| s.abs() < 16384), "no clipping");
        assert_eq!(l2, l1.iter().map(|s| s * 2).collect::<Vec<_>>());
        assert_eq!(r2, r1.iter().map(|s| s * 2).collect::<Vec<_>>());
        assert!(l0.iter().chain(&r0).all(|&s| s == 0));
        heard |= l1.iter().any(|&s| s != 0);
    }
    assert!(heard);
    assert!(
        silent.channel_meters.iter().any(|m| m.level > 0.0),
        "voice meters are pre-fader"
    );
    assert!(silent.sgu_meters.iter().flatten().all(|m| m.level == 0.0));
    assert!(unity.sgu_meters.iter().flatten().any(|m| m.level > 0.0));

    // 34SI22 and card: 0 % sounds like a mute; the fake card's +1000 becomes +500 at 50 %.
    let package = temp_package("3hs816.cpt32card", &fake_card(b"3HS816\x01\x00"));
    let open = || {
        Engine::with_cards(V4_MIX.to_vec(), SguVersion::V3_1, &mut |_, _| {
            Ok(Some(package.clone()))
        })
        .unwrap()
    };
    let (mut reference, mut si22_muted, mut si22_zero, mut card_half) =
        (open(), open(), open(), open());
    si22_muted.si22_muted = true;
    si22_zero.si22_gain = 0;
    card_half.cards[0].gain = 50;
    let mut card_seen = false;
    for _ in 0..40 {
        let (left, right) = reference.tick(FRAME_SAMPLES);
        assert_eq!(
            si22_zero.tick(FRAME_SAMPLES),
            si22_muted.tick(FRAME_SAMPLES)
        );
        let (half_l, half_r) = card_half.tick(FRAME_SAMPLES);
        assert_eq!(half_r, right);
        for (full, half) in left.iter().zip(&half_l) {
            let diff = full - half;
            assert!(
                diff == 0 || diff == 500,
                "card adds 1000 or nothing: {diff}"
            );
            card_seen |= diff == 500;
        }
    }
    assert!(card_seen);
    std::fs::remove_dir_all(package.parent().unwrap()).unwrap();
}

#[test]
fn missing_or_mismatched_card_drops_its_writes_but_keeps_playing() {
    let mut missing =
        Engine::with_cards(V4_MIX.to_vec(), SguVersion::V3_1, &mut |_, _| Ok(None)).unwrap();
    let wrong = temp_package("wrong.cpt32card", &fake_card(b"XXXXXX\x01\x00"));
    let mut mismatched = Engine::with_cards(V4_MIX.to_vec(), SguVersion::V3_1, &mut |_, _| {
        Ok(Some(wrong.clone()))
    })
    .unwrap();
    assert_eq!(missing.cards[0].status, CardStatus::Missing);
    assert_eq!(mismatched.cards[0].status, CardStatus::IdMismatch);
    for engine in [&mut missing, &mut mismatched] {
        for _ in 0..10 {
            engine.tick(FRAME_SAMPLES);
        }
        assert!(engine.cards[0].dropped > 0);
        assert_eq!(engine.cards[0].writes, 0);
        assert!(engine.error.is_none());
        assert_eq!(engine.elapsed_frames, 10);
    }
    std::fs::remove_dir_all(wrong.parent().unwrap()).unwrap();
}

/// Minimal v4 stream: no cards, no pools, `frames` frames that each write
/// SGU0 ch0 VOLUME = frame number + 1.
fn timer_stream(frames: u32, period: u32, repeat: bool) -> Vec<u8> {
    let mut body = Vec::new();
    for frame in 0..frames {
        body.extend_from_slice(&1u16.to_be_bytes());
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(&0x803u32.to_be_bytes());
        body.push(frame as u8 + 1);
    }
    let total = 48 + body.len() as u32;
    let mut bytes = b"SGUB".to_vec();
    bytes.extend_from_slice(&[4, 16, u8::from(repeat), 0]);
    for value in [frames, 0, 0, 48, 48, 48, total] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&48u16.to_be_bytes());
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend_from_slice(&period.to_be_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&body);
    bytes
}

#[test]
fn timer_streams_advance_on_their_frame_period() {
    // 120 Hz: two frames per 800-sample host tick.
    let mut engine = Engine::new(timer_stream(8, 400_000, false), SguVersion::V3_1).unwrap();
    assert_eq!(engine.frame_period(), 400_000);
    assert_eq!(engine.frames_per_second(), 120.0);
    engine.tick(FRAME_SAMPLES);
    assert_eq!(engine.elapsed_frames, 2);
    assert_eq!(engine.channel(0).volume, 2);
    engine.tick(FRAME_SAMPLES);
    assert_eq!(engine.elapsed_frames, 4);
    assert_eq!(engine.channel(0).volume, 4);

    // A period that is not a whole number of samples carries the remainder:
    // frames at t = 0, 480.5, 961, ... samples, so 100 frames start within 1 s.
    let mut engine = Engine::new(timer_stream(4, 480_500, true), SguVersion::V3_1).unwrap();
    for _ in 0..60 {
        engine.tick(FRAME_SAMPLES);
    }
    assert_eq!(engine.elapsed_frames, 100);
    assert_eq!(engine.loops, 24);
}

/// v4 stream without cards: `pools` are wave entries, `initial` and the
/// single frame are raw v4 operations.
fn wave_pool_stream(pools: &[[u8; 256]], initial: &[Vec<u8>], frame: &[Vec<u8>]) -> Vec<u8> {
    let directory = 48;
    let mut payload = directory + 16 * pools.len();
    let mut body = Vec::new();
    for (id, _) in pools.iter().enumerate() {
        body.extend_from_slice(&(id as u16).to_be_bytes());
        body.extend_from_slice(&[1, 0]);
        body.extend_from_slice(&256u32.to_be_bytes());
        body.extend_from_slice(&256u32.to_be_bytes());
        body.extend_from_slice(&(payload as u32).to_be_bytes());
        payload += 256;
    }
    for pool in pools {
        body.extend_from_slice(pool);
    }
    let initial_offset = 48 + body.len();
    for op in initial {
        body.extend_from_slice(op);
    }
    let frame_offset = 48 + body.len();
    body.extend_from_slice(&(frame.len() as u16).to_be_bytes());
    for op in frame {
        body.extend_from_slice(op);
    }
    let total = 48 + body.len();
    let mut bytes = b"SGUB".to_vec();
    bytes.extend_from_slice(&[4, 16, 0, 0]);
    for value in [
        1,
        pools.len(),
        initial.len(),
        directory,
        initial_offset,
        frame_offset,
        total,
    ] {
        bytes.extend_from_slice(&(value as u32).to_be_bytes());
    }
    bytes.extend_from_slice(&48u16.to_be_bytes());
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend_from_slice(&[0; 8]);
    bytes.extend_from_slice(&body);
    bytes
}

fn write_op(target: u8, address: u32, value: u8) -> Vec<u8> {
    let mut op = vec![0, target];
    op.extend_from_slice(&address.to_be_bytes());
    op.push(value);
    op
}

fn copy_op(target: u8, address: u32, pool: u16) -> Vec<u8> {
    let mut op = vec![2, target];
    op.extend_from_slice(&address.to_be_bytes());
    op.extend_from_slice(&pool.to_be_bytes());
    op.extend_from_slice(&0u32.to_be_bytes());
    op.extend_from_slice(&256u32.to_be_bytes());
    op
}

/// Wave entries: entry n holds byte value `n * 16 + i` (a ramp).
fn ramp_pools(count: usize) -> Vec<[u8; 256]> {
    (0..count)
        .map(|n| std::array::from_fn(|i| (n * 16 + i) as u8))
        .collect()
}

#[test]
fn channel_view_reports_wave_pool_entry_and_3ws8pnb_length() {
    let pools = ramp_pools(3);
    let initial = [
        copy_op(0, 0x000, 2),                    // SGU0 ch0 <- pool 2
        copy_op(0, 0x100, 1),                    // SGU0 ch1 <- pool 1
        copy_op(1, 0x300, 2),                    // SGU1 ch3 <- pool 2 (shared entry)
        copy_op(0, 0x200, 0),                    // SGU0 ch2 <- pool 0, overwritten in frame 0
        write_op(0, 0x800 + 0x20 + 0x1A, 3),     // ch1: 32 samples
        write_op(0, 0x800 + 0x40 + 0x1A, 2 | 4), // ch2: 64 samples, hold
        write_op(1, 0x800 + 0x60 + 0x1A, 1),     // SGU1 ch3: 128 samples
    ];
    let frame = [write_op(0, 0x205, 0xEE)];
    let stream = wave_pool_stream(&pools, &initial, &frame);

    let mut engine = Engine::new(stream.clone(), SguVersion::V3_1).unwrap();
    engine.tick(FRAME_SAMPLES);
    let view = |voice| engine.channel(voice);
    assert_eq!(view(0).wave_pool, Some(2));
    assert_eq!((view(0).wave_length, view(0).interpolate), (256, true));
    assert_eq!(view(1).wave_pool, Some(1));
    assert_eq!((view(1).wave_length, view(1).interpolate), (32, true));
    // A WRITE after the copy means the SRAM no longer is that entry.
    assert_eq!(view(2).wave_pool, None);
    assert_eq!((view(2).wave_length, view(2).interpolate), (64, false));
    assert_eq!(view(11).wave_pool, Some(2));
    assert_eq!((view(11).wave_length, view(11).interpolate), (128, true));
    // Never copied.
    assert_eq!(view(4).wave_pool, None);

    // Seek rebuilds the tracking from a fresh player.
    engine.seek(1);
    assert_eq!(engine.channel(1).wave_pool, Some(1));

    // Before 3WS8PNB SAMPLE_CONTROL is ignored: 256 samples.
    let mut engine = Engine::new(stream, SguVersion::V3).unwrap();
    engine.tick(FRAME_SAMPLES);
    assert_eq!(engine.channel(1).wave_length, 256);
    assert!(engine.channel(2).interpolate);
}

#[test]
fn wave_preview_draws_exactly_the_playing_samples() {
    let mut pools = ramp_pools(1);
    // Mark the bytes past each shorter length so leaking them would show.
    for (i, byte) in pools[0].iter_mut().enumerate() {
        *byte = if i == 0 { 0 } else { 0xFF };
    }
    pools[0][31] = 0x80;
    let mut initial = Vec::new();
    for (channel, control) in [0u32, 1, 2, 3, 4 | 3].into_iter().enumerate() {
        initial.push(copy_op(0, channel as u32 * 0x100, 0));
        initial.push(write_op(
            0,
            0x800 + channel as u32 * 0x20 + 0x1A,
            control as u8,
        ));
    }
    let mut engine =
        Engine::new(wave_pool_stream(&pools, &initial, &[]), SguVersion::V3_1).unwrap();
    engine.tick(FRAME_SAMPLES);
    let (width, height) = (128.0, 24.0);
    for (voice, length) in [(0, 256usize), (1, 128), (2, 64), (3, 32)] {
        let view = engine.channel(voice);
        assert_eq!(view.wave_length, length);
        let points = view.wave_points(width, height);
        // One point per sample plus the wrap back to sample 0 at the right edge.
        assert_eq!(points.len(), length + 1, "length {length}");
        let step = width / length as f32;
        for (i, [x, _]) in points.iter().enumerate() {
            assert_eq!(*x, i as f32 * step);
        }
        assert_eq!(points[0][1], height);
        assert_eq!(points[length][1], height, "wraps to sample 0");
        assert_eq!(points[1][1], 0.0);
    }
    // Same FREQUENCY, shorter table: one octave up per halving.
    let note = |voice: usize| {
        let mut view = engine.channel(voice);
        view.frequency = 220;
        view.midi_note().unwrap()
    };
    assert_eq!(note(0), 57.0);
    assert_eq!(note(1), 69.0);
    assert_eq!(note(2), 81.0);
    assert_eq!(note(3), 93.0);
    // 32 samples held: a step per sample, only bytes 0..31 drawn.
    let view = engine.channel(4);
    assert_eq!((view.wave_length, view.interpolate), (32, false));
    let points = view.wave_points(width, height);
    assert_eq!(points.len(), 64);
    assert_eq!(points[0], [0.0, height]);
    assert_eq!(points[1], [4.0, height]);
    assert_eq!(points[2], [4.0, 0.0]);
    let last = points[63];
    assert_eq!(last[0], width);
    assert_eq!(last[1], height - 128.0 / 255.0 * height);
}

/// v4 stream with no pools: `initial` operations, then one frame of `frame`.
fn ops_stream(initial: &[Vec<u8>], frame: &[Vec<u8>]) -> Vec<u8> {
    wave_pool_stream(&[], initial, frame)
}

/// Register writes that point SGU0 `channel` at `start..=end` and start it.
fn pcm_voice(channel: u32, waveform: u8, start: u32, end: u32, looped: bool) -> Vec<Vec<u8>> {
    let base = 0x800 + channel * 0x20;
    let mut ops = vec![write_op(0, base + 2, waveform)];
    for (offset, value) in [(0x10, start), (0x13, end), (0x16, start)] {
        for (i, byte) in value.to_be_bytes()[1..].iter().enumerate() {
            ops.push(write_op(0, base + offset + i as u32, *byte));
        }
    }
    ops.push(write_op(0, base, 0x01)); // FREQUENCY 0x100
    ops.push(write_op(0, base + 3, 0xFF));
    ops.push(write_op(0, base + 0x19, 1 | if looped { 2 } else { 0 }));
    ops
}

#[test]
fn pcm_overview_covers_start_through_end_and_progress_follows_playback() {
    // 256 bytes at 0x1000: a ramp, so column c of 128 holds bytes 2c and 2c+1.
    let mut initial: Vec<Vec<u8>> = (0..256u32)
        .map(|i| write_op(2, 0x1000 + i, i as u8))
        .collect();
    initial.extend(pcm_voice(0, 1, 0x1000, 0x10FF, true));
    let mut engine = Engine::new(ops_stream(&initial, &[]), SguVersion::V3_1).unwrap();
    // Initial operations apply on the first frame.
    engine.tick(FRAME_SAMPLES);
    let overview = engine.pcm_overview(0, 128);
    assert_eq!(overview.len(), 128);
    for (column, pair) in overview.iter().enumerate() {
        assert_eq!(*pair, [2 * column as u8, 2 * column as u8 + 1]);
    }
    // More columns than bytes: each column still shows one byte.
    let wide = engine.pcm_overview(0, 512);
    assert_eq!(wide[0], [0, 0]);
    assert_eq!(wide[511], [255, 255]);

    let (position, loop_start) = engine.channel(0).pcm_progress().unwrap();
    assert!(position > 0.0 && position <= 1.0, "{position}");
    assert_eq!(loop_start, 0.0);
    // Wavetable voices have no PCM progress.
    assert_eq!(engine.channel(1).pcm_progress(), None);
}

#[test]
fn sqv4_progress_uses_decoded_length_and_voice_history_records_output() {
    use cpt32::devices::sgu::sqv4::{Variant, encode_interleaved};
    let pcm: Vec<i16> = (0..4000)
        .map(|i| ((i as f32 * 0.05).sin() * 12000.0) as i16)
        .collect();
    let file = encode_interleaved(&pcm, 1, 48_000, Variant::H, 255).unwrap();
    let mut initial: Vec<Vec<u8>> = file
        .iter()
        .enumerate()
        .map(|(i, &byte)| write_op(2, 0x2000 + i as u32, byte))
        .collect();
    let end = 0x2000 + file.len() as u32;
    initial.extend(pcm_voice(0, 4, 0x2000, end, false));
    let mut engine = Engine::new(ops_stream(&initial, &[]), SguVersion::V3_1).unwrap();
    assert!(engine.voice_history(0).iter().all(|&s| s == 0));
    assert_eq!(
        engine.sqv4_overview(0, 128),
        None,
        "not loaded before frame 0"
    );
    engine.tick(FRAME_SAMPLES);
    let view = engine.channel(0);
    assert_eq!(view.sqv4_samples.map(|(n, _)| n), Some(4000));
    let (position, _) = view.pcm_progress().unwrap();
    assert!(position > 0.0 && position < 1.0, "{position}");
    // The core validated the file, so the whole sample decodes for the preview.
    let overview = engine.sqv4_overview(0, 128).unwrap();
    assert_eq!(overview.len(), 128);
    let low = overview.iter().map(|[low, _]| *low).min().unwrap();
    let high = overview.iter().map(|[_, high]| *high).max().unwrap();
    assert!(low < -10_000 && high > 10_000, "{low}..{high}");
    // Cached result is identical; other waveforms have none.
    assert_eq!(engine.sqv4_overview(0, 128).unwrap(), overview);
    assert_eq!(engine.sqv4_overview(1, 128), None);
    let recent = engine.voice_history(0);
    assert_eq!(recent.len(), cpt32::music::monitor::VOICE_HISTORY);
    assert!(recent.iter().any(|&s| s > 500) && recent.iter().any(|&s| s < -500));
    // Seek clears the per-voice history with the rest of the monitor state.
    engine.seek(0);
    assert!(engine.voice_history(0).iter().all(|&s| s == 0));
}
