/* Exercise actual syscall validation with translated host buffers. */
#include <assert.h>
#include <stdint.h>
#include <string.h>
#include <foundation/abi.h>
#define FOUNDATION_KERNEL_INTERNAL_H
#define FND_PLATFORM_TANG
#define FND_CAP_DISK 4u
#define CPT32_PERM_R 1u
#define CPT32_PERM_W 2u
struct fnd_proc { uint32_t caps; };
struct fnd_thread { struct fnd_proc *proc; };
_Alignas(4) static uint8_t user[0x20000];
static uint8_t disk_first[512], disk_last[512];
static int deny;
static int proc_user_range_ok(struct fnd_proc *p, uint32_t a, uint32_t n, uint32_t perm) {
    (void)p; (void)perm;
    return !deny && a < sizeof user && n <= sizeof user - a;
}
static void *physical(uintptr_t a) {
    if (a == 0x20000000u) return disk_first;
    if (a == 0x20fffe00u) return disk_last;
    assert(a < 0x20000000u || a >= 0x21000000u);
    return (void *)a;
}
static void mm_copy_from_user(void *dst, struct fnd_proc *p, uint32_t src, uint32_t n) {
    (void)p; memcpy(physical((uintptr_t)dst), user + src, n);
}
static void mm_copy_to_user(struct fnd_proc *p, uint32_t dst, const void *src, uint32_t n) {
    (void)p; memcpy(user + dst, physical((uintptr_t)src), n);
}
#include "../../kernel/blk.c"
int main(void) {
    struct fnd_proc p = {4}; struct fnd_thread t = {&p};
    uint32_t *req = (void *)user;
    req[0] = 0; req[1] = 3; req[4] = 0x100;
    assert(sys_blk_io(&t, 0) == FND_OK);
    uint32_t *info = (void *)(user + 0x100);
    assert(info[0] == 2 && info[1] == 512 && info[2] == 32768 && info[3] == 1);
    req[0] = 1; assert(sys_blk_io(&t, 0) == FND_E_NO_MEDIA); req[0] = 0;
    req[1] = 2; req[2] = 32767; req[3] = 1;
    memset(user + 0x100, 0xa5, 512);
    assert(sys_blk_io(&t, 0) == FND_OK && disk_last[511] == 0xa5);
    req[1] = 1; memset(user + 0x100, 0, 512);
    assert(sys_blk_io(&t, 0) == FND_OK && user[0x2ff] == 0xa5);
    req[2] = 0; disk_first[0] = 0x42;
    assert(sys_blk_io(&t, 0) == FND_OK && user[0x100] == 0x42);
    req[2] = 32768; assert(sys_blk_io(&t, 0) == FND_E_BAD_ARG);
    req[2] = 0xffffffffu; assert(sys_blk_io(&t, 0) == FND_E_BAD_ARG);
    req[2] = 32767; req[3] = 2; assert(sys_blk_io(&t, 0) == FND_E_BAD_ARG);
    req[3] = 0; assert(sys_blk_io(&t, 0) == FND_E_BAD_ARG);
    req[3] = 129; assert(sys_blk_io(&t, 0) == FND_E_BAD_ARG);
    req[3] = 1; req[2] = 0; req[4] = sizeof user - 256;
    assert(sys_blk_io(&t, 0) == FND_E_ACCESS);
    req[4] = 0x101; assert(sys_blk_io(&t, 0) == FND_E_BAD_ARG);
    req[4] = 0x100; req[5] = 1; assert(sys_blk_io(&t, 0) == FND_E_BAD_ARG);
    req[5] = 0; p.caps = 0; assert(sys_blk_io(&t, 0) == FND_E_ACCESS);
    p.caps = 4; deny = 1; assert(sys_blk_io(&t, 0) == FND_E_ACCESS);
    assert(!blk_pending()); blk_irq(); blk_proc_exited(&p);
    return 0;
}
