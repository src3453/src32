#include "cache.h"
#include <foundation/abi.h>

static struct window {
    uint32_t drive, lba, count, age;
    uint8_t data[DISK_CACHE_SECTORS * 512u] __attribute__((aligned(4)));
} windows[DISK_CACHE_WINDOWS];

static void forget(struct window *w) {
    if (w->count) {
        for (uint32_t i = 0; i < DISK_CACHE_WINDOWS; ++i)
            if (windows[i].count && windows[i].age > w->age)
                --windows[i].age;
        w->count = 0;
    }
}

void disk_cache_drop(uint32_t drive) {
    for (uint32_t i = 0; i < DISK_CACHE_WINDOWS; ++i)
        if (drive == DISK_CACHE_ALL || windows[i].drive == drive)
            forget(&windows[i]);
}

void disk_cache_write(uint32_t drive, uint32_t lba, uint32_t count) {
    for (uint32_t i = 0; i < DISK_CACHE_WINDOWS; ++i) {
        struct window *w = &windows[i];
        /* Subtractions avoid overflowing an untrusted lba + count. */
        if (count && w->count && w->drive == drive &&
            (lba >= w->lba ? lba - w->lba < w->count : w->lba - lba < count))
            forget(w);
    }
}

/* Bounded LRU ranks, with no timestamp wrap after a long-running session. */
static void touch(struct window *w) {
    uint32_t age = w->age;
    for (uint32_t i = 0; i < DISK_CACHE_WINDOWS; ++i)
        if (windows[i].count && windows[i].age < age)
            ++windows[i].age;
    w->age = 0;
}

int32_t disk_cache_read(uint32_t drive, uint32_t lba, uint32_t count, uint32_t capacity,
                        disk_read_fn read, const uint8_t **out) {
    if (!count || count > DISK_CACHE_SECTORS)
        return FND_E_BAD_ARG;
    struct window *victim = &windows[0];
    for (uint32_t i = 0; i < DISK_CACHE_WINDOWS; ++i) {
        struct window *w = &windows[i];
        if (w->count && w->drive == drive && lba >= w->lba && count <= w->count &&
            lba - w->lba <= w->count - count) {
            touch(w);
            *out = w->data + (lba - w->lba) * 512u;
            return FND_OK;
        }
        if (!w->count || (victim->count && w->age > victim->age))
            victim = w;
    }
    forget(victim); /* failed/partial reads must never publish bytes */
    *out = victim->data;
    uint32_t n = lba < capacity ? capacity - lba : 0;
    if (n > DISK_CACHE_SECTORS)
        n = DISK_CACHE_SECTORS;
    if (drive >= 4 || n < count)
        return read(drive, lba, count, victim->data);
    int32_t status = read(drive, lba, n, victim->data);
    if (status != FND_OK) {
        disk_cache_drop(DISK_CACHE_ALL);
        /* Speculation may cross an unreadable sector after the requested
         * range. Preserve exact-read success in that case; do not cache it. */
        return read(drive, lba, count, victim->data);
    }
    victim->drive = drive;
    victim->lba = lba;
    victim->age = DISK_CACHE_WINDOWS;
    victim->count = n;
    touch(victim);
    return FND_OK;
}
