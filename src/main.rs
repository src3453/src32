// Main entry point for the CPT32 emulator

use sdl2::{
    event::Event,
    keyboard::Keycode,
    pixels::PixelFormatEnum,
};

use cpt32::bus::Bus;
use cpt32::devices::vdp::vdp::{connect_vdp, VDP_VRAM_BASE};
use cpt32::devices::ram::connect_ram;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;

fn main() {
    // =========================
    // SDL初期化
    // =========================
    let sdl = sdl2::init().unwrap();
    let video = sdl.video().unwrap();

    let window = video
        .window("CPT32", WIDTH * 2, HEIGHT * 2)
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

    // =========================
    // VDP
    // =========================
    let vdp = connect_vdp(&mut bus);

    // テストパターン
    for i in 0..(WIDTH * HEIGHT) as usize {
        bus.write_u8(VDP_VRAM_BASE.wrapping_add(i as u32), (i % 256) as u8);
    }

    let mut event_pump = sdl.event_pump().unwrap();

    // =========================
    // メインループ
    // =========================
    'running: loop {
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
                        let i = y * WIDTH as usize + x;
                        let v = fb[i]&0x3F; // 6ビットカラーを仮にグレースケールに変換

                        let offset = y * pitch + x * 3;

                        // 仮: グレースケール
                        buf[offset] = v*4;
                        buf[offset + 1] = v*4;
                        buf[offset + 2] = v*4;
                    }
                }
            })
            .unwrap();

        canvas.clear();
        canvas.copy(&texture, None, None).unwrap();
        canvas.present();
    }
}