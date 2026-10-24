/* libsgc through the WAL "sgc" in Foundation: /dev/fb0 set up, drawing queued
 * and fed to /dev/sgc, with a deliberately tiny buffer so the FIFO-full and
 * buffer-full paths run. Argument "k" keeps the stream open afterwards so the
 * host can read the screen. */
#include <foundation/libsgc.h>
#include <foundation/sys.h>
#define CHECK(x) do { if (!(x)) fnd_exit(__LINE__); } while (0)
/* black, white, red, green, blue */
static const uint8_t palette[15] = {0,0,0, 255,255,255, 255,0,0, 0,255,0, 0,0,255};

int main(void) {
    fnd_init();
    const char *arg = fnd_argc() > 1 ? fnd_argv(1) : "";
    cpt_sgc g;
    uint32_t buf[CPT_SGC_MAX_COMMAND], tiny[CPT_SGC_MAX_COMMAND - 1];
    fnd_sgc_status st;
    CHECK(cpt_sgc_open(&g, tiny, CPT_SGC_MAX_COMMAND - 1) == FND_E_BAD_ARG);
    CHECK(cpt_sgc_open(&g, buf, CPT_SGC_MAX_COMMAND) == FND_OK);
    CHECK(cpt_sgc_display_init(&g, 0, 5, palette) == FND_OK);
    CHECK(cpt_sgc_rect(&g, 0, 0, 1, 1, 64) == FND_E_BAD_ARG);

    CHECK(cpt_sgc_clear(&g, 0) == FND_OK);
    CHECK(cpt_sgc_rect(&g, 120, 120, 10, 6, 1) == FND_OK);
    CHECK(cpt_sgc_rect(&g, 200, 100, 50, 40, 2) == FND_OK);
    CHECK(cpt_sgc_triangle(&g, 50, 100, 110, 100, 50, 160, 3) == FND_OK);
    CHECK(cpt_sgc_line(&g, 10, 200, 60, 200, 4) == FND_OK);
    CHECK(cpt_sgc_glyph(&g, 0x41, 150, 200, 1) == FND_OK);
    /* Drawn-nothing shapes must not reach the device. */
    CHECK(cpt_sgc_rect(&g, 5, 5, 0, 3, 1) == FND_OK);
    CHECK(cpt_sgc_triangle(&g, 0, 0, 5, 5, 10, 10, 1) == FND_OK);
    /* 100 alternating-color stripes: 600 words through an 8-word buffer and a
     * 256-word FIFO, so the loop has to wait for frames. */
    for (uint32_t i = 0; i < 100; ++i) {
        int32_t r;
        uint32_t tries = 0;
        while ((r = cpt_sgc_rect(&g, (int32_t)(i * 3), 220, 2, 2, 1 + (i & 1))) == FND_E_WOULD_BLOCK) {
            CHECK(++tries < 1000);
            fnd_sys_sleep(1);
            CHECK(cpt_sgc_flush(&g) == FND_OK);
        }
        CHECK(r == FND_OK);
    }
    CHECK(cpt_sgc_flush_wait(&g) == FND_OK);
    CHECK(cpt_sgc_pending(&g) == 0);
    fnd_sys_sleep(5); /* the VDP frame that consumes the last FIFO words */
    CHECK(cpt_sgc_status(&g, 0, &st) == FND_OK);
    CHECK(st.fifo_used == 0 && !(st.status & (FND_SGC_STATUS_FIFO_ERROR | FND_SGC_STATUS_FIFO_FULL)));
    if (arg[0] == 'k') {
        for (;;) fnd_sys_sleep(10);
    }
    CHECK(cpt_sgc_close(&g) == FND_OK);
    fnd_con_write("libsgc passed\n");
    return 0;
}
