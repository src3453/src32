/* An ordinary Foundation app; needs no MMIO capability or grant. */
#include <foundation/fb.h>
#include <foundation/sys.h>

int main(void) {
    fnd_init();
    fnd_handle fb;
    fnd_status st = fnd_file_open("/dev/fb0", FND_OPEN_WRITE, FND_MODE_DEFAULT, &fb);
    if (st < 0) return st;
    uint8_t rgb[768], row[320];
    for (uint32_t i = 0; i < 256; ++i) {
        rgb[i * 3] = (uint8_t)i;
        rgb[i * 3 + 1] = (uint8_t)(255 - i);
        rgb[i * 3 + 2] = (uint8_t)(i ^ 128);
    }
    st = fnd_fb_clut_write(fb, 0, 256, rgb);
    for (uint32_t y = 0; y < 240 && st == FND_OK; ++y) {
        for (uint32_t x = 0; x < 320; ++x) row[x] = (uint8_t)((x + y) & 255);
        uint32_t put;
        st = fnd_write(fb, row, sizeof row, &put);
    }
    if (st == FND_OK) fnd_sys_sleep((uint32_t)fnd_sys_time_get(FND_TIME_TICK_HZ));
    fnd_close(fb);
    return st;
}
