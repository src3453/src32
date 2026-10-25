/* Exercise the real DMAC and kernel copy helper, with guard bytes. */
#include "internal.h"
#define REG(off) FND_REG32(0x80050000u + (off))
#define SRC 0x00200000u
#define DST 0x00220000u
#define OTHER 0x00240000u
#define BYTE(addr) (*(volatile uint8_t *)(uintptr_t)(addr))
#define CHECK(expr) do { if (!(expr)) return __LINE__ + 100; } while (0)
static uint8_t pattern(uint32_t i) { return (uint8_t)(i * 37u + 11u); }
static void fill(uint32_t addr, uint32_t n, uint8_t value) {
    for (uint32_t i = 0; i < n; ++i) BYTE(addr + i) = value;
}
static int matches(uint32_t dst, uint32_t src, uint32_t n) {
    for (uint32_t i = 0; i < n; ++i)
        if (BYTE(dst + i) != BYTE(src + i)) return 0;
    return 1;
}
static struct fnd_proc from_proc, to_proc;
int main(void) {
    static const uint32_t sizes[] = {0, 1, 3, 4, 31, 32, 127, 255, 256, 257,
                                    511, 512, 4095, 4096, 4097, 8193};
    for (uint32_t i = 0; i < 65536; ++i) BYTE(SRC + i) = pattern(i);
    for (uint32_t s = 0; s < 4; ++s) for (uint32_t d = 0; d < 4; ++d)
    for (uint32_t j = 0; j < sizeof sizes / sizeof sizes[0]; ++j) {
        uint32_t n = sizes[j], to = DST + 4u + d;
        fill(DST, n + 12u, 0xCC);
        kmem_copy(to, SRC + s, n);
        CHECK(matches(to, SRC + s, n));
        for (uint32_t i = DST; i < to; ++i) CHECK(BYTE(i) == 0xCC);
        for (uint32_t i = to + n; i < DST + n + 12u; ++i) CHECK(BYTE(i) == 0xCC);
    }
    /* Overlap must keep the original forward-copy behavior (no DMA).
     * Use word-aligned ranges so the existing word loop is unambiguous. */
    for (uint32_t i = 0; i < 1024; ++i) BYTE(DST + i) = pattern(i);
    kmem_copy(DST + 4, DST, 512);
    for (uint32_t i = 4; i < 516; ++i) CHECK(BYTE(DST + i) == pattern(i % 4));
    for (uint32_t i = 0; i < 1024; ++i) BYTE(DST + i) = pattern(i);
    kmem_copy(DST, DST + 4, 512);
    for (uint32_t i = 0; i < 512; ++i) CHECK(BYTE(DST + i) == pattern(i + 4));
    kmem_copy(DST, DST, 512);
    CHECK(BYTE(DST) == pattern(4));

    /* Disabled arbitration/controller must fall back without hanging. */
    FND_REG32(0x80060004u) = 0;
    kmem_copy(DST, SRC, 1024);
    CHECK(matches(DST, SRC, 1024));
    FND_REG32(0x80060004u) = 1;
    REG(4) = 0;
    kmem_copy(DST, SRC + 1, 1024);
    CHECK(matches(DST, SRC + 1, 1024));
    REG(4) = 1;

    /* Keep a second RAM DMA active on disk's channel. Neither the enable
     * mask nor its IRQ enable/status may be destroyed by the copy helper. */
    REG(8) = REG(8) | 1u;
    REG(0xC) = 1u;
    REG(0x100) = SRC;
    REG(0x104) = OTHER;
    REG(0x108) = 16384;
    REG(0x10C) = 0x100; /* byte beats, increment/increment */
    kmem_copy(DST + 1, SRC + 1, 4096);
    CHECK(matches(DST + 1, SRC + 1, 4096));
    CHECK((REG(8) & 1u) && (REG(0xC) & 1u));
    while (REG(0x110) & 1u) {}
    CHECK((REG(0x110) & 6u) == 2u && (REG(0x10) & 1u));
    CHECK(matches(OTHER, SRC, 16384));
    REG(0x110) = 6;
    REG(0x10) = 1;
    REG(0xC) = 0;

    /* A busy ch1 must be left alone, with CPU fallback for this request. */
    REG(8) = REG(8) | 2u;
    REG(0x120) = SRC;
    REG(0x124) = OTHER;
    REG(0x128) = 65536;
    REG(0x12C) = 0x100;
    kmem_copy(DST, SRC + 1, 256);
    CHECK(REG(0x128) == 65536 && matches(DST, SRC + 1, 256));
    while (REG(0x130) & 1u) {}
    CHECK((REG(0x130) & 6u) == 2u && matches(OTHER, SRC, 65536));
    /* Stale errors must not poison the next transfer. */
    REG(0x128) = 0;
    REG(0x12C) = 0x100;
    CHECK(REG(0x130) & 4u);
    kmem_copy(DST, SRC, 4096);
    CHECK(matches(DST, SRC, 4096));
    CHECK(REG(0x120) == SRC && REG(0x124) == DST && REG(0x134) == 0);

    /* VM copies must split at BOTH processes' page boundaries. Alternate
     * frame allocation so neither virtual buffer is physically contiguous. */
    mm_init();
    CHECK(mm_vm);
    CHECK(mm_vm_create(&from_proc) == FND_OK && mm_vm_create(&to_proc) == FND_OK);
    for (uint32_t page = 0; page < 3; ++page) {
        CHECK(mm_vm_map(&from_proc, 0x800000 + page * MM_PAGE,
                       0x801000 + page * MM_PAGE, CPT32_PERM_R | CPT32_PERM_W) == FND_OK);
        CHECK(mm_vm_map(&to_proc, 0x900000 + page * MM_PAGE,
                       0x901000 + page * MM_PAGE, CPT32_PERM_R | CPT32_PERM_W) == FND_OK);
    }
    for (uint32_t page = 0; page < 3; ++page) {
        uint32_t run;
        uint8_t *p = mm_vm_ptr(&from_proc, 0x800000 + page * MM_PAGE, &run);
        for (uint32_t i = 0; i < MM_PAGE; ++i) p[i] = pattern(page * MM_PAGE + i);
    }
    for (uint32_t s = 0; s < 4; ++s) for (uint32_t d = 0; d < 4; ++d) {
        for (uint32_t page = 0; page < 3; ++page) {
            uint32_t run;
            uint8_t *p = mm_vm_ptr(&to_proc, 0x900000 + page * MM_PAGE, &run);
            fill((uint32_t)(uintptr_t)p, MM_PAGE, 0xCC);
        }
        mm_copy_user(&to_proc, 0x900004 + d, &from_proc, 0x800000 + s, 8193);
        for (uint32_t page = 0; page < 3; ++page) {
            uint32_t run;
            uint8_t *p = mm_vm_ptr(&to_proc, 0x900000 + page * MM_PAGE, &run);
            for (uint32_t i = 0; i < MM_PAGE; ++i) {
                uint32_t at = page * MM_PAGE + i;
                uint8_t expected = at >= 4 + d && at < 4 + d + 8193 ? pattern(at - 4 - d + s) : 0xCC;
                CHECK(p[i] == expected);
            }
        }
    }
    mm_release(&from_proc);
    mm_release(&to_proc);
    return 42;
}
