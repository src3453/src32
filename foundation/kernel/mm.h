#ifndef FOUNDATION_KERNEL_MM_H
#define FOUNDATION_KERNEL_MM_H
#include <stdint.h>

/* Memory management layer (doc/foundation_vm_plan.md, spec_Foundation.md §5).
 *
 * Two backends sit behind this interface and the kernel picks one at its first
 * process start from CPUID (CPU_FEATURES bit 9, Extension V):
 *
 *   slot  No V. Every process owns a fixed physical slot (§5.2) and the CPU
 *         checks it with the Extension P protection regions (§5.3).
 *   vm    V present. Every process has its own page table; user addresses are
 *         virtual and the kernel (S mode, untranslated) reaches them through
 *         the page table in software. No slots, no region limit.
 *
 * Kernel code never dereferences a user address. It checks it with
 * proc_user_range_ok and moves data with the mm_copy_* helpers, which are the
 * plain loads and stores of the slot backend. */

struct fnd_proc;

/* 1 once the vm backend is selected. Valid after mm_init. */
extern int mm_vm;

/* Picks the backend (idempotent). Called before the first process exists. */
void mm_init(void);

/* The installed Main RAM in bytes (§5.1). */
uint32_t mm_ram_bytes(void);

/* §5.3: makes proc the process whose memory the CPU enforces; nothing to do
 * when it already is. */
void proc_activate(struct fnd_proc *proc);
/* proc's regions or mappings changed (mem_grow): re-apply them if it is the
 * active process. */
void mm_reload(struct fnd_proc *proc);
/* proc exits: stop enforcing it and release what the backend holds for it
 * (the page table and frames of vm). */
void mm_release(struct fnd_proc *proc);

/* §5.4: no 32-bit overflow, nothing at or above 0xFFFF0000, and every byte
 * in memory of proc that has all of perm (CPT32_PERM_*). MMIO is never
 * accepted. VM write validation also resolves COW before any output or block;
 * returns 0 if a private frame cannot be allocated. */
int proc_user_range_ok(const struct fnd_proc *proc, uint32_t addr, uint32_t len, uint32_t perm);

/* Copies for addresses proc_user_range_ok accepted. kmem_copy and kmem_zero
 * work on physical (kernel) addresses. kmem_copy may use synchronous DMAC
 * ch1 for disjoint Main RAM ranges >= FND_COPY_DMA_MIN (default 256 bytes).
 * User translations/permission checks still belong to the mm_copy_* caller. */
void kmem_copy(uint32_t dst, uint32_t src, uint32_t len);
void kmem_zero(uint32_t start, uint32_t end);

void mm_vm_copy_from(void *dst, const struct fnd_proc *proc, uint32_t src, uint32_t len);
void mm_vm_copy_to(const struct fnd_proc *proc, uint32_t dst, const void *src, uint32_t len);
void mm_vm_copy(const struct fnd_proc *to, uint32_t dst, const struct fnd_proc *from, uint32_t src,
                uint32_t len);

static inline void mm_copy_from_user(void *dst, const struct fnd_proc *proc, uint32_t src, uint32_t len) {
    if (mm_vm)
        mm_vm_copy_from(dst, proc, src, len);
    else
        kmem_copy((uint32_t)(uintptr_t)dst, src, len);
}

static inline void mm_copy_to_user(const struct fnd_proc *proc, uint32_t dst, const void *src, uint32_t len) {
    if (mm_vm)
        mm_vm_copy_to(proc, dst, src, len);
    else
        kmem_copy(dst, (uint32_t)(uintptr_t)src, len);
}

/* From one process to another (IPC). */
static inline void mm_copy_user(const struct fnd_proc *to, uint32_t dst, const struct fnd_proc *from,
                                uint32_t src, uint32_t len) {
    if (mm_vm)
        mm_vm_copy(to, dst, from, src, len);
    else
        kmem_copy(dst, src, len);
}

static inline uint32_t mm_read32(const struct fnd_proc *proc, uint32_t addr) {
    uint32_t value;
    mm_copy_from_user(&value, proc, addr, 4u);
    return value;
}

static inline void mm_write32(const struct fnd_proc *proc, uint32_t addr, uint32_t value) {
    mm_copy_to_user(proc, addr, &value, 4u);
}

static inline uint32_t mm_read8(const struct fnd_proc *proc, uint32_t addr) {
    uint8_t value;
    mm_copy_from_user(&value, proc, addr, 1u);
    return value;
}

/* ---- vm backend: building an address space (proc.c, only when mm_vm) ---- */

#define MM_PAGE 4096u
#define MM_PAGE_MASK (MM_PAGE - 1u)

/* Clone VM mappings with COW; on failure child owns nothing, parent is unchanged. */
int32_t mm_vm_clone(struct fnd_proc *child, struct fnd_proc *parent);
/* Resolve an owned COW page; ACCESS for other pages, NO_MEMORY if copying fails. */
int32_t mm_vm_cow(const struct fnd_proc *proc, uint32_t va);

/* The page table of a new process: FND_E_NO_MEMORY if no frame is free. */
int32_t mm_vm_create(struct fnd_proc *proc);
/* Backs [start, end) with zeroed frames of the process, perm being
 * CPT32_PERM_*. Pages already mapped keep their frame and gain perm (a page
 * shared by two blocks is as permissive as both). FND_E_NO_MEMORY when
 * frames run out; what was mapped stays until mm_release. */
int32_t mm_vm_map(struct fnd_proc *proc, uint32_t start, uint32_t end, uint32_t perm);
/* Maps [vstart, vend) onto the physical addresses from pstart on without
 * owning them (kernel code, MMIO). dev marks a Device page. FND_E_ADDRESS_CONFLICT if a
 * page is already mapped. */
int32_t mm_vm_map_phys(struct fnd_proc *proc, uint32_t vstart, uint32_t vend, uint32_t pstart,
                       uint32_t perm, uint32_t dev);
/* Where user address va of proc lives and how many bytes from there stay in
 * the page; 0 if va is not mapped. */
uint8_t *mm_vm_ptr(const struct fnd_proc *proc, uint32_t va, uint32_t *run);
/* The kernel's DMA bounce buffer (MM_BOUNCE_BYTES, 4-byte aligned) for
 * transfers to or from user memory. */
#define MM_BOUNCE_BYTES 0x10000u
uint32_t mm_bounce(void);
/* Bytes of frames not yet handed out. */
uint32_t mm_vm_free_bytes(void);
/* The frame bytes proc holds as memory (not its tables). */
uint32_t mm_vm_proc_bytes(const struct fnd_proc *proc);

#endif
