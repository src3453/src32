#include "cmd.h"
#include "chip8_core.h"
#include <foundation/fb.h>
#include <foundation/sgu.h>
#include <foundation/keyboard.h>

static chip8_vm vm;
static uint8_t frame[320 * 240];
/* Physical HID usages in CHIP-8 keypad order: 0,1,...,F. */
static const uint8_t usages[16] = {27,30,31,32,20,26,8,4,22,7,29,6,33,21,9,25};
static uint16_t key_bits(const uint32_t bits[8]) {
    uint16_t result = 0;
    for (uint32_t k = 0; k < 16; ++k)
        if (bits[usages[k] / 32] & (1u << (usages[k] % 32))) result |= (uint16_t)(1u << k);
    return result;
}
static fnd_status put_at(fnd_handle h, uint32_t off, const void *data, uint32_t size) {
    fnd_status st = fnd_sgu_seek(h, off);
    uint32_t put = 0;
    if (st == FND_OK) st = fnd_write(h, data, size, &put);
    return st == FND_OK && put != size ? FND_E_IO : st;
}
static fnd_status sound_init(fnd_handle h) {
    uint8_t wave[256], regs[32] = {0};
    for (uint32_t i = 0; i < 256; ++i) wave[i] = i < 128 ? 0xE0 : 0x20;
    /* SGU frequency register is in Hz. Wavetable, centered pan, no modulation. */
    regs[0] = 1; regs[1] = 0xB8; regs[4] = 0xFF;
    fnd_status st = put_at(h, 0, wave, sizeof wave);
    return st == FND_OK ? put_at(h, 0x800, regs, sizeof regs) : st;
}
static fnd_status render(fnd_handle fb) {
    /* 5x integer pixels: 320x160, with 40-pixel black top/bottom bars. */
    for (uint32_t y = 0; y < 32; ++y)
        for (uint32_t dy = 0; dy < 5; ++dy)
            for (uint32_t x = 0; x < 64; ++x)
                for (uint32_t dx = 0; dx < 5; ++dx)
                    frame[(40 + y * 5 + dy) * 320 + x * 5 + dx] = vm.pixels[y * 64 + x];
    uint32_t put = 0;
    fnd_status st = fnd_fb_seek(fb, 0);
    if (st == FND_OK) st = fnd_write(fb, frame, sizeof frame, &put);
    if (st == FND_OK && put != sizeof frame) st = FND_E_IO;
    vm.dirty = 0;
    return st;
}
static fnd_status load_rom(const char *path) {
    fnd_handle h;
    fnd_status st = fnd_file_open(path, FND_OPEN_READ, FND_MODE_DEFAULT, &h);
    if (st < 0) return st;
    uint32_t total = 0, got;
    do {
        st = fnd_read(h, vm.mem + 0x200 + total, 3584 - total, &got);
        if (st != FND_OK) break;
        total += got;
    } while (got && total < 3584);
    if (st == FND_OK && total == 3584) {
        uint8_t extra;
        st = fnd_read(h, &extra, 1, &got);
        if (st == FND_OK && got) st = FND_E_LIMIT;
    }
    fnd_close(h);
    return st == FND_OK && !total ? FND_E_FORMAT : st;
}
static int32_t chip8(uint32_t argc) {
    if (argc != 2) return cmd_usage("chip8 ROM.ch8 (Esc to quit)");
    uint32_t hz = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_HZ);
    if (hz < 60 || hz > 1000000) return cmd_report("chip8", 0, FND_E_BAD_ARG);
    chip8_init(&vm, (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO));
    fnd_status st = load_rom(fnd_argv(1));
    if (st < 0) return cmd_report("chip8", fnd_argv(1), st);
    fnd_handle fb = 0, sound = 0;
    st = fnd_file_open("/dev/sgu", FND_OPEN_WRITE, FND_MODE_DEFAULT, &sound);
    if (st == FND_OK) st = sound_init(sound);
    if (st == FND_OK) st = fnd_file_open("/dev/fb0", FND_OPEN_WRITE, FND_MODE_DEFAULT, &fb);
    fnd_fb_info info;
    if (st == FND_OK) st = fnd_fb_get_info(fb, &info);
    if (st == FND_OK && (info.width != 320 || info.height != 240 || info.format != FND_FB_INDEX8))
        st = FND_E_UNSUPPORTED;
    const uint8_t palette[6] = {0,0,0,255,255,255};
    if (st == FND_OK) st = fnd_fb_clut_write(fb, 0, 2, palette);
    if (st == FND_OK) st = render(fb);
    uint32_t last = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO), cpu_phase = 0;
    uint8_t sounding = 0;
    fnd_keyboard keyboard;
    if (st == FND_OK) st = fnd_con_key_state(&keyboard); /* discard old edges */
    while (st == FND_OK) {
        st = fnd_con_key_state(&keyboard);
        if (st != FND_OK) break;
        /* Physical Esc, plus queued Esc so a short press/release is retained. */
        int quit = ((keyboard.down[1] | keyboard.pressed[1]) & (1u << 9)) != 0;
        fnd_key_t key;
        for (uint32_t k = 0; k < 256; ++k) {
            fnd_status ks = fnd_con_pollkey(&key);
            if (ks == FND_END) break;
            if (ks < 0) { st = ks; break; }
            if (key.code == FND_KEY_ESC) quit = 1;
        }
        if (quit || st != FND_OK) break;
        chip8_keys(&vm, key_bits(keyboard.down), key_bits(keyboard.pressed), key_bits(keyboard.released));
        uint32_t now = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO), elapsed = now - last;
        last = now;
        /* Bound catch-up after host pauses to 250 ms; timers still see all elapsed ticks. */
        uint32_t steps = elapsed;
        if (steps > hz / 4) steps = hz / 4;
        /* Saturate 8-bit timers without looping over an unbounded elapsed time. */
        if (elapsed > hz * 5u) { vm.delay = vm.sound = vm.sound_gate = 0; elapsed = hz; }
        for (uint32_t t = 0; t < elapsed; ++t) chip8_tick(&vm, hz);
        cpu_phase += steps * 700u;
        uint32_t budget = cpu_phase / hz;
        cpu_phase %= hz;
        for (uint32_t s = 0; s < budget; ++s) {
            int result = chip8_step(&vm);
            if (result < 0) { st = FND_E_FORMAT; break; }
            if (result > 0) break; /* no catch-up burst after key/draw waits */
        }
        uint8_t on = vm.sound_gate; /* original VIP ignores a timer set to 1 */
        if (st == FND_OK && sounding != on) {
            uint8_t volume = on ? 128 : 0;
            st = put_at(sound, 0x803, &volume, 1); sounding = on;
        }
        if (st == FND_OK && vm.dirty) st = render(fb);
        if (st == FND_OK) st = fnd_sys_sleep(1);
    }
    if (sound) fnd_close(sound);
    if (fb) fnd_close(fb);
    return st < 0 ? cmd_report("chip8", fnd_argv(1), st) : FND_OK;
}
CMD_MAIN(chip8)
