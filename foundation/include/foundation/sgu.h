#ifndef FOUNDATION_SGU_H
#define FOUNDATION_SGU_H
#include <foundation/fnd.h>
/* /dev/sgu: MMIO 0x80020000..0x80021fff, then 1 MiB shared PCMRAM. */
#define FND_SGU_CORE0 0u
#define FND_SGU_CORE1 0x1000u
#define FND_SGU_PCM 0x2000u
#define FND_SGU_SIZE 0x102000u
/* SGU0 ch0 (mono), or ch0/ch1 (left/right), signed big-endian PCM16.
 * CONFIG resets and pauses; START starts both channels in one kernel call.
 * SUBMIT accepts whole interleaved frames without waiting. A zero-byte call
 * queries queued frames. It does not change the stream seek position. */
#define FND_SGU_DAC_CONFIG 0x1300u
#define FND_SGU_DAC_SUBMIT 0x1301u
#define FND_SGU_DAC_START 0x1302u
static inline fnd_status fnd_sgu_dac_config(fnd_handle h, uint32_t channels) {
    uint32_t args[4] = {channels, 0, 0, 0};
    return fnd_call(h, FND_SGU_DAC_CONFIG, args, 0, 0, 0, 0, 0);
}
static inline fnd_status fnd_sgu_dac_start(fnd_handle h) {
    return fnd_call(h, FND_SGU_DAC_START, 0, 0, 0, 0, 0, 0);
}
static inline fnd_status fnd_sgu_dac_submit(fnd_handle h, const void *pcm,
        uint32_t bytes, uint32_t *done, uint32_t *queued) {
    fnd_msg rep;
    *done = 0;
    fnd_status st = fnd_call(h, FND_SGU_DAC_SUBMIT, 0, pcm, bytes, 0, 0, &rep);
    if (st == FND_OK || st == FND_E_WOULD_BLOCK) {
        *done = rep.a0; *queued = rep.a1;
    }
    return st;
}
/* Write several runs in one call, in order: each run is {offset:u32 BE, length:u16 BE,
 * data[length]} and the runs must fill the buffer exactly (at most FND_IPC_MAX_DATA bytes).
 * Offsets are stream offsets as for seek/write. Everything is validated before the first
 * byte is written, and the stream position is not changed. Returns the byte count in *done. */
#define FND_SGU_WRITE_RUNS 0x1303u
static inline fnd_status fnd_sgu_write_runs(fnd_handle h, const void *runs, uint32_t bytes,
                                            uint32_t *done) {
    fnd_msg rep;
    *done = 0;
    fnd_status st = fnd_call(h, FND_SGU_WRITE_RUNS, 0, runs, bytes, 0, 0, &rep);
    if (st == FND_OK) *done = rep.a0;
    return st;
}
static inline fnd_status fnd_sgu_seek(fnd_handle h, uint32_t offset) {
    uint32_t args[4] = {FND_SEEK_SET, 0, offset, 0};
    return fnd_call(h, FND_ST_SEEK, args, 0, 0, 0, 0, 0);
}
#endif
