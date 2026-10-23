#ifndef FOUNDATION_SGU_H
#define FOUNDATION_SGU_H
#include <foundation/fnd.h>
/* /dev/sgu: MMIO 0x80020000..0x80021fff, then 1 MiB shared PCMRAM. */
#define FND_SGU_CORE0 0u
#define FND_SGU_CORE1 0x1000u
#define FND_SGU_PCM 0x2000u
#define FND_SGU_SIZE 0x102000u
static inline fnd_status fnd_sgu_seek(fnd_handle h, uint32_t offset) {
    uint32_t args[4] = {FND_SEEK_SET, 0, offset, 0};
    return fnd_call(h, FND_ST_SEEK, args, 0, 0, 0, 0, 0);
}
#endif
