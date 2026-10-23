#include <foundation/sgc.h>
#include <foundation/sys.h>
#include <foundation/fb.h>
#define CHECK(x) do { if (!(x)) fnd_exit(__LINE__); } while (0)
static const uint8_t white[3] = {255, 255, 255};
int main(void) {
    fnd_init();
    fnd_handle fb, h, second;
    uint32_t done;
    fnd_sgc_status st;
    const char *arg = fnd_argc() > 1 ? fnd_argv(1) : "";
    CHECK(fnd_file_open("/dev/fb0", FND_OPEN_WRITE, FND_MODE_DEFAULT, &fb) == FND_OK);
    CHECK(fnd_fb_clut_write(fb, 1, 1, white) == FND_OK);
    CHECK(fnd_file_open("/dev/sgc", FND_OPEN_READ, FND_MODE_DEFAULT, &second) == FND_OK);
    CHECK(fnd_sgc_get_status(second, 0, &st) == FND_OK && st.id == 0x53474331u);
    CHECK(fnd_sgc_get_status(second, 1, &st) == FND_E_ACCESS);
    CHECK(fnd_sgc_reset(second) == FND_E_ACCESS);
    CHECK(fnd_close(second) == FND_OK);
    CHECK(fnd_file_open("/dev/sgc", 0, FND_MODE_DEFAULT, &second) < 0);
    CHECK(fnd_file_open("/dev/sgc", FND_OPEN_READ | FND_OPEN_WRITE, FND_MODE_DEFAULT, &h) == FND_OK);
    CHECK(fnd_file_open("/dev/sgc", FND_OPEN_READ, FND_MODE_DEFAULT, &second) == FND_E_BUSY);
    CHECK(fnd_sgc_get_status(h, 0, &st) == FND_OK && (st.control & 1) && st.output_gp == 1 &&
          st.fifo_used == 0 && st.fifo_free == FND_SGC_FIFO_WORDS);
    CHECK(fnd_sgc_set_output(h, 8) == FND_E_BAD_ARG);
    /* A partial word is rejected; nothing reaches the FIFO. */
    uint8_t odd[3] = {0, 0, 0};
    CHECK(fnd_call(h, FND_ST_WRITE, 0, odd, 3, 0, 0, 0) == FND_E_BAD_ARG);
    /* Fill the FIFO exactly: NOP words, then a full FIFO must not block. */
    uint32_t nops[FND_SGC_FIFO_WORDS + 8];
    for (uint32_t i = 0; i < FND_SGC_FIFO_WORDS + 8; ++i) nops[i] = FND_SGC_OP_NOP;
    CHECK(fnd_sgc_submit(h, nops, FND_SGC_FIFO_WORDS + 8, &done) == FND_OK && done == FND_SGC_FIFO_WORDS);
    CHECK(fnd_sgc_submit(h, nops, 1, &done) == FND_E_WOULD_BLOCK && done == 0);
    CHECK(fnd_sgc_get_status(h, 0, &st) == FND_OK && st.fifo_free == 0 && !(st.status & FND_SGC_STATUS_FIFO_FULL));
    CHECK(fnd_sgc_fifo_clear(h) == FND_OK);
    CHECK(fnd_sgc_get_status(h, FND_SGC_STATUS_FIFO_ERROR, &st) == FND_OK && (st.status & FND_SGC_STATUS_FIFO_ERROR));
    CHECK(fnd_sgc_get_status(h, 0, &st) == FND_OK && st.fifo_used == 0 && !(st.status & FND_SGC_STATUS_FIFO_ERROR));
    CHECK(fnd_sgc_get_status(h, 0x100, &st) == FND_E_BAD_ARG);
    /* Black backdrop, then a white 10x6 rectangle at (20,30) (color 1 = white CLUT entry). */
    uint32_t cmds[] = {FND_SGC_OP_SET_COLOR, FND_SGC_OP_SCREEN_FILL, FND_SGC_OP_SET_COLOR | 1u,
                       FND_SGC_OP_RECT, 20, 30, 30, 36};
    CHECK(fnd_sgc_submit(h, cmds, 8, &done) == FND_OK && done == 8);
    if (arg[0] == 'l') { /* leak: exit while open and with commands pending */
        for (;;) fnd_sys_sleep(10);
    }
    if (arg[0] == 'x') return 0; /* exit with the stream open */
    fnd_sys_sleep(5); /* let the next VDP frame consume the FIFO */
    CHECK(fnd_sgc_get_status(h, 0, &st) == FND_OK && st.fifo_used == 0 && !(st.status & 0x16));
    CHECK(fnd_close(h) == FND_OK);
    CHECK(fnd_close(fb) == FND_OK);
    fnd_con_write("sgc passed\n");
    return 0;
}
