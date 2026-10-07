//! Headless SRC32 execution harness for compiler pipeline smoke tests.
use std::env;
use std::fs;

use cpt32::bus::Bus;
use cpt32::bus::connect_devices_with_vdp;
use cpt32::cpu::Cpu;

use cpt32::devices::pec::idc::idc::connect_idc_from_config;
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let path = args
        .first()
        .expect("usage: src32_testbench PROGRAM.bin [expected] [--allow-running]");
    let allow_running = args.iter().any(|arg| arg == "--allow-running");
    let expected: u32 = args
        .iter()
        .skip(1)
        .find(|arg| arg.as_str() != "--allow-running")
        .map_or(Ok(0), |value| value.parse())
        .expect("expected must be an unsigned integer");
    let image = fs::read(&path).expect("failed to read program binary");

    let mut bus = Bus::new();
    let vdp = connect_devices_with_vdp(&mut bus);
    connect_idc_from_config(&mut bus).expect("Invalid disk configuration");
    let mut cpu = Cpu::new(bus);
    cpu.load_program(0, &image);
    cpu.run(5_000_000);

    let result = cpu.read_reg(1);
    if cpu.is_running() {
        assert!(
            allow_running,
            "program did not halt (pc=0x{:08X})",
            cpu.pc()
        );
        let vpu_status = cpu.read_mem_u32_be(0x8003_0004);
        assert_eq!(
            vpu_status & 0x38,
            0,
            "VPU reported an error: 0x{vpu_status:08X}"
        );
        let vdp_ref = vdp.borrow();
        let framebuffer = vdp_ref.framebuffer();
        let has_cube_pixel =
            (0..240).any(|y| (0..320).any(|x| framebuffer.get_pixel(x, y) != (0, 0, 0)));
        assert!(
            has_cube_pixel,
            "VPU command stream produced no visible pixels"
        );
        println!(
            "PASS: {path} still running at cycle budget {}; VPU status=0x{vpu_status:08X}",
            cpu.cycles()
        );
    } else {
        assert_eq!(
            result, expected,
            "R1 mismatch: got {result}, expected {expected}"
        );
        println!("PASS: {path} => R1={result} cycles={}", cpu.cycles());
    }
}
