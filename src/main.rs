// Main entry point for the CPT32 emulator

use std::env;

use sdl2::{
    event::Event,
    keyboard::Keycode,
    pixels::PixelFormatEnum,
};

use cpt32::bus::Bus;
use cpt32::devices::vdp::vdp::{connect_vdp, VDP_VRAM_BASE};
use cpt32::devices::ram::connect_ram;
use cpt32::cpu::Cpu;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;

fn load_program(path: &str, bus: &mut Bus) {
    let data = std::fs::read(path).expect("Failed to read program file");
    for (i, byte) in data.iter().enumerate() {
        bus.write_u8(i as u32, *byte);
    }
}

fn main() {
    // =========================
    // SDL初期化
    // =========================
    let sdl = sdl2::init().unwrap();
    let video = sdl.video().unwrap();

    let window = video
        .window("CPT32", WIDTH*2, HEIGHT*2)
        .position_centered()
        .build()
        .unwrap();

    let mut canvas = window.into_canvas().accelerated().build().unwrap();

    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture_streaming(PixelFormatEnum::RGB24, WIDTH, HEIGHT)
        .unwrap();


    // =========================
    // バスとデバイスの初期化
    // =========================
    let mut bus = Bus::new();
    connect_ram(&mut bus); // RAMを接続

    let program_path = env::args().nth(1).expect("Usage: cargo run <program.bin>");
    load_program(&program_path, &mut bus); // プログラムをロード
    // =========================
    // VDP
    // =========================
    let vdp = connect_vdp(&mut bus);

    // テストパターン
    for i in 0..(WIDTH * HEIGHT) as usize {
        bus.write_u8(VDP_VRAM_BASE.wrapping_add(i as u32), (i % 256) as u8);
    }

    let mut event_pump = sdl.event_pump().unwrap();
    let mut cpu = Cpu::new(bus);
    // =========================
    // メインループ
    // =========================
    'running: loop {
        cpu.run(cpt32::cpu::CYCLES_PER_SCANLINE as usize);
        //cpu.run(1); // step実行
        // --- イベント ---
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                _ => {}
            }
        }

        // --- 描画 ---
        let vdp_ref = vdp.borrow();
        let fb = vdp_ref.framebuffer();

        texture
            .with_lock(None, |buf: &mut [u8], pitch: usize| {
                for y in 0..HEIGHT as usize {
                    for x in 0..WIDTH as usize {
                        let (r, g, b) = fb.get_pixel(x, y);
                        
                        let offset = y * pitch + x * 3;

                        // RGB24フォーマットでバッファに書き込む
                        buf[offset] = r;
                        buf[offset + 1] = g;
                        buf[offset + 2] = b;
                    }
                }
            })
            .unwrap();

        canvas.clear();
        canvas.copy(&texture, None, None).unwrap();
        canvas.present();
    }
}