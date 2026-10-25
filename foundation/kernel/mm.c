/* Memory management backends (doc/foundation_vm_plan.md); the interface is
 * mm.h. The slot backend programs the Extension P protection regions
 * (spec_Foundation.md §5.3). The vm backend keeps a page table per process for
 * Extension V (doc/spec_CPU_vm.md) and a frame pool above the kernel area. */
#include "internal.h"
#include <cpt32/bmc.h>

int mm_vm;
static int mm_decided;

/* Synchronous RAM copies reserve DMAC ch1; disk I/O owns ch0. The kernel
 * is not preempted, so no process can exit or change mappings while we wait.
 * Keep the CPU path for small copies and for platforms without the DMAC.
 * Set to 0 for a CPU-only comparison build. */
#ifndef FND_COPY_DMA_MIN
#define FND_COPY_DMA_MIN 256u
#endif
#if FND_COPY_DMA_MIN > 0
#define COPY_DMA_REG(offset) FND_REG32(0x80050000u + (offset))
#define COPY_DMA_BIT 2u
static int copy_dma(uint32_t dst, uint32_t src, uint32_t len) {
    uint32_t ram = mm_ram_bytes();
    if (src >= ram || dst >= ram || len > ram - src || len > ram - dst)
        return 0;
    /* A partial DMA followed by CPU fallback is safe only for disjoint ranges.
     * Preserve the existing forward-copy behavior for overlapping ranges. */
    if ((dst > src && dst - src < len) || (src > dst && src - dst < len))
        return 0;
    if (COPY_DMA_REG(0) != 0x444D4101u || !(FND_REG32(0x80060004u) & 1u) ||
        !(COPY_DMA_REG(4) & 1u) ||
        (COPY_DMA_REG(0x130) & 1u))
        return 0;
    uint32_t width = 0; /* byte beats also support differing alignments */
    if (((dst ^ src) & 3u) == 0) {
        while (len && (src & 3u)) {
            *(volatile uint8_t *)(uintptr_t)dst++ = *(const volatile uint8_t *)(uintptr_t)src++;
            --len;
        }
        width = 2u; /* 32-bit increment/increment */
    }
    uint32_t bytes = width ? len & ~3u : len;
    if (!bytes) {
        for (uint32_t i = 0; i < len; ++i)
            *(volatile uint8_t *)(uintptr_t)(dst + i) = *(const volatile uint8_t *)(uintptr_t)(src + i);
        return 1;
    }
    COPY_DMA_REG(8) = COPY_DMA_REG(8) | COPY_DMA_BIT;
    COPY_DMA_REG(0xC) = COPY_DMA_REG(0xC) & ~COPY_DMA_BIT; /* polling, no IRQ */
    COPY_DMA_REG(0x10) = COPY_DMA_BIT;
    COPY_DMA_REG(0x130) = 6u; /* clear this channel's DONE/ERROR */
    COPY_DMA_REG(0x138) = 0u; /* P0: share bus with CPU polling, no starvation */
    COPY_DMA_REG(0x120) = src;
    COPY_DMA_REG(0x124) = dst;
    COPY_DMA_REG(0x128) = bytes;
    COPY_DMA_REG(0x12C) = 0x100u | width;
    uint32_t status;
    do { status = COPY_DMA_REG(0x130); } while (status & 1u);
    uint32_t remain = COPY_DMA_REG(0x134);
    COPY_DMA_REG(0x130) = 6u;
    if (!(status & 2u) || (status & 4u) || remain) {
        /* The caller retries the original, disjoint ranges on the CPU. */
        return 0;
    }
    for (uint32_t i = bytes; i < len; ++i)
        *(volatile uint8_t *)(uintptr_t)(dst + i) = *(const volatile uint8_t *)(uintptr_t)(src + i);
    return 1;
}
#endif

/* By words, eight per step where it can: the compiler keeps nothing in
 * registers, so the loop overhead is most of the cost. */
void kmem_copy(uint32_t dst, uint32_t src, uint32_t len) {
    if (dst == src) return;
#if FND_COPY_DMA_MIN > 0
    if (len >= FND_COPY_DMA_MIN && copy_dma(dst, src, len)) return;
#endif
    uint32_t at = 0;
    if (((dst | src) & 3u) == 0) {
        volatile uint32_t *d = (volatile uint32_t *)(uintptr_t)dst;
        const volatile uint32_t *s = (const volatile uint32_t *)(uintptr_t)src;
        for (; at + 32u <= len; at += 32u, d += 8, s += 8) {
            d[0] = s[0], d[1] = s[1], d[2] = s[2], d[3] = s[3];
            d[4] = s[4], d[5] = s[5], d[6] = s[6], d[7] = s[7];
        }
        for (; at + 4u <= len; at += 4u)
            *d++ = *s++;
    }
    volatile uint8_t *d = (volatile uint8_t *)(uintptr_t)dst;
    const volatile uint8_t *s = (const volatile uint8_t *)(uintptr_t)src;
    for (; at < len; ++at)
        d[at] = s[at];
}

void kmem_zero(uint32_t start, uint32_t end) {
    volatile uint32_t *d = (volatile uint32_t *)(uintptr_t)start;
    uint32_t n = (end - start) / 4u;
    for (; n >= 8u; n -= 8u, d += 8)
        d[0] = 0, d[1] = 0, d[2] = 0, d[3] = 0, d[4] = 0, d[5] = 0, d[6] = 0, d[7] = 0;
    for (; n; --n)
        *d++ = 0;
}

/* §5.1: the installed Main RAM as the BMC reports it; hardware without the
 * register is taken to have v1's 16 MiB. */
#define RAM_BYTES_FALLBACK 0x01000000u

uint32_t mm_ram_bytes(void) {
    uint32_t bytes = bmc_main_ram_bytes();
    return bytes ? bytes : RAM_BYTES_FALLBACK;
}

/* ---- slot backend ---- */

static struct fnd_proc *active; /* process whose memory the CPU enforces */
static uint32_t active_regions; /* regions with nonzero FLAGS on the CPU */

/* MTSR takes a constant system register, so each region field gets its own
 * writer. Separate noinline functions keep the optimizer from merging the
 * calls into one MTSR with a computed register number. */
#define REGION_WRITERS(n)                                                                    \
    static __attribute__((noinline)) void start_##n(uint32_t v) { cpt32_mtsr(CPT32_SR_RGN_START(n), v); } \
    static __attribute__((noinline)) void end_##n(uint32_t v) { cpt32_mtsr(CPT32_SR_RGN_END(n), v); }     \
    static __attribute__((noinline)) void flags_##n(uint32_t v) { cpt32_mtsr(CPT32_SR_RGN_FLAGS(n), v); }
REGION_WRITERS(0)
REGION_WRITERS(1)
REGION_WRITERS(2)
REGION_WRITERS(3)
REGION_WRITERS(4)
REGION_WRITERS(5)
REGION_WRITERS(6)
REGION_WRITERS(7)
typedef void (*region_writer)(uint32_t);
static const region_writer region_writers[3][FND_REGIONS] = {
    {start_0, start_1, start_2, start_3, start_4, start_5, start_6, start_7},
    {end_0, end_1, end_2, end_3, end_4, end_5, end_6, end_7},
    {flags_0, flags_1, flags_2, flags_3, flags_4, flags_5, flags_6, flags_7}};

/* field: 0 = START, 1 = END, 2 = FLAGS. */
static void region_write(uint32_t index, uint32_t field, uint32_t value) {
    region_writers[field][index](value);
}

/* §5.3: program each region, then disable the ones the previous process had
 * beyond them. Region checks apply only in U mode (spec_CPU_priv.md), so the
 * old and new settings are never seen half-written, and the 8 + 4n MTSR
 * writes of disabling everything first (about a third of a context switch)
 * become 3n + (old - n). A switch between threads of the same process
 * leaves the regions alone. */
static void slot_activate(struct fnd_proc *proc) {
    uint32_t n = proc->region_count;
    for (uint32_t i = 0; i < n; ++i) {
        region_write(i, 0, proc->regions[i].start);
        region_write(i, 1, proc->regions[i].end);
        region_write(i, 2, proc->regions[i].flags);
    }
    for (uint32_t i = n; i < active_regions; ++i)
        region_write(i, 2, 0);
    active_regions = n;
}

static void slot_release(void) {
    for (uint32_t i = 0; i < active_regions; ++i)
        region_write(i, 2, 0);
    active_regions = 0;
}

static int slot_range_ok(const struct fnd_proc *proc, uint32_t addr, uint32_t len, uint32_t perm) {
    while (len) {
        uint32_t covered = 0;
        for (uint32_t i = 0; i < proc->region_count; ++i) {
            const struct fnd_region *r = &proc->regions[i];
            if (!r->mmio && (r->flags & perm) == perm && r->start <= addr && addr < r->end) {
                covered = r->end - addr;
                break;
            }
        }
        if (!covered)
            return 0;
        if (covered >= len)
            return 1;
        addr += covered;
        len -= covered;
    }
    return 1;
}

/* ---- vm backend ---- */

/* Frames come from [POOL_BASE, installed RAM): the area below holds the
 * kernel, its heap and the staged boot image (§5.1), which the processes
 * never see. 64K frames are 256 MiB. */
#define POOL_BASE 0x00400000u
#define MAX_FRAMES 65536u

#define PTE_V 0x01u
#define PTE_R 0x02u
#define PTE_W 0x04u
#define PTE_X 0x08u
#define PTE_U 0x10u
#define PTE_DEV 0x20u
#define PTE_OWNED 0x100u /* SW bit: the frame is the process's and is freed with it */
#define PTE_COW 0x200u /* SW bit: logically writable, shared until a write */
#define PTE_FRAME 0xFFFFF000u
#define PTE_RWX (PTE_R | PTE_W | PTE_X)

static uint8_t frame_refs[MAX_FRAMES]; /* at most FND_MAX_PROCS references */
static uint32_t pool_frames, free_frames, next_hint;

static void vm_init(void) {
    uint32_t ram = mm_ram_bytes();
    pool_frames = ram > POOL_BASE ? (ram - POOL_BASE) / MM_PAGE : 0;
    if (pool_frames > MAX_FRAMES)
        pool_frames = MAX_FRAMES;
    free_frames = pool_frames;
    next_hint = 0;
    for (uint32_t i = 0; i < MAX_FRAMES; ++i)
        frame_refs[i] = 0;
    /* The first frames are the DMA bounce buffer for good. */
    uint32_t reserved = MM_BOUNCE_BYTES / MM_PAGE;
    for (uint32_t f = 0; f < reserved && f < pool_frames; ++f) {
        frame_refs[f] = 1;
        --free_frames;
    }
    next_hint = reserved;
}

uint32_t mm_bounce(void) { return POOL_BASE; }

/* The address of a zeroed free frame, or 0. */
static uint32_t frame_alloc(void) {
    if (free_frames == 0)
        return 0;
    for (uint32_t n = 0; n < pool_frames; ++n) {
        uint32_t f = next_hint + n;
        if (f >= pool_frames)
            f -= pool_frames;
        if (!frame_refs[f]) {
            frame_refs[f] = 1;
            --free_frames;
            next_hint = f + 1u;
            uint32_t pa = POOL_BASE + f * MM_PAGE;
            kmem_zero(pa, pa + MM_PAGE);
            return pa;
        }
    }
    return 0;
}

static void frame_free(uint32_t pa) {
    uint32_t f = (pa - POOL_BASE) / MM_PAGE;
    if (pa < POOL_BASE || f >= pool_frames || !frame_refs[f])
        return;
    if (--frame_refs[f] == 0)
        ++free_frames;
}

uint32_t mm_vm_free_bytes(void) { return free_frames * MM_PAGE; }

uint32_t mm_vm_proc_bytes(const struct fnd_proc *proc) { return proc->vm_pages * MM_PAGE; }

static uint32_t *table_at(uint32_t pa) { return (uint32_t *)(uintptr_t)pa; }

int32_t mm_vm_create(struct fnd_proc *proc) {
    proc->vm_pages = 0;
    proc->pt_root = frame_alloc();
    return proc->pt_root ? FND_OK : FND_E_NO_MEMORY;
}

/* The level-2 entry for va, or 0 if its table is missing and create is 0. */
static uint32_t *l2_entry(const struct fnd_proc *proc, uint32_t va, int create) {
    uint32_t *root = table_at(proc->pt_root);
    uint32_t l1 = root[va >> 22];
    if (!(l1 & PTE_V)) {
        if (!create)
            return 0;
        uint32_t table = frame_alloc();
        if (!table)
            return 0;
        l1 = table | PTE_V; /* R = W = X = 0: a pointer to the next level */
        root[va >> 22] = l1;
    }
    return table_at(l1 & PTE_FRAME) + ((va >> 12) & 0x3FFu);
}

/* Only owned COW pages are eligible. A true RX page must stay RX. */
int32_t mm_vm_cow(const struct fnd_proc *proc, uint32_t va) {
    uint32_t *entry = proc->pt_root ? l2_entry(proc, va, 0) : 0;
    if (!entry || (*entry & (PTE_V | PTE_U | PTE_OWNED | PTE_COW | PTE_DEV)) !=
                             (PTE_V | PTE_U | PTE_OWNED | PTE_COW))
        return FND_E_ACCESS;
    uint32_t pa = *entry & PTE_FRAME;
    if (frame_refs[(pa - POOL_BASE) / MM_PAGE] > 1u) {
        uint32_t copy = frame_alloc();
        if (!copy)
            return FND_E_NO_MEMORY;
        kmem_copy(copy, pa, MM_PAGE);
        frame_free(pa);
        pa = copy;
    }
    *entry = pa | ((*entry & ~PTE_FRAME & ~PTE_COW) | PTE_W);
    mm_reload((struct fnd_proc *)proc);
    return FND_OK;
}

int32_t mm_vm_map(struct fnd_proc *proc, uint32_t start, uint32_t end, uint32_t perm) {
    uint32_t flags = PTE_V | PTE_U | ((perm & CPT32_PERM_R) ? PTE_R : 0) | ((perm & CPT32_PERM_W) ? PTE_W : 0) |
                     ((perm & CPT32_PERM_X) ? PTE_X : 0);
    for (uint32_t page = start & ~MM_PAGE_MASK; page < end; page += MM_PAGE) {
        uint32_t *entry = l2_entry(proc, page, 1);
        if (!entry)
            return FND_E_NO_MEMORY;
        if (*entry & PTE_V) {
            if ((*entry & PTE_COW) && (flags & PTE_W)) {
                int32_t status = mm_vm_cow(proc, page);
                if (status != FND_OK)
                    return status;
            }
            *entry |= flags;
        } else {
            uint32_t frame = frame_alloc();
            if (!frame)
                return FND_E_NO_MEMORY;
            *entry = frame | flags | PTE_OWNED;
            ++proc->vm_pages;
        }
        if (page + MM_PAGE < page)
            break; /* the last page of the address space */
    }
    return FND_OK;
}

int32_t mm_vm_map_phys(struct fnd_proc *proc, uint32_t vstart, uint32_t vend, uint32_t pstart,
                       uint32_t perm, uint32_t dev) {
    uint32_t flags = PTE_V | PTE_U | ((perm & CPT32_PERM_R) ? PTE_R : 0) | ((perm & CPT32_PERM_W) ? PTE_W : 0) |
                     ((perm & CPT32_PERM_X) ? PTE_X : 0) | (dev ? PTE_DEV : 0);
    uint32_t frame = pstart & PTE_FRAME;
    for (uint32_t page = vstart & ~MM_PAGE_MASK; page < vend; page += MM_PAGE, frame += MM_PAGE) {
        uint32_t *entry = l2_entry(proc, page, 1);
        if (!entry)
            return FND_E_NO_MEMORY;
        if (*entry & PTE_V) {
            /* A second grant in the same page (grants are not page aligned) */
            if ((*entry & (PTE_FRAME | PTE_DEV | PTE_OWNED)) != (frame | (dev ? PTE_DEV : 0)))
                return FND_E_ADDRESS_CONFLICT;
            *entry |= flags;
            continue;
        }
        *entry = frame | flags;
    }
    return FND_OK;
}

static void vm_destroy(struct fnd_proc *proc) {
    if (!proc->pt_root)
        return;
    uint32_t *root = table_at(proc->pt_root);
    for (uint32_t i = 0; i < 1024u; ++i) {
        if (!(root[i] & PTE_V))
            continue;
        uint32_t *table = table_at(root[i] & PTE_FRAME);
        for (uint32_t k = 0; k < 1024u; ++k) {
            if ((table[k] & (PTE_V | PTE_OWNED)) == (PTE_V | PTE_OWNED))
                frame_free(table[k] & PTE_FRAME);
        }
        frame_free(root[i] & PTE_FRAME);
    }
    frame_free(proc->pt_root);
    proc->pt_root = 0;
    proc->vm_pages = 0;
}

/* Build the child completely before write-protecting the parent. Failure
 * drops child references/tables and leaves the parent's PTEs untouched. */
int32_t mm_vm_clone(struct fnd_proc *child, struct fnd_proc *parent) {
    int32_t status = mm_vm_create(child);
    if (status != FND_OK)
        return status;
    uint32_t *root = table_at(parent->pt_root);
    for (uint32_t i = 0; i < 1024u; ++i) {
        if (!(root[i] & PTE_V))
            continue;
        uint32_t *src = table_at(root[i] & PTE_FRAME);
        uint32_t *dst = l2_entry(child, i << 22, 1);
        if (!dst) {
            status = FND_E_NO_MEMORY;
            break;
        }
        for (uint32_t k = 0; k < 1024u; ++k) {
            uint32_t pte = src[k];
            if (!(pte & PTE_V))
                continue;
            if ((pte & PTE_DEV) || ((pte & PTE_W) && !(pte & PTE_OWNED))) {
                status = FND_E_ACCESS;
                break;
            }
            if (pte & PTE_OWNED) {
                ++frame_refs[((pte & PTE_FRAME) - POOL_BASE) / MM_PAGE];
                ++child->vm_pages;
                if (pte & PTE_W)
                    pte = (pte & ~PTE_W) | PTE_COW;
            }
            dst[k] = pte;
        }
        if (status != FND_OK)
            break;
    }
    if (status != FND_OK) {
        vm_destroy(child);
        return status;
    }
    for (uint32_t i = 0; i < 1024u; ++i) {
        if (!(root[i] & PTE_V))
            continue;
        uint32_t *src = table_at(root[i] & PTE_FRAME);
        for (uint32_t k = 0; k < 1024u; ++k)
            if ((src[k] & (PTE_V | PTE_OWNED | PTE_W)) == (PTE_V | PTE_OWNED | PTE_W))
                src[k] = (src[k] & ~PTE_W) | PTE_COW;
    }
    mm_reload(parent);
    return FND_OK;
}

/* Prepare the complete destination before copying, including page-crossing
 * outputs. Checked syscall outputs cannot fail halfway through a copy.
 * A single-threaded process cannot fork while blocked on a checked output. */
static int vm_prepare_write(const struct fnd_proc *proc, uint32_t addr, uint32_t len) {
    while (len) {
        uint32_t *entry = proc->pt_root ? l2_entry(proc, addr, 0) : 0;
        if (entry && (*entry & PTE_COW) && mm_vm_cow(proc, addr) != FND_OK)
            return 0;
        uint32_t run = MM_PAGE - (addr & MM_PAGE_MASK);
        if (run >= len)
            break;
        addr += run;
        len -= run;
    }
    return 1;
}

uint8_t *mm_vm_ptr(const struct fnd_proc *proc, uint32_t va, uint32_t *run) {
    if (!proc->pt_root)
        return 0;
    uint32_t *entry = l2_entry(proc, va, 0);
    if (!entry || !(*entry & PTE_V))
        return 0;
    *run = MM_PAGE - (va & MM_PAGE_MASK);
    return (uint8_t *)(uintptr_t)((*entry & PTE_FRAME) | (va & MM_PAGE_MASK));
}

static int vm_range_ok(const struct fnd_proc *proc, uint32_t addr, uint32_t len, uint32_t perm) {
    uint32_t need = (perm & CPT32_PERM_R ? PTE_R : 0) | (perm & CPT32_PERM_W ? PTE_W : 0) |
                    (perm & CPT32_PERM_X ? PTE_X : 0);
    uint32_t start = addr, bytes = len;
    while (len) {
        uint32_t *entry = proc->pt_root ? l2_entry(proc, addr, 0) : 0;
        uint32_t flags = entry ? *entry : 0;
        if (flags & PTE_COW)
            flags |= PTE_W;
        /* A Device page is MMIO the kernel must not touch for a user. */
        if (!entry || (*entry & (PTE_V | PTE_U | PTE_DEV)) != (PTE_V | PTE_U) || (flags & need) != need)
            return 0;
        uint32_t covered = MM_PAGE - (addr & MM_PAGE_MASK);
        if (covered >= len)
            break;
        addr += covered;
        len -= covered;
    }
    return !(perm & CPT32_PERM_W) || vm_prepare_write(proc, start, bytes);
}

void mm_vm_copy_from(void *dst, const struct fnd_proc *proc, uint32_t src, uint32_t len) {
    uint32_t to = (uint32_t)(uintptr_t)dst;
    while (len) {
        uint32_t run;
        uint8_t *from = mm_vm_ptr(proc, src, &run);
        if (!from)
            return;
        if (run > len)
            run = len;
        kmem_copy(to, (uint32_t)(uintptr_t)from, run);
        to += run;
        src += run;
        len -= run;
    }
}

void mm_vm_copy_to(const struct fnd_proc *proc, uint32_t dst, const void *src, uint32_t len) {
    if (!vm_prepare_write(proc, dst, len))
        return;
    uint32_t from = (uint32_t)(uintptr_t)src;
    while (len) {
        uint32_t run;
        uint8_t *to = mm_vm_ptr(proc, dst, &run);
        if (!to)
            return;
        if (run > len)
            run = len;
        kmem_copy((uint32_t)(uintptr_t)to, from, run);
        from += run;
        dst += run;
        len -= run;
    }
}

void mm_vm_copy(const struct fnd_proc *to, uint32_t dst, const struct fnd_proc *from, uint32_t src,
                uint32_t len) {
    if (!vm_prepare_write(to, dst, len))
        return;
    while (len) {
        uint32_t dst_run, src_run;
        uint8_t *d = mm_vm_ptr(to, dst, &dst_run);
        uint8_t *s = mm_vm_ptr(from, src, &src_run);
        if (!d || !s)
            return;
        uint32_t run = dst_run < src_run ? dst_run : src_run;
        if (run > len)
            run = len;
        kmem_copy((uint32_t)(uintptr_t)d, (uint32_t)(uintptr_t)s, run);
        dst += run;
        src += run;
        len -= run;
    }
}

/* ---- interface ---- */

void mm_init(void) {
    if (mm_decided)
        return;
    mm_decided = 1;
#ifndef FND_MM_SLOT_ONLY
    if (cpt32_cpu_features() & CPT32_FEATURE_V) {
        vm_init();
        mm_vm = 1;
        /* U mode is translated from now on; S mode never is. */
        cpt32_mtsr(CPT32_SR_VCTL, CPT32_VCTL_EN);
    }
#endif
}

void proc_activate(struct fnd_proc *proc) {
    if (proc == active)
        return;
    if (mm_vm)
        cpt32_mtsr(CPT32_SR_PTBR, proc->pt_root); /* flushes the TLB: no ASID */
    else
        slot_activate(proc);
    active = proc;
}

void mm_reload(struct fnd_proc *proc) {
    if (active != proc)
        return;
    if (mm_vm) {
        cpt32_mtsr(CPT32_SR_TLBFLUSH, 0); /* the TLB keeps old translations until told otherwise */
    } else {
        active = 0;
        proc_activate(proc);
    }
}

void mm_release(struct fnd_proc *proc) {
    if (active == proc) {
        if (!mm_vm)
            slot_release();
        active = 0;
    }
    if (mm_vm)
        vm_destroy(proc);
}

int proc_user_range_ok(const struct fnd_proc *proc, uint32_t addr, uint32_t len, uint32_t perm) {
    if (len == 0)
        return 1;
    if (addr + len < addr || addr + len > 0xFFFF0000u)
        return 0;
    return mm_vm ? vm_range_ok(proc, addr, len, perm) : slot_range_ok(proc, addr, len, perm);
}
