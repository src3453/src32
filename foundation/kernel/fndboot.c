/* FNDBOOT bundled modules (doc/spec_Foundation.md §11). stage1 has already
 * checked the header fields it uses and copied the kernel; this file checks
 * the module table before anything is spawned and starts the modules. */
#include <foundation/kernel.h>

#include "mm.h"

#define FNDBOOT_MAGIC 0x464E4442u /* "FNDB" */
#define FNDBOOT_HEADER 64u
#define FNDBOOT_ENTRY 48u
#define FNDBOOT_GRANT 12u
#define FNDBOOT_MAX_SIZE 0x00200000u

static uint32_t word(const uint8_t *p) { return *(const uint32_t *)(const void *)p; }

/* [offset, offset + length) inside size, 4-byte aligned, without overflow. */
static int span_ok(uint32_t size, uint32_t offset, uint32_t length) {
    return !(offset & 3u) && offset <= size && length <= size - offset;
}

static const uint8_t *entry_at(const uint8_t *boot, uint32_t index) {
    return boot + word(boot + 32) + index * FNDBOOT_ENTRY;
}

int32_t fnd_boot_check(const void *fndboot) {
    const uint8_t *boot = fndboot;
    if ((uint32_t)(uintptr_t)boot & 3u)
        return FND_E_BAD_ARG;
    uint32_t size = word(boot + 8), count = word(boot + 28), table = word(boot + 32);
    if (word(boot) != FNDBOOT_MAGIC || word(boot + 4) != 1u || size < FNDBOOT_HEADER ||
        size > FNDBOOT_MAX_SIZE)
        return FND_E_FORMAT;
    for (uint32_t offset = 36; offset < FNDBOOT_HEADER; offset += 4)
        if (word(boot + offset))
            return FND_E_FORMAT;
    if (count == 0 || count > FND_MAX_PROCS || !span_ok(size, table, count * FNDBOOT_ENTRY))
        return FND_E_FORMAT;
    for (uint32_t i = 0; i < count; ++i) {
        const uint8_t *entry = entry_at(boot, i);
        uint32_t grants = word(entry + 20);
        if (!span_ok(size, word(entry), word(entry + 4)) || word(entry + 8) > FND_PRIO_IDLE ||
            grants > FND_MAX_GRANTS || (grants && !span_ok(size, word(entry + 16), grants * FNDBOOT_GRANT)) ||
            word(entry + 24) || word(entry + 28) || entry[32] == 0)
            return FND_E_FORMAT;
        uint32_t end = 32;
        while (end < 48u && entry[end])
            ++end;
        if (end == 48u) /* name[16] must hold its NUL */
            return FND_E_FORMAT;
    }
    return (int32_t)count;
}

int32_t fnd_boot_spawn(const void *fndboot, uint32_t index, const struct fnd_image *libs, uint32_t lib_count) {
    const uint8_t *boot = fndboot;
    const uint8_t *entry = entry_at(boot, index);
    const char *name = (const char *)entry + 32;
    /* The pipe service lives in the slot 6 range, which only the VM backend can spare (§11). */
    if (!mm_vm && name[0] == 'p' && name[1] == 'i' && name[2] == 'p' && name[3] == 'e' && !name[4])
        return 0;
    const struct fnd_grant *grants = word(entry + 20) ? (const void *)(boot + word(entry + 16)) : 0;
    return fnd_proc_spawn_module(name, word(entry + 8), word(entry + 12), boot + word(entry),
                                 word(entry + 4), libs, lib_count, grants, word(entry + 20));
}
