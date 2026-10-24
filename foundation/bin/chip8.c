#include "cmd.h"
#include "chip8_core.h"
#include "chip8_options.h"
#include <foundation/libsgc.h>
#include <foundation/sgu.h>
#include <foundation/keyboard.h>

static chip8_vm vm;
/* SGC command buffer. Worst case is every other pixel of a row changing:
 * 1024 runs * (SET_COLOR + RECT) = 6144 words. */
#define QUEUE_WORDS 8192u
static uint32_t queue[QUEUE_WORDS];
static cpt_sgc sgc;
static uint8_t shown[64 * 32]; /* what the SGC plane shows once the queue drains */
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
/* 5x integer pixels: 320x160, with 40-pixel black top/bottom bars. Paints only
 * the pixels that differ from what the plane already shows, as one RECT per
 * horizontal run. SGC keeps its plane between frames, so nothing else is sent.
 * Only called with an empty queue, which always has room. */
static fnd_status queue_diff(void) {
    for (uint32_t y = 0; y < 32; ++y)
        for (uint32_t x = 0; x < 64;) {
            uint32_t i = y * 64 + x, v = vm.pixels[i];
            if (v == shown[i]) { ++x; continue; }
            uint32_t end = x;
            while (end < 64 && vm.pixels[y * 64 + end] == v && shown[y * 64 + end] != v) ++end;
            fnd_status st = cpt_sgc_rect(&sgc, (int32_t)(x * 5), (int32_t)(40 + y * 5), (int32_t)((end - x) * 5), 5, v);
            if (st != FND_OK) return st;
            for (uint32_t k = x; k < end; ++k) shown[y * 64 + k] = (uint8_t)v;
            x = end;
        }
    vm.dirty = 0;
    return FND_OK;
}
/* A full FIFO is not an error: the rest goes out on a later loop pass. */
static fnd_status render(void) {
    fnd_status st = FND_OK;
    if (!cpt_sgc_pending(&sgc)) st = queue_diff(); /* else the plane is still catching up */
    if (st == FND_E_WOULD_BLOCK) st = FND_OK; /* buffer full: vm.dirty stays set */
    return st == FND_OK ? cpt_sgc_flush(&sgc) : st;
}
static fnd_status clear_screen(void) {
    fnd_status st = cpt_sgc_clear(&sgc, 0);
    for (uint32_t i = 0; i < sizeof shown; ++i) shown[i] = 0;
    return st == FND_OK ? cpt_sgc_flush(&sgc) : st;
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
    if (argc < 2 || argc > 5) return cmd_usage(CHIP8_USAGE);
    const char *argv[5];
    for (uint32_t i = 0; i < argc; ++i) argv[i] = fnd_argv(i);
    chip8_options options;
    if (!chip8_parse_options(argc, argv, &options)) return cmd_usage(CHIP8_USAGE);
    uint32_t hz = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_HZ);
    if (hz < 60 || hz > 1000000) return cmd_report("chip8", 0, FND_E_BAD_ARG);
    chip8_init(&vm, (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO));
    fnd_status st = load_rom(options.rom);
    if (st < 0) return cmd_report("chip8", options.rom, st);
    fnd_handle sound = 0;
    st = fnd_file_open("/dev/sgu", FND_OPEN_WRITE, FND_MODE_DEFAULT, &sound);
    if (st == FND_OK) st = sound_init(sound);
    const uint8_t palette[6] = {0,0,0,255,255,255};
    /* fb0 only sets up the display and CLUT; every pixel is drawn through the SGC. */
    if (st == FND_OK) st = cpt_sgc_open(&sgc, queue, QUEUE_WORDS);
    if (st == FND_OK) st = cpt_sgc_display_init(&sgc, 0, 2, palette);
    if (st == FND_OK) st = clear_screen();
    if (st == FND_OK) st = render();
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
        uint32_t budget = chip8_cpu_budget(elapsed, hz, options.cycles, &cpu_phase);
        /* Saturate 8-bit timers without looping over an unbounded elapsed time. */
        if (elapsed > hz * 5u) { vm.delay = vm.sound = vm.sound_gate = 0; elapsed = hz; }
        for (uint32_t t = 0; t < elapsed; ++t) chip8_tick(&vm, hz);
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
        if (st == FND_OK && (vm.dirty || cpt_sgc_pending(&sgc))) st = render();
        if (st == FND_OK) st = fnd_sys_sleep(1);
    }
    if (sound) fnd_close(sound);
    cpt_sgc_close(&sgc);
    return st < 0 ? cmd_report("chip8", options.rom, st) : FND_OK;
}
CMD_MAIN(chip8)
