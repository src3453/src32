/* Foundation platform glue. Engine sources and game data are external inputs. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <foundation/fb.h>
#include <foundation/keyboard.h>
#include <foundation/sys.h>
#include "doomgeneric.h"
#include "doomkeys.h"
#include "i_video.h"

static fnd_handle fb;
static uint32_t tick_hz, start_tick, frame_limit, frames;
static unsigned next_usage;
static fnd_keyboard keyboard;
static uint8_t held[256];
static uint8_t rgb[320 * 200 * 3];

static void cleanup(void) {
    if (fb) { fnd_close(fb); fb = 0; }
}

static void check(fnd_status status) {
    if (status < 0) {
        cleanup();
        fprintf(stderr, "doom: %s\n", fnd_strerror(status));
        exit(1);
    }
}

static void write_all(const void *data, uint32_t bytes) {
    const uint8_t *p = data;
    while (bytes) {
        uint32_t put = 0;
        check(fnd_write(fb, p, bytes, &put));
        if (!put || put > bytes) check(FND_E_IO);
        p += put; bytes -= put;
    }
}

void DG_Init(void) {
    tick_hz = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_HZ);
    if (!tick_hz || tick_hz > 1000000u) check(FND_E_BAD_ARG);
    start_tick = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO);
    if (!DG_ScreenBuffer || atexit(cleanup)) check(FND_E_NO_MEMORY);
    check(fnd_file_open("/dev/fb0", FND_OPEN_WRITE, FND_MODE_DEFAULT, &fb));
    fnd_fb_info info;
    check(fnd_fb_get_info(fb, &info));
    if (info.width != 320 || info.height != 240) check(FND_E_UNSUPPORTED);
    check(fnd_fb_set_format(fb, FND_FB_RGB888));
    /* RGB888 leaves the shared SGC/GP CLUT intact. Black bars at y=0..19,
     * 220..239; the engine's native 320x200 image is centred between them. */
    check(fnd_fb_seek(fb, 0));
    write_all(rgb, 320 * 20 * 3);
    check(fnd_fb_seek(fb, 320 * 220 * 3));
    write_all(rgb, 320 * 20 * 3);
    check(fnd_con_key_state(&keyboard));
}

void DG_DrawFrame(void) {
    for (uint32_t i = 0; i < 320 * 200; ++i) {
        struct color c = colors[DG_ScreenBuffer[i]];
        rgb[i * 3] = c.r; rgb[i * 3 + 1] = c.g; rgb[i * 3 + 2] = c.b;
    }
    check(fnd_fb_seek(fb, 320 * 20 * 3));
    write_all(rgb, sizeof rgb);
    if (frame_limit && ++frames >= frame_limit) {
        cleanup();
        puts("doom: frame limit reached");
        exit(0);
    }
}

uint32_t DG_GetTicksMs(void) {
    uint32_t ticks = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO) - start_tick;
    return (uint32_t)((uint64_t)ticks * 1000u / tick_hz);
}

void DG_SleepMs(uint32_t ms) {
    if (ms) {
        uint64_t ticks = ((uint64_t)ms * tick_hz + 999u) / 1000u;
        if (ticks >= FND_FOREVER) ticks = FND_FOREVER - 1u;
        check(fnd_sys_sleep((uint32_t)ticks));
    } else fnd_sys_yield();
}

static unsigned char key_for(unsigned usage) {
    if (usage >= 4 && usage <= 29) return (unsigned char)('a' + usage - 4);
    if (usage >= 30 && usage <= 38) return (unsigned char)('1' + usage - 30);
    if (usage >= 58 && usage <= 67) return (unsigned char)(KEY_F1 + usage - 58);
    switch (usage) {
    case 39: return '0'; case 40: return KEY_ENTER; case 41: return KEY_ESCAPE;
    case 42: return KEY_BACKSPACE; case 43: return KEY_TAB; case 44: return KEY_USE;
    case 45: return KEY_MINUS; case 46: return KEY_EQUALS;
    case 54: return KEY_STRAFE_L; case 55: return KEY_STRAFE_R;
    case 68: return KEY_F11; case 69: return KEY_F12; case 72: return KEY_PAUSE;
    case 79: return KEY_RIGHTARROW; case 80: return KEY_LEFTARROW;
    case 81: return KEY_DOWNARROW; case 82: return KEY_UPARROW;
    case 224: return KEY_FIRE;
    case 225: return KEY_RSHIFT;
    case 226: return KEY_RALT;
    default: return 0;
    }
}

int DG_GetKey(int *pressed, unsigned char *key) {
    if (!next_usage) {
        check(fnd_con_key_state(&keyboard));
        /* The physical state is the input source; drain text type-ahead so
         * keys pressed during play cannot later become shell commands. */
        fnd_key_t text;
        for (unsigned i = 0; i < 256; ++i) {
            fnd_status st = fnd_con_pollkey(&text);
            if (st == FND_END) break;
            check(st);
        }
    }
    while (next_usage < 256) {
        unsigned u = next_usage++;
        unsigned bit = 1u << (u % 32);
        uint32_t down_bits = keyboard.down[u / 32];
        uint32_t press_bits = keyboard.pressed[u / 32];
        /* Left/right modifiers share one Doom key. Releasing one side must
         * not release that key while the other side remains held. */
        if (u >= 224 && u <= 226) {
            down_bits |= down_bits >> 4;
            press_bits |= press_bits >> 4;
        }
        int down = !!(down_bits & bit);
        /* Preserve short taps: emit press followed by release on the next
         * scan even if both edges arrived before this query. */
        int state = held[u] ? down : down || !!(press_bits & bit);
        unsigned char mapped = key_for(u);
        if (mapped && state != held[u]) {
            held[u] = (uint8_t)state; *pressed = state; *key = mapped;
            return 1;
        }
    }
    next_usage = 0;
    return 0;
}

void DG_SetWindowTitle(const char *title) { (void)title; }

int main(int argc, char **argv) {
    for (int i = 1; i < argc; ++i) {
        if (!strcmp(argv[i], "-fnd-frames")) {
            if (++i >= argc) return 2;
            char *end;
            unsigned long n = strtoul(argv[i], &end, 10);
            if (!n || *end || n > 1000000u) return 2;
            frame_limit = (uint32_t)n;
        }
    }
    doomgeneric_Create(argc, argv);
    for (;;) doomgeneric_Tick();
}
