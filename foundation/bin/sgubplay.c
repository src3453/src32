#include "cmd.h"
#include <foundation/sgu.h>
#include <cpt32/sgub.h>

#define MAX_STREAM 0x00E00000u
typedef struct { fnd_handle handle; fnd_status error; uint32_t position; } sound_output;

/* The shared parser validates the complete file before calling this. Open
 * lazily so invalid/unsupported files do not even silence the current SGU. */
static int32_t sound_write(void *context, uint8_t target, uint32_t address,
                           const uint8_t *data, uint32_t size) {
    sound_output *out = context;
    if (out->error < 0) return out->error;
    if (!out->handle)
        out->error = fnd_file_open("/dev/sgu", FND_OPEN_WRITE, FND_MODE_DEFAULT, &out->handle);
    uint32_t offset = target == 2 ? FND_SGU_PCM + address : target * 0x1000u + address;
    if (out->error == FND_OK && out->position != offset) {
        out->error = fnd_sgu_seek(out->handle, offset);
        if (out->error == FND_OK) out->position = offset;
    }
    while (size && out->error == FND_OK) {
        uint32_t n = size < FND_IPC_MAX_DATA ? size : FND_IPC_MAX_DATA, put = 0;
        out->error = fnd_write(out->handle, data, n, &put);
        if (out->error == FND_OK && (!put || put > n)) out->error = FND_E_IO;
        if (out->error < 0) break;
        data += put; size -= put; out->position += put;
    }
    return out->error;
}

static fnd_status load(const char *path, uint8_t **bytes, uint32_t *size) {
    fnd_handle h;
    fnd_status st = fnd_file_open(path, FND_OPEN_READ, FND_MODE_DEFAULT, &h);
    if (st < 0) return st;
    fnd_stat_t stat;
    st = fnd_stream_stat(h, &stat);
    if (st == FND_OK && (stat.type != FND_TYPE_FILE || stat.size_hi ||
                         stat.size_lo < 20u || stat.size_lo > MAX_STREAM)) st = FND_E_FORMAT;
    if (st == FND_OK) {
        *size = stat.size_lo;
        *bytes = fnd_mem_alloc(*size);
        if (!*bytes) st = FND_E_NO_MEMORY;
    }
    uint32_t pos = 0;
    while (st == FND_OK && pos < *size) {
        uint32_t got = 0, n = *size - pos;
        if (n > FND_IPC_MAX_DATA) n = FND_IPC_MAX_DATA;
        st = fnd_read(h, *bytes + pos, n, &got);
        if (st == FND_OK && (!got || got > n)) st = FND_E_FORMAT;
        pos += got;
    }
    if (st == FND_OK) {
        uint8_t extra; uint32_t got = 0;
        st = fnd_read(h, &extra, 1, &got);
        if (st == FND_OK && got) st = FND_E_FORMAT;
    }
    fnd_status closed = fnd_close(h);
    return st == FND_OK ? closed : st;
}

static int32_t sgubplay(uint32_t argc) {
    if (argc != 2) return cmd_usage("sgubplay FILE");
    const char *path = fnd_argv(1);
    uint32_t hz = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_HZ);
    if (hz < 60 || hz > 1000000u) return cmd_report("sgubplay", 0, FND_E_BAD_ARG);
    uint8_t *bytes = 0; uint32_t size = 0;
    fnd_status st = load(path, &bytes, &size);
    sound_output sound = {0, FND_OK, 0};
    sgub_output_t output = {&sound, 7u, sound_write};
    sgub_status_t result = SGUB_STATUS_ERROR_STATE;
    if (st == FND_OK) {
        result = sgub_init_output(bytes, size, &output);
        if (result < 0) st = sound.error < 0 ? sound.error :
            result == SGUB_STATUS_ERROR_TARGET ? FND_E_UNSUPPORTED : FND_E_FORMAT;
    }
    if (st == FND_OK) {
        cmd_out("sgubplay: playing "); cmd_out(path); cmd_out(" (Esc/q to stop)\n");
        result = sgub_step(); /* frame zero follows initialization */
    }
    uint32_t last = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO), phase = 0;
    while (st == FND_OK && result >= 0 && result != SGUB_STATUS_COMPLETE) {
        fnd_key_t key;
        fnd_status ks = fnd_con_pollkey(&key);
        if (ks < 0) { st = ks; break; }
        if (ks == FND_OK && (key.code == FND_KEY_ESC ||
            (key.code == FND_KEY_CHAR && (key.ch == 'q' || key.ch == 'Q')))) break;
        uint32_t now = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO), elapsed = now - last;
        last = now;
        /* Bound catch-up and avoid overflow after a long stall; never derive
         * frame timing from CPU cycles or generate an independent IRQ. */
        if (elapsed > hz) elapsed = hz;
        phase += elapsed * 60u;
        if (phase > hz * 60u) phase = hz * 60u;
        /* Poll the keyboard between every frame, including catch-up frames. */
        if (phase >= hz) {
            phase -= hz;
            result = sgub_step();
        }
        if (result >= 0 && result != SGUB_STATUS_COMPLETE && phase < hz) st = fnd_sys_sleep(1);
    }
    if (result < 0 && st == FND_OK) st = sound.error < 0 ? sound.error : FND_E_IO;
    /* Closing (also on process kill) silences both SGUs in the driver. */
    if (sound.handle) {
        fnd_status closed = fnd_close(sound.handle);
        if (st == FND_OK) st = closed;
    }
    fnd_mem_free(bytes);
    if (st < 0) return cmd_report("sgubplay", path, st);
    cmd_out(result == SGUB_STATUS_COMPLETE ? "sgubplay: complete\n" : "sgubplay: stopped\n");
    return FND_OK;
}
CMD_MAIN(sgubplay)
