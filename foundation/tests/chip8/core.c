#include "../../bin/chip8_core.h"
#define CHECK(x) do { if (!(x)) return __LINE__; } while (0)
static chip8_vm c;
static int op(uint32_t code) {
    c.pc = 0x200; c.mem[0x200] = (uint8_t)(code >> 8); c.mem[0x201] = (uint8_t)code;
    return chip8_step(&c);
}
int test_arithmetic(void) {
    chip8_init(&c, 1);
    CHECK(op(0x61FF) == 0 && c.v[1] == 255);
    CHECK(op(0x7102) == 0 && c.v[1] == 1);
    c.v[1] = 200; c.v[2] = 100;
    CHECK(op(0x8124) == 0 && c.v[1] == 44 && c.v[15] == 1);
    c.v[1] = 2; c.v[2] = 3;
    CHECK(op(0x8125) == 0 && c.v[1] == 255 && c.v[15] == 0);
    CHECK(op(0x8127) == 0 && c.v[1] == 4 && c.v[15] == 0);
    c.v[1] = c.v[2] = 3;
    CHECK(op(0x8125) == 0 && c.v[1] == 0 && c.v[15] == 1);
    c.v[1] = 100; c.v[2] = 0x83;
    CHECK(op(0x8126) == 0 && c.v[1] == 0x41 && c.v[15] == 1 && c.v[2] == 0x83);
    CHECK(op(0x812E) == 0 && c.v[1] == 6 && c.v[15] == 1);
    c.v[15] = 0x83;
    CHECK(op(0x8FF6) == 0 && c.v[15] == 1);
    c.v[1] = 1; c.v[15] = 255;
    CHECK(op(0x81F4) == 0 && c.v[1] == 0 && c.v[15] == 1);
    for (uint32_t n = 1; n <= 3; ++n) {
        c.v[1] = 0xCC; c.v[2] = 0xAA; c.v[15] = 255;
        CHECK(op(0x8120 + n) == 0 && c.v[15] == 0);
        CHECK(c.v[1] == (n == 1 ? 0xEE : n == 2 ? 0x88 : 0x66));
    }
    CHECK(op(0x8120) == 0 && c.v[1] == c.v[2]);
    CHECK(op(0xC10F) == 0 && c.v[1] <= 15 && c.random != 1);
    return 0;
}
int test_memory_control(void) {
    chip8_init(&c, 7);
    c.v[0] = 12; c.v[1] = 34; c.v[2] = 255; c.i = 0x300;
    CHECK(op(0xF255) == 0 && c.i == 0x303 && c.mem[0x302] == 255);
    c.i = 0x300; c.v[0] = 0;
    CHECK(op(0xF265) == 0 && c.i == 0x303 && c.v[0] == 12);
    CHECK(op(0xF233) == 0 && c.mem[0x303] == 2 && c.mem[0x304] == 5 && c.mem[0x305] == 5);
    CHECK(op(0xF229) == 0 && c.i == 0x9B);
    CHECK(op(0xA400) == 0 && op(0xF21E) == 0 && c.i == 0x4FF);
    CHECK(op(0x300C) == 0 && c.pc == 0x204);
    CHECK(op(0x400C) == 0 && c.pc == 0x202);
    c.v[1] = c.v[0];
    CHECK(op(0x5010) == 0 && c.pc == 0x204);
    CHECK(op(0x9010) == 0 && c.pc == 0x202);
    CHECK(op(0xB300) == 0 && c.pc == 0x30C);
    CHECK(op(0x1234) == 0 && c.pc == 0x234);
    for (uint32_t i = 0; i < 12; ++i) CHECK(op(0x2345) == 0 && c.pc == 0x345);
    CHECK(op(0x2345) == -1);
    for (uint32_t i = 0; i < 12; ++i) CHECK(op(0x00EE) == 0 && c.pc == 0x202);
    CHECK(op(0x00EE) == -1);
    const uint32_t bad[] = {0x0123,0x00FD,0x5121,0x9121,0x8128,0xE199,0xF199};
    for (uint32_t i = 0; i < sizeof bad / sizeof *bad; ++i) CHECK(op(bad[i]) == -1);
    c.i = 4095;
    CHECK(op(0xF133) == -1 && op(0xF155) == -1 && op(0xF165) == -1);
    c.pc = 4095; CHECK(chip8_step(&c) == -1);
    return 0;
}
int test_draw_timing_keys(void) {
    chip8_init(&c, 1); c.i = 0x300; c.mem[c.i] = 0xFF;
    c.v[0] = 127; c.v[1] = 63;
    CHECK(op(0xD011) == 1 && c.pc == 0x200);
    chip8_tick(&c, 100); CHECK(c.draw_ready == 0);
    chip8_tick(&c, 100);
    CHECK(op(0xD011) == 0 && c.pixels[31 * 64 + 63] == 1 && c.pixels[31 * 64] == 0);
    CHECK(c.v[15] == 0 && op(0xD011) == 1);
    chip8_tick(&c, 100); chip8_tick(&c, 100);
    CHECK(op(0xD011) == 0 && c.pixels[2047] == 0 && c.v[15] == 1);
    CHECK(op(0x00E0) == 0 && c.dirty);
    c.delay = 100; c.sound = 100; c.timer_phase = 0;
    for (uint32_t i = 0; i < 100; ++i) chip8_tick(&c, 100);
    CHECK(c.delay == 40 && c.sound == 40 && c.timer_phase == 0);
    c.v[1] = 1; CHECK(op(0xF118) == 0 && c.sound == 1 && !c.sound_gate);
    c.v[1] = 2; CHECK(op(0xF118) == 0 && c.sound_gate);
    chip8_tick(&c, 60); CHECK(c.sound == 1 && c.sound_gate);
    chip8_tick(&c, 60); CHECK(c.sound == 0 && !c.sound_gate);
    c.v[1] = 5; CHECK(op(0xF115) == 0 && op(0xF107) == 0 && c.v[1] == 5);
    for (uint32_t k = 0; k < 16; ++k) {
        c.v[1] = (uint8_t)k; chip8_keys(&c, (uint16_t)(1u << k), 0, 0);
        CHECK(op(0xE19E) == 0 && c.pc == 0x204);
        CHECK(op(0xE1A1) == 0 && c.pc == 0x202);
        CHECK(op(0xF10A) == 1 && chip8_step(&c) == 1);
        chip8_keys(&c, 0, 0, (uint16_t)(1u << k));
        CHECK(c.wait_reg == -1 && c.v[1] == k);
    }
    chip8_keys(&c, 0, 0, 0); CHECK(op(0xF10A) == 1);
    chip8_keys(&c, 0, 4, 4); CHECK(c.wait_reg == -1 && c.v[1] == 2);
    return 0;
}
