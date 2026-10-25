/* Frame sharing, rollback, allocation failure and complete release. */
#include "internal.h"
#include "images.h"
static struct fnd_proc a, b, c, pressure;
static uint32_t failures, baseline, pid;
#define CHECK(x) do { if (!(x) && !failures) failures = __LINE__; } while (0)
static void memory_checks(void) {
    baseline = mm_vm_free_bytes();
    CHECK(mm_vm_create(&a) == FND_OK);
    CHECK(mm_vm_map(&a, 0x800000, 0x803000, CPT32_PERM_R | CPT32_PERM_W) == FND_OK);
    CHECK(mm_vm_map(&a, 0x900000, 0x901000, CPT32_PERM_R | CPT32_PERM_X) == FND_OK);
    mm_write32(&a, 0x800000, 123);
    uint32_t before = mm_vm_free_bytes(), run;
    CHECK(mm_vm_clone(&b, &a) == FND_OK);
    CHECK(before - mm_vm_free_bytes() == 2 * MM_PAGE); /* root + one L2, no data */
    CHECK(mm_vm_ptr(&a, 0x800000, &run) == mm_vm_ptr(&b, 0x800000, &run));
    CHECK(mm_vm_cow(&a, 0x900000) == FND_E_ACCESS);
    CHECK(!proc_user_range_ok(&b, 0x900000, 4, CPT32_PERM_W));
    CHECK(proc_user_range_ok(&b, 0x800000, 4, CPT32_PERM_W));
    mm_write32(&b, 0x800000, 456);
    CHECK(mm_read32(&a, 0x800000) == 123 && mm_read32(&b, 0x800000) == 456);
    CHECK(mm_vm_clone(&c, &a) == FND_OK);
    /* Exhaust frames. Failed fork (including partial child tables) releases
     * its allocations; a COW write must leave the shared data untouched. */
    CHECK(mm_vm_create(&pressure) == FND_OK);
    uint32_t addr = 0x1000;
    while (mm_vm_free_bytes() > 7 * MM_PAGE) {
        if (mm_vm_map(&pressure, addr, addr + MM_PAGE, CPT32_PERM_R | CPT32_PERM_W) != FND_OK)
            break;
        addr += MM_PAGE;
    }
    /* Three L1 regions need four tables: failure after sharing a leaf. */
    CHECK(mm_vm_map(&a, 0x1000000, 0x1001000, CPT32_PERM_R | CPT32_PERM_W) == FND_OK);
    CHECK(mm_vm_map(&a, 0x1400000, 0x1401000, CPT32_PERM_R | CPT32_PERM_W) == FND_OK);
    struct fnd_proc failed = {0};
    before = mm_vm_free_bytes();
    CHECK(mm_vm_clone(&failed, &a) == FND_E_NO_MEMORY);
    CHECK(!failed.pt_root && mm_vm_free_bytes() == before);
    CHECK(mm_vm_cow(&a, 0x1000000) == FND_E_ACCESS); /* failed clone kept W */
    while (mm_vm_free_bytes()) {
        if (mm_vm_map(&pressure, addr, addr + MM_PAGE, CPT32_PERM_R | CPT32_PERM_W) != FND_OK)
            break;
        addr += MM_PAGE;
    }
    CHECK(mm_vm_cow(&a, 0x801000) == FND_E_NO_MEMORY);
    CHECK(!proc_user_range_ok(&a, 0x801000, 4, CPT32_PERM_W));
    CHECK(mm_read32(&a, 0x800000) == 123);
    mm_release(&pressure);
    mm_release(&b);
    mm_release(&c);
    before = mm_vm_free_bytes();
    CHECK(mm_vm_cow(&a, 0x801000) == FND_OK);
    CHECK(mm_vm_free_bytes() == before); /* last owner needs no copy */
    mm_release(&a);
    CHECK(mm_vm_free_bytes() == baseline);
}
static uint32_t on_tick(uint32_t ticks) {
    struct fnd_proc *p = fnd_proc_get(pid);
    if (p && p->state == FND_PROC_EXITED) {
        if (p->exit_code != 42) return 0x20000u + (uint32_t)p->exit_code;
        if (mm_vm) CHECK(mm_vm_free_bytes() == baseline);
        return failures ? 0x100 + failures : 42;
    }
    return ticks > 300 ? 0x200 : 0;
}
int main(void) {
    mm_init();
    if (mm_vm) memory_checks();
    int32_t result = fnd_proc_spawn_wax("fork", FND_PRIO_APP, img_guest, img_guest_size, 0, 0, 0, 0);
    if (result < 0) return 2;
    pid = (uint32_t)result;
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
