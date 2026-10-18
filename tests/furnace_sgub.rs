use cpt32::devices::sgu::s3w2::S3w2Sound;
use cpt32::music::StreamPlayer;
use std::cell::RefCell;
use std::rc::Rc;

// Produced by the Furnace test/sgub/smoke.py A4 pitch macro fixture.
// Only its unused PCM initialization records were removed; all frame writes
// are verbatim exporter output. Keep the reset strobes and initial silent tick.
const STREAM: &[u8] = include_bytes!("fixtures/furnace_pitch.sgub");
// User-provided Furnace v2 export: two shared wave pools, a full PCM silence
// FILL, and one note at 131 Hz. Preserve the complete file for interoperability.
const V2_STREAM: &[u8] = include_bytes!("fixtures/furnace_minimal_v2.sgub");

#[test]
fn furnace_v2_sample_plays_pool_wave_and_initializes_pcm_ram() {
    let chips = std::array::from_fn(|_| Rc::new(RefCell::new(S3w2Sound::new())));
    let mut player = StreamPlayer::from_bytes(V2_STREAM).unwrap();
    player.advance_vsync(&chips).unwrap();
    let mut chip = chips[0].borrow_mut();
    assert_eq!(chip.channels[0].frequency, 131);
    assert_eq!(chip.channels[0].volume, 255);
    for index in 0..256 {
        assert_eq!(
            chip.read_channel_wavetable(0, index as u8),
            V2_STREAM[328 + index]
        );
    }
    assert_eq!(chip.read_pcm_ram(0xfffff), 128);
    let (left, right) = chip.clock_mixed(800);
    assert!(left.iter().any(|&sample| sample != 0));
    assert_eq!(left, right);
}

fn guest_driver() -> &'static [u8] {
    static DRIVER: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    DRIVER.get_or_init(|| {
        use std::process::Command;
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let build = repo
            .join("target")
            .join(format!("sgub-driver-test-{}", std::process::id()));
        std::fs::create_dir_all(&build).unwrap();
        let asm = build.join("driver.asm");
        let bin = build.join("driver.bin");
        let python = if cfg!(windows) { "python" } else { "python3" };
        for args in [
            vec![
                repo.join("tools/solc/solc.py"),
                "compile".into(),
                // Match the existing launcher profile's source entry point.
                // The root entry must include the canonical v1/v2 driver.
                repo.join("sgu_music_driver.sol"),
                "-O".into(),
                "speed".into(),
                "-o".into(),
                asm.clone(),
            ],
            vec![repo.join("tools/asm/asm.py"), asm, "-o".into(), bin.clone()],
        ] {
            let result = Command::new(python)
                .args(args)
                .current_dir(repo)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "guest build failed: {}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        std::fs::read(bin).unwrap()
    })
}

type Guest = (
    cpt32::cpu_select::AnyCpu,
    Rc<RefCell<cpt32::devices::vdp::vdp::Vdp>>,
);

fn load_guest(stream: &[u8]) -> Guest {
    use cpt32::bus::{Bus, connect_devices_with_vdp};
    use cpt32::cpu_select::{AnyCpu, CpuKind};
    let mut bus = Bus::new();
    let vdp = connect_devices_with_vdp(&mut bus);
    let mut cpu = AnyCpu::new(CpuKind::Pipelined, bus);
    cpu.load_program(0, guest_driver());
    cpu.load_program(cpt32::music::guest::STREAM_BASE, stream);
    (cpu, vdp)
}

fn boot_guest(stream: &[u8]) -> Guest {
    StreamPlayer::from_bytes(stream).unwrap();
    let (mut cpu, vdp) = load_guest(stream);
    // Wait for the driver's IRQ enable, rather than guessing that a HALT is
    // startup success: rejected streams also HALT and must fail this assertion.
    for _ in 0..50 {
        cpu.run(1_000_000);
        if cpu.read_mem_u8(0xffff0045) & 1 != 0 {
            return (cpu, vdp);
        }
    }
    panic!(
        "SGUB guest did not validate and initialize within 50 million cycles: pc={:x}, running={}, valid={}",
        cpu.pc(),
        cpu.is_running(),
        cpu.read_mem_u32_be(0x100000)
    );
}

fn guest_vsync((cpu, vdp): &mut Guest) {
    let frame = cpu.read_mem_u32_be(0x100010);
    vdp.borrow_mut().tick();
    cpu.set_irq_source(0, true);
    cpu.set_irq_source(0, false);
    // Dense reset frames can span more than one CPU frame budget with the
    // default size-optimized Sol compiler. Signal only this host VSync and
    // wait for its handler to return before inspecting the completed writes.
    for _ in 0..50 {
        cpu.run(cpt32::cpu::CYCLES_PER_FRAME as usize);
        if cpu.irq_enabled() && cpu.read_mem_u32_be(0x100010) != frame {
            return;
        }
    }
    panic!("SGUB IRQ failed to complete");
}

#[test]
fn guest_driver_plays_real_furnace_v2_offsets_and_full_pcm_fill() {
    let mut guest = boot_guest(V2_STREAM);
    let cpu = &mut guest.0;
    assert_eq!(cpu.read_mem_u8(0x180fffff), 128);
    guest_vsync(&mut guest);
    let cpu = &mut guest.0;
    assert_eq!(cpu.read_mem_u8(0x80020801), 131);
    assert_eq!(cpu.read_mem_u8(0x80020803), 255);
    assert_eq!(cpu.read_mem_u8(0x80021803), 0);
    for index in 0..256 {
        assert_eq!(
            cpu.read_mem_u8(0x80020000 + index),
            V2_STREAM[328 + index as usize]
        );
    }
    guest_vsync(&mut guest);
    assert_eq!(guest.0.read_mem_u8(0x80020803), 255);
}

#[test]
fn guest_driver_plays_furnace_pcm_export() {
    let stream = include_bytes!("fixtures/furnace_pcm_v2.sgub");
    let chips = std::array::from_fn(|_| Rc::new(RefCell::new(S3w2Sound::new())));
    let mut reference = StreamPlayer::from_bytes(stream).unwrap();
    let mut guest = boot_guest(stream);
    for frame in 0..8 {
        reference.advance_vsync(&chips).unwrap();
        guest_vsync(&mut guest);
        for chip in 0..2 {
            for addr in 0x800..=0x8ff {
                assert_eq!(
                    guest
                        .0
                        .read_mem_u8(0x80020000 + chip as u32 * 0x1000 + addr),
                    chips[chip].borrow_mut().read_register(addr),
                    "frame {frame}, chip {chip}, register {addr:x}"
                );
            }
        }
    }
}

#[test]
fn guest_driver_retains_v1_pitch_macro_playback() {
    let mut guest = boot_guest(STREAM);
    for frequency in [0u16, 440, 466, 415] {
        guest_vsync(&mut guest);
        let cpu = &mut guest.0;
        assert_eq!(
            u16::from_be_bytes([cpu.read_mem_u8(0x80020800), cpu.read_mem_u8(0x80020801)]),
            frequency
        );
    }
}

fn memory_operation(opcode: u8, target: u8, address: u32, args: &[u8]) -> Vec<u8> {
    let mut op = vec![opcode, target];
    op.extend_from_slice(&address.to_be_bytes()[1..]);
    op.extend_from_slice(args);
    op
}

fn pcm_copy(address: u32, source: u32, length: u32) -> Vec<u8> {
    let mut args = vec![0, 0]; // pool ID 0
    args.extend_from_slice(&source.to_be_bytes());
    args.extend_from_slice(&length.to_be_bytes());
    memory_operation(2, 2, address, &args)
}

fn fill(target: u8, address: u32, length: u32, value: u8) -> Vec<u8> {
    let mut args = length.to_be_bytes().to_vec();
    args.push(value);
    memory_operation(1, target, address, &args)
}

fn pcm_pool_stream() -> Vec<u8> {
    let pool = [11, 22, 33, 44, 55, 66, 77];
    let init = [fill(2, 0x120, 37, 128), pcm_copy(0x140, 2, 3)].concat();
    let frame = [
        pcm_copy(0x123, 1, 5),
        fill(0, 0x100, 19, 7),
        memory_operation(0, 0, 0x803, &[9]),
        memory_operation(0, 0, 0x803, &[11]),
        fill(2, 0x128, 3, 85),
    ]
    .concat();
    let init_start = 40 + 16 + pool.len() as u32;
    let frame_start = init_start + init.len() as u32;
    let total = frame_start + 2 + frame.len() as u32;
    let mut stream = b"SGUB\x02\x10\x01\0".to_vec();
    for value in [1, 1, 2, 40, init_start, frame_start, total] {
        stream.extend_from_slice(&value.to_be_bytes());
    }
    stream.extend_from_slice(&[0, 40, 0, 0]);
    stream.extend_from_slice(&[0, 0, 2, 0]); // PCM pool directory entry
    for value in [pool.len() as u32, pool.len() as u32, 56] {
        stream.extend_from_slice(&value.to_be_bytes());
    }
    stream.extend_from_slice(&pool);
    stream.extend_from_slice(&init);
    stream.extend_from_slice(&[0, 5]);
    stream.extend_from_slice(&frame);
    stream
}

#[test]
fn guest_v2_copies_pcm_subranges_handles_fill_tails_and_does_not_reinitialize_on_repeat() {
    let stream = pcm_pool_stream();
    let mut guest = boot_guest(&stream);
    assert_eq!(guest.0.read_mem_u8(0x18000120), 128);
    assert_eq!(guest.0.read_mem_u8(0x18000140), 33);
    assert_eq!(guest.0.read_mem_u8(0x18000142), 55);
    guest_vsync(&mut guest);
    for (index, value) in [22, 33, 44, 55, 66, 85, 85, 85, 128].iter().enumerate() {
        assert_eq!(guest.0.read_mem_u8(0x18000123 + index as u32), *value);
    }
    for index in 0..19 {
        assert_eq!(guest.0.read_mem_u8(0x80020100 + index), 7);
    }
    assert_eq!(guest.0.read_mem_u8(0x80020113), 128);
    assert_eq!(guest.0.read_mem_u8(0x80020803), 11);
    guest.0.write_mem_u8(0x18000140, 90);
    // The sole frame repeats with index returning to 1, so inspect a finite
    // CPU interval here rather than waiting for a changed frame index.
    guest.1.borrow_mut().tick();
    guest.0.set_irq_source(0, true);
    guest.0.set_irq_source(0, false);
    guest.0.run(cpt32::cpu::CYCLES_PER_FRAME as usize);
    assert_eq!(guest.0.read_mem_u8(0x18000140), 90);
    assert_eq!(guest.0.read_mem_u8(0x18000123), 22);
}

#[test]
fn guest_v2_rejects_bad_offsets_and_overflowing_ranges_before_initial_upload() {
    let good = pcm_pool_stream();
    // Invalid directory size/count, payload offset, FILL length and COPY_POOL
    // source must not wrap signed arithmetic into an accepted address range.
    for (offset, value) in [
        (12, u32::MAX),
        (16, u32::MAX),
        (52, u32::MAX),
        (68, 0x7fffffff),
        (80, u32::MAX),
    ] {
        let mut stream = good.clone();
        stream[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        assert!(StreamPlayer::from_bytes(&stream).is_err());
        let (mut cpu, _) = load_guest(&stream);
        cpu.run(1_000_000);
        assert_eq!(
            cpu.read_mem_u8(0xffff0045),
            0,
            "accepted invalid field at {offset}"
        );
        assert_eq!(
            cpu.read_mem_u8(0x18000120),
            0,
            "performed initialization before rejecting field at {offset}"
        );
    }
}

#[test]
fn furnace_register_stream_plays_pitch_macro_and_silences_unused_chip() {
    let chips = std::array::from_fn(|_| Rc::new(RefCell::new(S3w2Sound::new())));
    chips[1].borrow_mut().channels[0].volume = 255;
    let mut player = StreamPlayer::from_bytes(STREAM).unwrap();
    for frequency in [0, 440, 466, 415, 440, 440, 440, 440] {
        player.advance_vsync(&chips).unwrap();
        assert_eq!(chips[0].borrow().channels[0].frequency, frequency);
        assert!(chips[1].borrow().channels.iter().all(|ch| ch.volume == 0));
        if frequency != 0 {
            let (left, _) = chips[0].borrow_mut().clock_mixed(800);
            assert!(left.iter().any(|&sample| sample != 0));
        }
    }
    player.advance_vsync(&chips).unwrap();
    assert!(chips.iter().all(|chip| {
        chip.borrow()
            .channels
            .iter()
            .all(|ch| ch.volume == 0 && ch.pcm_control == 0)
    }));
}

#[test]
fn furnace_whole_song_repeat_replays_frame_zero_resets() {
    let mut stream = STREAM.to_vec();
    stream[6] = 1;
    let chips = std::array::from_fn(|_| Rc::new(RefCell::new(S3w2Sound::new())));
    let mut player = StreamPlayer::from_bytes(&stream).unwrap();
    for _ in 0..8 {
        player.advance_vsync(&chips).unwrap();
    }
    chips[1].borrow_mut().channels[0].volume = 255;
    player.advance_vsync(&chips).unwrap();
    assert_eq!(chips[0].borrow().channels[0].frequency, 0);
    assert_eq!(chips[0].borrow().channels[0].volume, 0);
    assert_eq!(chips[1].borrow().channels[0].volume, 0);
    player.advance_vsync(&chips).unwrap();
    assert_eq!(chips[0].borrow().channels[0].frequency, 440);
}
