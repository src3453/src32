#include <foundation/sgu.h>
#include <foundation/sys.h>
#define CHECK(x) do { if (!(x)) { fnd_con_write("DAC API failed\n"); fnd_exit(__LINE__); } } while (0)
static uint8_t pcm[4096];
int main(void) {
    fnd_init(); fnd_handle h; uint32_t done, queued;
    CHECK(fnd_file_open("/dev/sgu", FND_OPEN_READ, FND_MODE_DEFAULT, &h) == FND_OK);
    CHECK(fnd_sgu_dac_config(h, 2) == FND_E_ACCESS);
    CHECK(fnd_sgu_dac_start(h) == FND_E_ACCESS);
    CHECK(fnd_sgu_dac_submit(h, pcm, 4, &done, &queued) == FND_E_ACCESS);
    CHECK(fnd_close(h) == FND_OK);
    CHECK(fnd_file_open("/dev/sgu", FND_OPEN_WRITE, FND_MODE_DEFAULT, &h) == FND_OK);
    CHECK(fnd_sgu_dac_start(h) == FND_E_BAD_ARG);
    CHECK(fnd_sgu_dac_config(h, 3) == FND_E_BAD_ARG);
    CHECK(fnd_sgu_dac_config(h, 2) == FND_OK);
    CHECK(fnd_sgu_dac_submit(h, pcm, 3, &done, &queued) == FND_E_BAD_ARG);
    CHECK(fnd_sgu_dac_submit(h, pcm, 4092, &done, &queued) == FND_OK && done == 4092 && queued == 1023);
    CHECK(fnd_sgu_dac_submit(h, pcm, 4096, &done, &queued) == FND_OK && done == 4096 && queued == 2047);
    CHECK(fnd_sgu_dac_submit(h, pcm, 8, &done, &queued) == FND_OK && done == 4 && queued == 2048);
    CHECK(fnd_sgu_dac_submit(h, pcm, 4, &done, &queued) == FND_E_WOULD_BLOCK && !done && queued == 2048);
    CHECK(fnd_sgu_dac_submit(h, 0, 0, &done, &queued) == FND_OK && !done && queued == 2048);
    CHECK(fnd_sgu_dac_start(h) == FND_OK);
    CHECK(fnd_sys_sleep(10) == FND_OK);
    CHECK(fnd_sgu_dac_submit(h, 0, 0, &done, &queued) == FND_OK && !queued);
    CHECK(fnd_sgu_dac_config(h, 1) == FND_OK);
    CHECK(fnd_sgu_dac_submit(h, pcm, 4096, &done, &queued) == FND_OK && done == 4096 && queued == 2048);
    CHECK(fnd_close(h) == FND_OK);
    fnd_con_write("DAC API passed\n"); fnd_exit(0);
}
