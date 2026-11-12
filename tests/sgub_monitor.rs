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
