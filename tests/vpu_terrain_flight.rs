//! Real C guest regression. Build the WAX as documented in the example first.
use cpt32::bus::{Bus, connect_devices_with_vdp};
use cpt32::cpu::CYCLES_PER_FRAME;
use cpt32::cpu_select::{AnyCpu, CpuKind};
use cpt32::devices::pec::pec::{HostInputEvent, connect_pec_input};

#[test]
#[ignore = "requires target/terrain-c/vpu_terrain_flight.wax (LLVM/Clang build)"]
fn terrain_renders_and_held_keys_stop_on_release() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let image = std::fs::read(root.join("target/terrain-c/vpu_terrain_flight.wax"))
        .expect("build examples/vpu-terrain-flight-c first");
    for kind in [CpuKind::Cached, CpuKind::Pipelined] {
        let mut bus = Bus::new();
        let input = connect_pec_input(&mut bus);
        let vdp = connect_devices_with_vdp(&mut bus);
        let mut cpu = AnyCpu::new(kind, bus);
        let wax = cpt32::wax::parse(&image).unwrap();
        for block in &wax.blocks {
            cpu.load_program(block.load_address, block.data);
        }
        cpu.set_pc(wax.entry);
        let frames = |cpu: &mut AnyCpu, n: usize| {
            for _ in 0..n {
                vdp.borrow_mut().tick();
                cpu.set_irq_source(0, true);
                cpu.set_irq_source(0, false);
                cpu.run(CYCLES_PER_FRAME as usize);
            }
            // Finish the current draw without another host VSync, then sample
            // the complete frame while the guest waits for IRQ0.
            cpu.run(8 * CYCLES_PER_FRAME as usize);
            let vdp = vdp.borrow();
            let framebuffer = vdp.framebuffer();
            (0..240)
                .flat_map(|y| (0..320).map(move |x| (x, y)))
                .map(|(x, y)| framebuffer.get_pixel(x, y))
                .collect::<Vec<_>>()
        };
        let sequence = std::cell::Cell::new(0u16);
        let key = |cpu: &mut AnyCpu, usage, down| {
            input.send(HostInputEvent::Key { usage, down }).unwrap();
            // A mouse-position barrier confirms the asynchronous PeC worker
            // applied every preceding event before the next guest update.
            let marker = sequence.get() + 1;
            sequence.set(marker);
            input
                .send(HostInputEvent::MousePosition { x: marker, y: 0 })
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while cpu.read_mem_u32_be(0x80040040) != u32::from(marker) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "PeC input barrier timed out"
                );
                std::thread::yield_now();
            }
        };
        // Pause before the first guest update so images are deterministic.
        key(&mut cpu, 44, true);
        key(&mut cpu, 44, false);
        let first = frames(&mut cpu, 12);
        let stable = frames(&mut cpu, 8);
        let mut ppm = b"P6\n320 240\n255\n".to_vec();
        for &(r, g, b) in &stable {
            ppm.extend_from_slice(&[r, g, b]);
        }
        std::fs::write(root.join("target/terrain-c/terrain.ppm"), ppm).unwrap();
        assert!(
            first == stable,
            "paused rendering must remain stable ({kind:?})"
        );
        let green = stable
            .iter()
            .filter(|&&(r, g, b)| u16::from(g) * 4 > u16::from(r) * 5 && g > b + 8)
            .count();
        assert!(
            green > 5000,
            "terrain missing; green pixels={green} ({kind:?})"
        );
        assert!(
            stable[60 * 320 + 160].2 > stable[60 * 320 + 160].0,
            "sky missing"
        );
        assert!(
            stable.iter().all(|&rgb| rgb != (0, 0, 0)),
            "uncovered mesh/sky pixels"
        );
        assert_eq!(cpu.read_mem_u32_be(0x8003_0004) & 0x38, 0, "VPU FIFO error");
        assert_eq!(
            cpu.read_mem_u32_be(0x8001_0008) & 0x1e,
            0,
            "SGC FIFO/config error"
        );

        // Holding Right rotates every guest frame, including with motion paused.
        key(&mut cpu, 79, true);
        let turn1 = frames(&mut cpu, 8);
        let turn2 = frames(&mut cpu, 8);
        assert!(turn1 != stable, "turn key did nothing");
        assert!(turn1 != turn2, "holding a key must keep turning");
        key(&mut cpu, 79, false);
        let released = frames(&mut cpu, 8);
        assert!(
            released == frames(&mut cpu, 8),
            "key-up did not stop turning"
        );

        key(&mut cpu, 44, true);
        key(&mut cpu, 44, false);
        let moving = frames(&mut cpu, 8);
        assert!(
            released != moving,
            "resuming flight must move across terrain"
        );
        key(&mut cpu, 21, true);
        key(&mut cpu, 21, false);
        key(&mut cpu, 44, true);
        key(&mut cpu, 44, false);
        assert!(
            stable == frames(&mut cpu, 12),
            "reset must restore the seeded world"
        );
        assert!(cpu.is_running());

        println!(
            "{kind:?}: terrain={green} pixels; sky, pause, held key, release, flight, reset passed"
        );
    }
}
