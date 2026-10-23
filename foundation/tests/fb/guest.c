/* Runs as an ordinary WAX app: fs transfers a kernel-backed CHANNEL. */
#include <foundation/fb.h>
#include <foundation/sys.h>

#define CHECK(x) do { if (!(x)) fnd_exit(__LINE__); } while (0)
static uint8_t data[5000], got[5000], palette[768], saved_palette[768];

int main(void) {
    fnd_init();
    fnd_handle h, second;
    fnd_fb_info info;
    uint32_t n;
    const char *arg = fnd_argc() > 1 ? fnd_argv(1) : "";
    CHECK(fnd_file_open("/dev/fb0", FND_OPEN_READ | FND_OPEN_WRITE, FND_MODE_DEFAULT, &h) == FND_OK);
    if (arg[0] == 'l' || arg[0] == 'f') {
        uint8_t bad = 0xFF;
        CHECK(fnd_write(h, &bad, 1, &n) == FND_OK);
        CHECK(fnd_fb_clut_write(h, 0, 1, &data[0]) == FND_OK);
        if (arg[0] == 'f') *(volatile uint8_t *)0x10000000u = 0; /* U must fault */
        return 0; /* intentionally leave h open: proc_exit must restore it */
    }
    CHECK(fnd_file_open("/dev/fb0", FND_OPEN_READ, FND_MODE_DEFAULT, &second) == FND_E_BUSY);
    CHECK(fnd_fb_get_info(h, &info) == FND_OK && info.width == 320 && info.height == 240 &&
          info.stride == 320 && info.size == 76800 && info.format == FND_FB_INDEX8 && info.clut_entries == 256);
    CHECK(fnd_fb_clut_read(h, 0, 256, saved_palette) == FND_OK);
    for (uint32_t i = 0; i < sizeof palette; ++i) palette[i] = (uint8_t)(i * 13u);
    CHECK(fnd_fb_clut_write(h, 0, 256, palette) == FND_OK);
    CHECK(fnd_fb_clut_read(h, 0, 256, got) == FND_OK);
    for (uint32_t i = 0; i < sizeof palette; ++i) CHECK(got[i] == palette[i]);
    CHECK(fnd_fb_clut_write(h, 255, 2, palette) == FND_E_BAD_ARG);
    uint32_t args[4] = {0xFFFFFFFFu, 1, 0, 0};
    CHECK(fnd_call(h, FND_FB_CLUT_WRITE, args, palette, 3, 0, 0, 0) == FND_E_BAD_ARG);
    args[0] = 255;
    CHECK(fnd_call(h, FND_FB_CLUT_WRITE, args, palette, 2, 0, 0, 0) == FND_E_BAD_ARG);
    CHECK(fnd_call(h, FND_FB_CLUT_READ, args, 0, 0, got, 2, 0) == FND_E_LIMIT);
    CHECK(fnd_fb_set_format(h, 3) == FND_E_BAD_ARG);
    CHECK(fnd_call(h, 0xDEADu, 0, 0, 0, 0, 0, 0) == FND_E_UNSUPPORTED);

    /* Prove validation happens before hardware side effects, in VM and slots. */
    fnd_msg req = {FND_ST_WRITE, 0, 0, 0, 0, 0, 0x10000000u, 1}, rep = {0};
    CHECK(fnd_sys_ipc_call(h, &req, &rep) == FND_E_ACCESS);
    req = (fnd_msg){FND_FB_CLUT_READ, 0, 0, 1, 0, 0, 0, 0};
    rep.buf = 0x80000000u;
    rep.len = 3;
    CHECK(fnd_sys_ipc_call(h, &req, &rep) == FND_E_ACCESS);

    for (uint32_t format = 0; format < 3; ++format) {
        fnd_status st = fnd_fb_set_format(h, format);
        CHECK(st == FND_OK);
        CHECK(fnd_fb_get_info(h, &info) == FND_OK && info.size == 76800u * (format + 1) &&
              info.stride == 320u * (format + 1));
        for (uint32_t i = 0; i < sizeof data; ++i) data[i] = (uint8_t)(i * 7 + format + 17);
        CHECK(fnd_fb_seek(h, 0x6000) == FND_OK);
        CHECK(fnd_write(h, data, sizeof data, &n) == FND_OK && n == sizeof data);
        CHECK(fnd_fb_seek(h, 0x6000) == FND_OK);
        CHECK(fnd_read(h, got, sizeof got, &n) == FND_OK && n == sizeof got);
        for (uint32_t i = 0; i < sizeof data; ++i) CHECK(got[i] == data[i]);
        CHECK(fnd_fb_seek(h, info.size - 1) == FND_OK);
        CHECK(fnd_write(h, data, 2, &n) == FND_E_NO_SPACE && n == 0);
        CHECK(fnd_write(h, data, 1, &n) == FND_OK && n == 1);
        CHECK(fnd_read(h, got, 1, &n) == FND_OK && n == 0);
        CHECK(fnd_fb_seek(h, info.size + 1) == FND_E_BAD_ARG);
        args[0] = FND_SEEK_END; args[1] = 0xFFFFFFFFu; args[2] = 0xFFFFFFFFu;
        CHECK(fnd_call(h, FND_ST_SEEK, args, 0, 0, 0, 0, &rep) == FND_OK && rep.a0 == 0 && rep.a1 == info.size - 1);
        CHECK(fnd_read(h, got, 2, &n) == FND_OK && n == 1 && got[0] == data[0]);
    }
    fnd_stat_t stat;
    CHECK(fnd_stream_stat(h, &stat) == FND_OK && stat.type == FND_TYPE_DEVICE &&
          stat.size_lo == info.size && (stat.flags & FND_STAT_SEEKABLE));
    CHECK(fnd_sync(h) == FND_OK);
    CHECK(fnd_call(h, FND_ST_TRUNCATE, 0, 0, 0, 0, 0, 0) == FND_E_UNSUPPORTED);
    int32_t dup = fnd_sys_handle_dup(h);
    CHECK(dup > 0 && fnd_close(h) == FND_OK);
    CHECK(fnd_file_open("/dev/fb0", FND_OPEN_WRITE, FND_MODE_DEFAULT, &second) == FND_E_BUSY);
    CHECK(fnd_fb_get_info((uint32_t)dup, &info) == FND_OK);
    CHECK(fnd_close((uint32_t)dup) == FND_OK);
    CHECK(fnd_file_open("/dev/fb0", FND_OPEN_READ, FND_MODE_DEFAULT, &h) == FND_OK);
    CHECK(fnd_fb_clut_read(h, 0, 256, got) == FND_OK);
    for (uint32_t i = 0; i < sizeof saved_palette; ++i) CHECK(got[i] == saved_palette[i]);
    CHECK(fnd_write(h, data, 1, &n) == FND_E_ACCESS);
    CHECK(fnd_fb_clut_write(h, 0, 1, data) == FND_E_ACCESS);
    CHECK(fnd_fb_set_format(h, 0) == FND_E_ACCESS);
    CHECK(fnd_call(h, FND_ST_TRUNCATE, 0, 0, 0, 0, 0, 0) == FND_E_ACCESS);
    CHECK(fnd_close(h) == FND_OK);
    /* A failed stream-handle allocation must release its temporary lease. */
    int32_t factory = fnd_sys_svc_lookup("dev.fb0");
    CHECK(factory > 0);
    uint32_t handles[64], count = 0;
    int32_t extra;
    while ((extra = fnd_sys_handle_dup((uint32_t)factory)) > 0) handles[count++] = (uint32_t)extra;
    CHECK(extra == FND_E_LIMIT);
    args[0] = FND_OPEN_WRITE;
    CHECK(fnd_call((uint32_t)factory, FND_DEV_OPEN, args, 0, 0, 0, 0, 0) == FND_E_LIMIT);
    for (uint32_t i = 0; i < count; ++i) CHECK(fnd_close(handles[i]) == FND_OK);
    CHECK(fnd_call((uint32_t)factory, FND_DEV_OPEN, args, 0, 0, 0, 0, &rep) == FND_OK);
    CHECK((rep.flags & FND_MSG_XFER_HANDLE) && fnd_close(rep.a3) == FND_OK);
    args[0] = FND_OPEN_EXEC;
    CHECK(fnd_call((uint32_t)factory, FND_DEV_OPEN, args, 0, 0, 0, 0, 0) == FND_E_BAD_ARG);
    CHECK(fnd_close((uint32_t)factory) == FND_OK);
    CHECK(fnd_file_open("/dev/fb0", FND_OPEN_WRITE, FND_MODE_DEFAULT, &h) == FND_OK);
    CHECK(fnd_read(h, got, 1, &n) == FND_E_ACCESS);
    CHECK(fnd_fb_clut_read(h, 0, 1, got) == FND_E_ACCESS);
    CHECK(fnd_close(h) == FND_OK);
    fnd_con_write("fb0 passed\n");
    return 0;
}
