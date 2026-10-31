/* Execute the real adapter with a fake Foundation device boundary. */
#include <foundation/fnd.h>
#include <foundation/abi.h>
#include <foundation/keyboard.h>
#define FOUNDATION_SYS_H
static uint32_t clock_tick, slept;
static int32_t mock_time(uint32_t selector) { return selector == FND_TIME_TICK_HZ ? 100 : clock_tick; }
#define fnd_sys_time_get mock_time
#define fnd_sys_sleep(t) (slept = (t), 0)
#define fnd_sys_yield() 0
#define fnd_sys_debug_write(p, n) 0
#define main doom_main
#include "../../../tools/doomgeneric/doomgeneric_foundation.c"
#undef main
#include "../../../tools/doomgeneric/w_file_foundation.c"

pixel_t screen[320 * 200];
pixel_t *DG_ScreenBuffer = screen;
struct color colors[256];
void doomgeneric_Create(int argc, char **argv) { (void)argc; (void)argv; }
void doomgeneric_Tick(void) {}
static fnd_keyboard physical;
static uint8_t display[320 * 240 * 3];
static uint32_t position, closed, clut_writes;
fnd_handle fnd_console(void) { return 1; }
const char *fnd_strerror(fnd_status st) { (void)st; return "mock error"; }
fnd_status fnd_file_open(const char *p, uint32_t f, uint32_t m, fnd_handle *h) {
    (void)p; (void)f; (void)m; *h = 2; return FND_OK;
}
fnd_status fnd_close(fnd_handle h) { (void)h; ++closed; return FND_OK; }
static fnd_key_t queued[8];
static unsigned queued_count, queued_next;
static uint8_t clut[768];
static fnd_status format_status;
fnd_status fnd_con_pollkey(fnd_key_t *key) {
    if (queued_next >= queued_count) return FND_END;
    *key = queued[queued_next++]; return FND_OK;
}
fnd_status fnd_write(fnd_handle h, const void *p, uint32_t n, uint32_t *put) {
    (void)h;
    /* Force short writes, including across scanline boundaries. */
    if (n > 997) n = 997;
    if (position + n > sizeof display) return FND_E_NO_SPACE;
    memcpy(display + position, p, n); position += n; *put = n; return FND_OK;
}
fnd_status fnd_call(fnd_handle h, uint32_t op, const uint32_t a[4], const void *data,
                    uint32_t n, void *out, uint32_t cap, fnd_msg *reply) {
    (void)h; (void)data; (void)n; (void)cap;
    if (op == FND_CON_KEY_STATE) {
        memcpy(out, &physical, sizeof physical); reply->len = sizeof physical;
        memset(physical.pressed, 0, sizeof physical.pressed);
        memset(physical.released, 0, sizeof physical.released);
    } else if (op == FND_FB_GET_INFO) {
        fnd_fb_info info = {320, 240, 320, 320 * 240, 0, 256};
        memcpy(out, &info, sizeof info); reply->len = sizeof info;
    } else if (op == FND_ST_SEEK) position = a[2];
    else if (op == FND_FB_CLUT_WRITE) { ++clut_writes; memcpy(clut + a[0] * 3, data, n); }
    else if (op == FND_FB_SET_FORMAT) return format_status;
    return FND_OK;
}
#define CHECK(x) do { if (!(x)) return __LINE__; } while (0)
static void scan(void) { int p; unsigned char k; while (DG_GetKey(&p, &k)) {} }
static void bit(uint32_t *bits, unsigned u) { bits[u / 32] |= 1u << (u % 32); }
int test_platform(void) {
    memset(display, 99, sizeof display);
    clock_tick = 0xfffffff0u;
    DG_Init();
    clock_tick = 4;
    CHECK(DG_GetTicksMs() == 200);
    DG_SleepMs(1); CHECK(slept == 1);
    DG_SleepMs(21); CHECK(slept == 3);
    colors[7].r = 17; colors[7].g = 83; colors[7].b = 241;
    memset(screen, 7, sizeof screen);
    DG_DrawFrame();
    CHECK(!clut_writes);
    for (unsigned i = 0; i < 320 * 240; ++i) {
        int image = i >= 320 * 20 && i < 320 * 220;
        CHECK(display[i * 3] == (image ? 17 : 0));
        CHECK(display[i * 3 + 1] == (image ? 83 : 0));
        CHECK(display[i * 3 + 2] == (image ? 241 : 0));
    }
    int p; unsigned char k;
    bit(physical.pressed, 82); /* tap entirely between polls */
    CHECK(DG_GetKey(&p, &k) && p && k == KEY_UPARROW);
    scan();
    CHECK(DG_GetKey(&p, &k) && !p && k == KEY_UPARROW); scan();
    bit(physical.down, 224); bit(physical.down, 228);
    CHECK(DG_GetKey(&p, &k) && p && k == KEY_FIRE); scan();
    physical.down[7] &= ~1u;
    CHECK(!DG_GetKey(&p, &k)); /* right Ctrl remains held */
    physical.down[7] = 0;
    CHECK(DG_GetKey(&p, &k) && !p && k == KEY_FIRE); scan();
    bit(physical.down, 44);
    CHECK(DG_GetKey(&p, &k) && p && k == KEY_USE); scan();
    physical.down[1] = 0; scan(); scan();
    cleanup(); CHECK(closed == 1 && !fb);
    cleanup(); CHECK(closed == 1);
    return 0;
}

/* Tang: INDEX8 only. Indices go to fb0 untouched, colours through the CLUT,
 * and serial keys are held for a while because a terminal has no key release. */
int test_tang(void) {
    memset(display, 99, sizeof display);
    position = clut_writes = 0;
    fb = 0; indexed = 0; clut_valid = 0; frames = 0;
    format_status = FND_E_NO_MEDIA; /* RGB888 refused */
    clock_tick = 0;
    DG_Init();
    CHECK(!fb); /* fb0 stays closed until the first frame */
    colors[7].r = 17; colors[7].g = 83; colors[7].b = 241;
    colors[9].r = 1; colors[9].g = 2; colors[9].b = 3;
    memset(screen, 7, sizeof screen);
    screen[0] = 9;
    DG_DrawFrame();
    CHECK(fb && indexed);
    CHECK(clut_writes == 4 && clut[21] == 17 && clut[22] == 83 && clut[23] == 241 && clut[27] == 1);
    for (unsigned i = 0; i < 320 * 240; ++i) {
        int image = i >= 320 * 20 && i < 320 * 220;
        CHECK(display[i] == (image ? (i == 320 * 20 ? 9 : 7) : 0));
    }
    DG_DrawFrame(); /* unchanged palette: no CLUT traffic */
    CHECK(clut_writes == 4);
    colors[7].r = 18;
    DG_DrawFrame();
    CHECK(clut_writes == 8 && clut[21] == 18);
    int p; unsigned char k;
    scan();
    queued[0] = (fnd_key_t){FND_KEY_UP, 0, 0};
    queued[1] = (fnd_key_t){FND_KEY_CHAR, 'F', 0};
    queued[2] = (fnd_key_t){FND_KEY_CHAR, ' ', 0};
    queued_count = 3; queued_next = 0;
    CHECK(DG_GetKey(&p, &k) && p && k == KEY_UPARROW);
    CHECK(DG_GetKey(&p, &k) && p && k == KEY_FIRE);
    CHECK(DG_GetKey(&p, &k) && p && k == KEY_USE);
    CHECK(!DG_GetKey(&p, &k)); /* all held */
    clock_tick += 10; /* 100 ms: still held; the terminal repeats UP */
    queued[0] = (fnd_key_t){FND_KEY_UP, 0, 0};
    queued_count = 1; queued_next = 0;
    CHECK(!DG_GetKey(&p, &k));
    clock_tick += 15; /* 250 ms after the repeat of UP: FIRE and USE expire */
    int released = 0;
    while (DG_GetKey(&p, &k)) { CHECK(!p && (k == KEY_FIRE || k == KEY_USE)); ++released; }
    CHECK(released == 2);
    clock_tick += 30;
    CHECK(DG_GetKey(&p, &k) && !p && k == KEY_UPARROW);
    CHECK(!DG_GetKey(&p, &k));
    cleanup();
    format_status = FND_OK;
    return 0;
}

int test_wad(const char *path) {
    wad_file_t *wad = stdc_wad_file.OpenFile((char *)path);
    CHECK(wad && wad->mapped && wad->length == 256);
    for (unsigned i = 0; i < 256; ++i) CHECK(wad->mapped[i] == i);
    uint8_t data[16];
    CHECK(stdc_wad_file.Read(wad, 250, data, sizeof data) == 6);
    CHECK(data[0] == 250 && data[5] == 255);
    CHECK(stdc_wad_file.Read(wad, 256, data, sizeof data) == 0);
    CHECK(stdc_wad_file.Read(wad, 0xffffffffu, data, sizeof data) == 0);
    stdc_wad_file.CloseFile(wad);
    CHECK(!stdc_wad_file.OpenFile("missing-foundation-doom-test-file.wad"));
    /* Streamed mode (WADs above wad_ram_limit): same reads without a RAM copy. */
    extern unsigned wad_ram_limit;
    unsigned saved = wad_ram_limit;
    wad_ram_limit = 100;
    wad = stdc_wad_file.OpenFile((char *)path);
    CHECK(wad && !wad->mapped && wad->length == 256);
    CHECK(stdc_wad_file.Read(wad, 10, data, sizeof data) == 16 && data[0] == 10 && data[15] == 25);
    CHECK(stdc_wad_file.Read(wad, 250, data, sizeof data) == 6 && data[0] == 250 && data[5] == 255);
    CHECK(stdc_wad_file.Read(wad, 3, data, sizeof data) == 16 && data[0] == 3); /* seek backwards */
    CHECK(stdc_wad_file.Read(wad, 256, data, sizeof data) == 0);
    CHECK(stdc_wad_file.Read(wad, 0xffffffffu, data, sizeof data) == 0);
    stdc_wad_file.CloseFile(wad);
    wad_ram_limit = saved;
    return 0;
}
