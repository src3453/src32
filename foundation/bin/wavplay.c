#include "cmd.h"
#include "wav_core.h"
#include <foundation/sgu.h>

static wav_decoder decoder;
static uint8_t pcm[FND_IPC_MAX_DATA];
typedef struct { fnd_handle handle; uint32_t position; } wav_file;
static fnd_status read_at(void *context, uint32_t offset, uint8_t *data, uint32_t bytes) {
    wav_file *f = context;
    if (f->position != offset) {
        uint32_t args[4] = {FND_SEEK_SET, 0, offset, 0};
        fnd_status st = fnd_call(f->handle, FND_ST_SEEK, args, 0, 0, 0, 0, 0);
        if (st < 0) return st;
        f->position = offset;
    }
    while (bytes) {
        uint32_t got = 0;
        fnd_status st = fnd_read(f->handle, data, bytes, &got);
        if (st < 0) return st;
        if (!got || got > bytes) return FND_E_FORMAT;
        bytes -= got; data += got; f->position += got;
    }
    return FND_OK;
}
static int32_t wavplay(uint32_t argc) {
    if (argc != 2) return cmd_usage("wavplay FILE");
    const char *path = fnd_argv(1);
    wav_file file = {0}; fnd_handle sound = 0;
    fnd_status st = fnd_file_open(path, FND_OPEN_READ, FND_MODE_DEFAULT, &file.handle);
    fnd_stat_t stat; wav_info info;
    if (st == FND_OK) st = fnd_stream_stat(file.handle, &stat);
    if (st == FND_OK && (stat.type != FND_TYPE_FILE || stat.size_hi)) st = FND_E_FORMAT;
    if (st == FND_OK) st = wav_parse(&file, read_at, stat.size_lo, &info);
    if (st == FND_OK) st = fnd_file_open("/dev/sgu", FND_OPEN_WRITE, FND_MODE_DEFAULT, &sound);
    if (st == FND_OK) st = fnd_sgu_dac_config(sound, info.channels);
    uint32_t used = 0, size = 0, queued = 0, started = 0, stopped = 0;
    if (st == FND_OK) {
        wav_begin(&decoder, &file, read_at, &info);
        cmd_out("wavplay: playing "); cmd_out(path); cmd_out(" (Esc/q to stop)\n");
    }
    while (st == FND_OK) {
        fnd_key_t key;
        fnd_status ks = fnd_con_pollkey(&key);
        if (ks < 0) { st = ks; break; }
        if (ks == FND_OK && (key.code == FND_KEY_ESC ||
            (key.code == FND_KEY_CHAR && (key.ch == 'q' || key.ch == 'Q')))) { stopped = 1; break; }
        if (used == size && !decoder.ended) {
            st = wav_render(&decoder, pcm, sizeof pcm, &size); used = 0;
            if (st < 0) break;
        }
        uint32_t done = 0;
        st = fnd_sgu_dac_submit(sound, pcm + used, size - used, &done, &queued);
        if (st == FND_E_WOULD_BLOCK) st = FND_OK;
        if (st < 0) break;
        if (done > size - used || done % (info.channels * 2u)) { st = FND_E_IO; break; }
        used += done;
        /* Prefill both FIFOs while paused, then enable them together. */
        if (!started && (queued == 2048 || (decoder.ended && used == size))) {
            st = fnd_sgu_dac_start(sound); started = 1;
            if (st < 0) break;
        }
        if (decoder.ended && used == size && !queued) break;
        /* Refill without sleeping until backpressure; 1 tick = 10 ms,
         * below the 42.7 ms FIFO duration. No synthetic audio IRQs. */
        if (started && (!done || used < size || (decoder.ended && used == size))) st = fnd_sys_sleep(1);
    }
    if (sound) { fnd_status closed = fnd_close(sound); if (st == FND_OK) st = closed; }
    if (file.handle) { fnd_status closed = fnd_close(file.handle); if (st == FND_OK) st = closed; }
    if (st < 0) return cmd_report("wavplay", path, st);
    cmd_out(stopped ? "wavplay: stopped\n" : "wavplay: complete\n");
    return FND_OK;
}
CMD_MAIN(wavplay)
