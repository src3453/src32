#include <foundation/sgu.h>
#include <foundation/keyboard.h>
#include <foundation/sys.h>
#define CHECK(x) do { if (!(x)) fnd_exit(__LINE__); } while (0)
static void seek(fnd_handle h, uint32_t off) { CHECK(fnd_sgu_seek(h, off) == FND_OK); }
int main(void) {
    fnd_init();
    fnd_handle h, other;
    uint32_t n;
    uint8_t value = 77, got;
    const char *arg = fnd_argc() > 1 ? fnd_argv(1) : "";
    CHECK(fnd_file_open("/dev/sgu", FND_OPEN_READ | FND_OPEN_WRITE, FND_MODE_DEFAULT, &h) == FND_OK);
    CHECK(fnd_file_open("/dev/sgu", FND_OPEN_READ, FND_MODE_DEFAULT, &other) == FND_E_BUSY);
    fnd_stat_t st;
    CHECK(fnd_stream_stat(h, &st) == FND_OK && st.size_lo == FND_SGU_SIZE);
    for (uint32_t core = 0; core < 2; ++core)
        for (uint32_t ch = 0; ch < 8; ++ch) {
            uint32_t base = core * 0x1000u;
            seek(h, base + ch * 256 + 255);
            value = (uint8_t)(core * 8 + ch + 32);
            CHECK(fnd_write(h, &value, 1, &n) == FND_OK && n == 1);
            seek(h, base + ch * 256 + 255);
            CHECK(fnd_read(h, &got, 1, &n) == FND_OK && n == 1 && got == value);
            seek(h, base + 0x800 + ch * 32);
            uint8_t regs[10] = {1,0xB8,0,80,0xFF,0,0,0,0,0}, back[10];
            CHECK(fnd_write(h, regs, sizeof regs, &n) == FND_OK && n == sizeof regs);
            seek(h, base + 0x800 + ch * 32);
            CHECK(fnd_read(h, back, sizeof back, &n) == FND_OK && n == sizeof back);
            for (uint32_t i = 0; i < sizeof back; ++i) CHECK(back[i] == regs[i]);
            seek(h, base + 0x819 + ch * 32);
            uint8_t playing = 1;
            CHECK(fnd_write(h, &playing, 1, &n) == FND_OK);
        }
    const uint32_t offsets[] = {FND_SGU_PCM, FND_SGU_PCM + 0x80000, FND_SGU_SIZE - 1};
    for (uint32_t i = 0; i < 3; ++i) {
        value = (uint8_t)(200 + i); seek(h, offsets[i]);
        CHECK(fnd_write(h, &value, 1, &n) == FND_OK && n == 1);
        seek(h, offsets[i]); CHECK(fnd_read(h, &got, 1, &n) == FND_OK && got == value && n == 1);
    }
    CHECK(fnd_sgu_seek(h, FND_SGU_SIZE + 1) == FND_E_BAD_ARG);
    seek(h, FND_SGU_SIZE - 1);
    uint8_t overflow[2] = {99,99};
    CHECK(fnd_write(h, overflow, 2, &n) == FND_E_NO_SPACE && n == 0);
    CHECK(fnd_read(h, &got, 1, &n) == FND_OK && got == 202);
    seek(h, FND_SGU_SIZE);
    CHECK(fnd_read(h, &got, 1, &n) == FND_OK && n == 0);
    CHECK(fnd_write(h, &value, 1, &n) == FND_E_NO_SPACE && n == 0);
    seek(h, 0x803); value = 128;
    CHECK(fnd_write(h, &value, 1, &n) == FND_OK);
    if (arg[0] == 'l') fnd_exit(0); /* leaked stream must silence on exit */
    if (arg[0] == 'f') { *(volatile uint8_t *)(uintptr_t)0x80020803u = 255; fnd_exit(99); }
    int32_t dup = fnd_sys_handle_dup(h);
    CHECK(dup > 0 && fnd_close(h) == FND_OK);
    seek((uint32_t)dup, 0x803);
    CHECK(fnd_read((uint32_t)dup, &got, 1, &n) == FND_OK && got == 128);
    CHECK(fnd_close((uint32_t)dup) == FND_OK);
    CHECK(fnd_file_open("/dev/sgu", FND_OPEN_READ, FND_MODE_DEFAULT, &h) == FND_OK);
    for (uint32_t core = 0; core < 2; ++core)
        for (uint32_t ch = 0; ch < 8; ++ch) {
            seek(h, core * 0x1000 + 0x803 + ch * 32);
            CHECK(fnd_read(h, &got, 1, &n) == FND_OK && got == 0);
            seek(h, core * 0x1000 + 0x819 + ch * 32);
            CHECK(fnd_read(h, &got, 1, &n) == FND_OK && got == 0);
        }
    CHECK(fnd_write(h, &value, 1, &n) == FND_E_ACCESS);
    fnd_close(h);
    if (arg[0] == 'k') {
        fnd_keyboard keys;
        CHECK(fnd_con_key_state(&keys) == FND_OK);
        uint32_t start = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO);
        int pressed = 0, released = 0, simultaneous = 0;
        while ((uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO) - start < 300) {
            CHECK(fnd_con_key_state(&keys) == FND_OK);
            /* HID 30 (1), 31 (2) held together, then released separately. */
            if ((keys.down[0] & 0xC0000000u) == 0xC0000000u) simultaneous = 1;
            if (keys.pressed[0] & 0x40000000u) pressed = 1;
            if (keys.released[0] & 0x40000000u) released = 1;
            if (pressed && released && simultaneous) break;
            fnd_sys_sleep(1);
        }
        CHECK(pressed && released && simultaneous);
        fnd_key_t key;
        while (fnd_con_pollkey(&key) == FND_OK) {}
    }
    fnd_con_write("chip8 devices passed\n");
    fnd_exit(0);
}
