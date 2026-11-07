/* Micro-benchmark for the file read path used by the doom WAD backend:
 * seek, small/large reads and repeated reads of /doom.wad, in ms per call. */
#include <foundation/fnd.h>
#include <foundation/sys.h>

static uint32_t hz, lcg = 12345;
static uint8_t buf[8192] __attribute__((aligned(4)));

static uint32_t now(void) { return (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO); }

static void put_u32(uint32_t v) {
    char t[12];
    int n = 0;
    do { t[n++] = (char)('0' + v % 10u); v /= 10u; } while (v);
    char out[13];
    int o = 0;
    while (n) out[o++] = t[--n];
    out[o] = 0;
    fnd_con_write(out);
}

static void report(const char *name, uint32_t ticks, uint32_t n) {
    fnd_con_write(name);
    put_u32((uint32_t)((uint64_t)ticks * 1000000u / hz / n));
    fnd_con_write(" us/op\n");
}

static fnd_status seek(fnd_handle h, uint32_t off) {
    uint32_t args[4] = {FND_SEEK_SET, 0, off, 0};
    return fnd_call(h, FND_ST_SEEK, args, 0, 0, 0, 0, 0);
}

static uint32_t rnd(uint32_t limit) {
    lcg = lcg * 1103515245u + 12345u;
    return (lcg >> 8) % limit;
}

void main(void) {
    fnd_init();
    hz = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_HZ);
    fnd_handle h;
    if (fnd_file_open("/doom.wad", FND_OPEN_READ, FND_MODE_DEFAULT, &h) < 0) fnd_exit(1);
    const uint32_t span = 11000000u, n = 40;
    uint32_t t, got;

    t = now();
    for (uint32_t i = 0; i < n; ++i) seek(h, rnd(span));
    report("seek random          ", now() - t, n);

    t = now();
    for (uint32_t i = 0; i < n; ++i) { seek(h, rnd(span)); fnd_read(h, buf, 512, &got); }
    report("seek+read 512 random ", now() - t, n);

    t = now();
    for (uint32_t i = 0; i < n; ++i) { seek(h, rnd(span)); fnd_read(h, buf, 4096, &got); }
    report("seek+read 4096 random", now() - t, n);

    t = now();
    for (uint32_t i = 0; i < n; ++i) { seek(h, 5000001u); fnd_read(h, buf, 4096, &got); }
    report("seek+read 4096 same  ", now() - t, n);

    seek(h, 3000000u);
    t = now();
    for (uint32_t i = 0; i < n; ++i) fnd_read(h, buf, 4096, &got);
    report("read 4096 sequential ", now() - t, n);

    seek(h, 3000000u);
    t = now();
    for (uint32_t i = 0; i < n; ++i) fnd_read(h, buf, 512, &got);
    report("read 512 sequential  ", now() - t, n);
    fnd_exit(0);
}
