/* Run the real disk cache against deterministic media and injected errors. */
#include <assert.h>
#include <stdio.h>
#include <string.h>
#include <foundation/abi.h>
#include "cache.h"

static uint8_t media[4][1024 * 512];
static uint32_t reads, last_count, fail_large, fail_all;

static int32_t read_sectors(uint32_t drive, uint32_t lba, uint32_t count, void *buf) {
    ++reads;
    last_count = count;
    if (fail_all || (fail_large && count > 8) || drive >= 4 || lba >= 1024 || count > 1024 - lba) {
        memset(buf, 0xEE, count * 512); /* a failed read may have written bytes */
        return FND_E_IO;
    }
    memcpy(buf, media[drive] + lba * 512, count * 512);
    return FND_OK;
}

static const uint8_t *get(uint32_t drive, uint32_t lba, uint32_t count, uint32_t cap) {
    const uint8_t *out = 0;
    assert(disk_cache_read(drive, lba, count, cap, read_sectors, &out) == FND_OK);
    assert(memcmp(out, media[drive] + lba * 512, count * 512) == 0);
    return out;
}

static void working_set(void) {
    /* The observed metadata -> echo -> metadata -> two WAL windows trace. */
    uint32_t trace[] = {0, 256, 0, 512, 640};
    for (uint32_t round = 0; round < 3; ++round)
        for (uint32_t i = 0; i < 5; ++i)
            get(0, trace[i], 8, 1024);
    assert(reads == 4);
    get(0, 7, 8, 1024); /* nonzero offset inside a window */
    assert(reads == 4);
}

static void replacement(void) {
    for (uint32_t i = 0; i < 4; ++i) get(0, i * 128, 8, 1024);
    get(0, 0, 8, 1024); /* retain recently used first window */
    get(0, 512, 8, 1024);
    get(0, 0, 8, 1024);
    assert(reads == 5);
    get(0, 128, 8, 1024); /* least recently used window was evicted */
    assert(reads == 6);
    /* Repeated invalidation must preserve LRU ranks without an age clock. */
    for (uint32_t i = 0; i < 100; ++i) {
        disk_cache_write(0, 128, 1);
        get(0, 128, 8, 1024);
        get(0, 0, 8, 1024);
    }
    assert(reads == 106);
}

static void writes(void) {
    get(0, 0, 8, 1024);
    get(0, 100, 8, 1024); /* hit in first window */
    get(0, 120, 16, 1024); /* overlapping second window [120,248) */
    get(1, 0, 8, 1024);
    assert(reads == 3);
    disk_cache_write(0, 124, 8); /* invalidates both overlapping windows */
    memset(media[0] + 124 * 512, 0xA5, 8 * 512);
    get(0, 124, 8, 1024);
    get(0, 0, 8, 1024);
    get(1, 0, 8, 1024);
    assert(reads == 5);
    disk_cache_write(0, 0xFFFFFFFEu, 8); /* no addition overflow */
    get(0, 0, 8, 1024);
    assert(reads == 5);
}

static void identify(void) {
    get(0, 0, 8, 1024);
    get(0, 256, 8, 1024);
    get(1, 0, 8, 1024);
    disk_cache_drop(0);
    memset(media[0], 0x77, sizeof media[0]); /* replacement media */
    get(0, 0, 8, 1024);
    get(0, 256, 8, 1024);
    get(1, 0, 8, 1024);
    assert(reads == 5);
}

static void failed_prefetch(void) {
    get(1, 0, 8, 1024);
    fail_large = 1;
    get(0, 256, 8, 1024); /* failed prefetch, successful exact retry */
    assert(reads == 3 && last_count == 8);
    fail_large = 0;
    get(1, 0, 8, 1024); /* speculation error discarded all cached windows */
    get(0, 256, 8, 1024); /* exact retry was not published as a full window */
    assert(reads == 5);
}

static void failed_read(void) {
    get(0, 0, 8, 1024);
    fail_all = 1;
    const uint8_t *out;
    assert(disk_cache_read(0, 256, 8, 1024, read_sectors, &out) == FND_E_IO);
    assert(reads == 3);
    fail_all = 0;
    get(0, 0, 8, 1024);
    get(0, 256, 8, 1024);
    assert(reads == 5); /* no poisoned or previously cached bytes survive */
}

static void boundaries(void) {
    get(0, 1020, 4, 1024);
    assert(last_count == 4);
    get(0, 1023, 1, 1024);
    assert(reads == 1);
    get(1, 0, 8, 0); /* unknown capacity uses exact, uncached reads */
    assert(last_count == 8);
    get(1, 0, 8, 0);
    assert(reads == 3);
    const uint8_t *out;
    assert(disk_cache_read(0, 0, 0, 1024, read_sectors, &out) == FND_E_BAD_ARG);
    assert(disk_cache_read(0, 0, 129, 1024, read_sectors, &out) == FND_E_BAD_ARG);
    assert(reads == 3);
}

int main(int argc, char **argv) {
    assert(argc == 2);
    for (uint32_t d = 0; d < 4; ++d)
        for (uint32_t i = 0; i < sizeof media[d]; ++i)
            media[d][i] = (uint8_t)(d * 31 + i / 512 * 7 + i % 512);
    if (!strcmp(argv[1], "working_set")) working_set();
    else if (!strcmp(argv[1], "replacement")) replacement();
    else if (!strcmp(argv[1], "writes")) writes();
    else if (!strcmp(argv[1], "identify")) identify();
    else if (!strcmp(argv[1], "failed_prefetch")) failed_prefetch();
    else if (!strcmp(argv[1], "failed_read")) failed_read();
    else if (!strcmp(argv[1], "boundaries")) boundaries();
    else assert(0);
    puts("PASS");
    return 0;
}
