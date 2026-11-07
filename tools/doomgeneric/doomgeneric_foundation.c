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
extern unsigned wad_reads, wad_bytes, wad_ms;
/* Engine view settings: -fnd-light trades image size and detail for speed. */
extern int detailLevel, screenblocks;
void R_SetViewSize(int blocks, int detail);
static int light;
static uint32_t last_start;
static uint32_t tick_hz, start_tick, frame_limit, frames, stats_every, draw_ms, stats_start;
static unsigned next_usage;
static fnd_keyboard keyboard;
static uint8_t held[256];
static uint8_t rgb[320 * 200 * 3];
/* Tang's bitmap VDP is INDEX8 only: the engine's palette indices go to fb0 as
 * they are and the 256-entry CLUT carries the colours. */
static int indexed;
static uint8_t clut[768], clut_valid;
/* Serial terminals have no key release: UART keys are held for a short time
 * and renewed by the terminal's auto-repeat. */
#define UART_HOLD_MS 200u
#define UART_KEYS 64
static struct { unsigned char key; uint8_t down, pending; uint32_t until; } uart_keys[UART_KEYS];

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
    check(fnd_con_key_state(&keyboard));
    if (light) {
        detailLevel = 1; /* config files are compiled out of this port */
        screenblocks = 8;
        R_SetViewSize(screenblocks, detailLevel);
    }
}

/* fb0 is opened for the first frame, not in DG_Init: while it is open the
 * bitmap replaces the console, and init-time console output would land in
 * the bitmap's first rows. Opening late keeps the text console readable. */
static void open_fb(void) {
    check(fnd_file_open("/dev/fb0", FND_OPEN_WRITE, FND_MODE_DEFAULT, &fb));
    fnd_fb_info info;
    check(fnd_fb_get_info(fb, &info));
    if (info.width != 320 || info.height != 240) check(FND_E_UNSUPPORTED);
    fnd_status format = fnd_fb_set_format(fb, FND_FB_RGB888);
    if (format == FND_E_NO_MEDIA) indexed = 1;
    else check(format);
    /* RGB888 leaves the shared SGC/GP CLUT intact. Black bars at y=0..19,
     * 220..239; the engine's native 320x200 image is centred between them.
     * In INDEX8 the bars are index 0, which is black in every Doom palette. */
    uint32_t pixel = indexed ? 1u : 3u;
    memset(rgb, 0, 320 * 20 * pixel); /* black bars */
    check(fnd_fb_seek(fb, 0));
    write_all(rgb, 320 * 20 * pixel);
    check(fnd_fb_seek(fb, 320 * 220 * pixel));
    write_all(rgb, 320 * 20 * pixel);
}

static void upload_palette(void) {
    uint8_t now[768];
    for (uint32_t i = 0; i < 256; ++i) {
        now[i * 3] = colors[i].r; now[i * 3 + 1] = colors[i].g; now[i * 3 + 2] = colors[i].b;
    }
    if (clut_valid && !memcmp(now, clut, sizeof now)) return;
    for (uint32_t first = 0; first < 256; first += 64)
        check(fnd_fb_clut_write(fb, first, 64, now + first * 3));
    memcpy(clut, now, sizeof now);
    clut_valid = 1;
}

uint32_t DG_GetTicksMs(void);

void DG_DrawFrame(void) {
    uint32_t draw_start = DG_GetTicksMs();
    if (!frames) stats_start = draw_start;
    if (!fb) open_fb();
    if (indexed) {
        upload_palette();
        check(fnd_fb_seek(fb, 320 * 20));
        write_all(DG_ScreenBuffer, 320 * 200);
    } else {
        for (uint32_t i = 0; i < 320 * 200; ++i) {
            struct color c = colors[DG_ScreenBuffer[i]];
            rgb[i * 3] = c.r; rgb[i * 3 + 1] = c.g; rgb[i * 3 + 2] = c.b;
        }
        check(fnd_fb_seek(fb, 320 * 20 * 3));
        write_all(rgb, sizeof rgb);
    }
    draw_ms += DG_GetTicksMs() - draw_start;
    ++frames;
    if (frame_limit > 10 && frames == frame_limit - 10) last_start = DG_GetTicksMs();
    if (stats_every && frames % stats_every == 0) {
        printf("doom-stats: frames=%u elapsed=%ums draw=%ums" "\n", (unsigned)frames,
               (unsigned)(DG_GetTicksMs() - stats_start), (unsigned)draw_ms);
        fflush(stdout);
    }
    if (frame_limit && frames >= frame_limit) {
        cleanup();
        FILE *report = fopen("/doomstats.txt", "w");
        if (report) {
            fprintf(report, "frames=%u elapsed=%ums draw=%ums init=%ums last10=%ums" "\n", (unsigned)frames,
                    (unsigned)(DG_GetTicksMs() - stats_start), (unsigned)draw_ms, (unsigned)stats_start,
                    (unsigned)(DG_GetTicksMs() - last_start));
            fprintf(report, "wad reads=%u bytes=%u ms=%u" "\n", wad_reads, wad_bytes, wad_ms);
            fclose(report);
        }
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

/* Terminal keys: arrows/Enter/Esc/Tab/Backspace, ' ' = use, 'f' = fire,
 * 'z' = run, 'x' = strafe modifier; other letters, digits and ",.-=" pass through. */
static unsigned char uart_map(const fnd_key_t *k) {
    switch (k->code) {
    case FND_KEY_ENTER: return KEY_ENTER;
    case FND_KEY_BACKSPACE: return KEY_BACKSPACE;
    case FND_KEY_LEFT: return KEY_LEFTARROW;
    case FND_KEY_RIGHT: return KEY_RIGHTARROW;
    case FND_KEY_UP: return KEY_UPARROW;
    case FND_KEY_DOWN: return KEY_DOWNARROW;
    case FND_KEY_TAB: return KEY_TAB;
    case FND_KEY_ESC: return KEY_ESCAPE;
    case FND_KEY_CHAR: break;
    default: return 0;
    }
    uint32_t c = k->ch;
    if (c >= 'A' && c <= 'Z') c += 'a' - 'A';
    if (c == ' ') return KEY_USE;
    if (c == 'f') return KEY_FIRE;
    if (c == 'z') return KEY_RSHIFT;
    if (c == 'x') return KEY_RALT;
    if ((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == ',' || c == '.' || c == '-' || c == '=')
        return (unsigned char)c;
    return 0;
}

static void uart_feed(const fnd_key_t *k) {
    unsigned char mapped = uart_map(k);
    if (!mapped) return;
    uint32_t until = DG_GetTicksMs() + UART_HOLD_MS;
    int free_slot = -1;
    for (int i = 0; i < UART_KEYS; ++i) {
        if (uart_keys[i].key == mapped) { uart_keys[i].until = until; uart_keys[i].down = 1; return; }
        if (!uart_keys[i].key && free_slot < 0) free_slot = i;
    }
    if (free_slot >= 0) {
        uart_keys[free_slot].key = mapped; uart_keys[free_slot].down = 1;
        uart_keys[free_slot].pending = 1; uart_keys[free_slot].until = until;
    }
}

static int uart_event(int *pressed, unsigned char *key) {
    uint32_t now = DG_GetTicksMs();
    for (int i = 0; i < UART_KEYS; ++i) {
        if (!uart_keys[i].key) continue;
        if (uart_keys[i].pending) {
            uart_keys[i].pending = 0; *pressed = 1; *key = uart_keys[i].key;
            return 1;
        }
        if ((int32_t)(now - uart_keys[i].until) >= 0) {
            *pressed = 0; *key = uart_keys[i].key; uart_keys[i].key = 0; uart_keys[i].down = 0;
            return 1;
        }
    }
    return 0;
}

int DG_GetKey(int *pressed, unsigned char *key) {
    if (!next_usage) {
        check(fnd_con_key_state(&keyboard));
        /* Drain text type-ahead so keys pressed during play cannot later
         * become shell commands; serial keys feed the UART key path. */
        fnd_key_t text;
        for (unsigned i = 0; i < 256; ++i) {
            fnd_status st = fnd_con_pollkey(&text);
            if (st == FND_END) break;
            check(st);
            uart_feed(&text);
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
    return uart_event(pressed, key);
}

void DG_SetWindowTitle(const char *title) { (void)title; }

int main(int argc, char **argv) {
    for (int i = 1; i < argc; ++i) {
        if (!strcmp(argv[i], "-fnd-stats") && i + 1 < argc) {
            stats_every = (uint32_t)strtoul(argv[++i], 0, 10);
            continue;
        }
        if (!strcmp(argv[i], "-fnd-light")) {
            light = 1;
            continue;
        }
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
