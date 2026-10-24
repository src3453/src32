#include <foundation/fnd.h>
#include <foundation/sys.h>
#define CHECK(x) do { if (!(x)) fnd_exit(__LINE__); } while (0)
static uint8_t data[4101];
int main(void) {
    fnd_init();
    fnd_handle h, other;
    uint32_t got;
    fnd_stat_t st;
    const char *paths[] = {"/dev/trng", "/dev/prng", "/dev/uart0"};
    for (unsigned i = 0; i < 3; ++i) {
        CHECK(fnd_path_stat(paths[i], &st) == FND_OK && st.type == FND_TYPE_DEVICE);
        CHECK(fnd_file_open(paths[i], FND_OPEN_READ, FND_MODE_DEFAULT, &h) == FND_OK);
        CHECK(fnd_file_open(paths[i], FND_OPEN_READ, FND_MODE_DEFAULT, &other) == FND_OK);
        CHECK(fnd_close(other) == FND_OK);
        CHECK(fnd_stream_stat(h, &st) == FND_OK && st.type == FND_TYPE_DEVICE);
        CHECK(st.mode == (i == 2 ? 0x36u : 0x24u));
        CHECK(st.flags == (i == 2 ? 0u : FND_STAT_READ_ONLY));
        CHECK(fnd_call(h, FND_ST_READ, 0, 0, 0, 0, 0, 0) == FND_OK);
        CHECK(fnd_call(h, FND_ST_WRITE, 0, 0, 0, 0, 0, 0) == FND_E_ACCESS);
        CHECK(fnd_call(h, FND_ST_SEEK, 0, 0, 0, 0, 0, 0) == FND_E_UNSUPPORTED);
        CHECK(fnd_sync(h) == FND_OK);
        if (i < 2) {
            CHECK(fnd_read(h, data, sizeof data, &got) == FND_OK && got == sizeof data);
            CHECK(fnd_close(h) == FND_OK);
            CHECK(fnd_file_open(paths[i], FND_OPEN_WRITE, FND_MODE_DEFAULT, &h) == FND_E_ACCESS);
        } else CHECK(fnd_close(h) == FND_OK);
    }
    fnd_con_write("pec passed\n");
    return 0;
}
