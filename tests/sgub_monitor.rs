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
