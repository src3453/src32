/* Processes, user slots and protection regions (doc/spec_Foundation.md §5,
 * §9.2, §9.4). Processes come from code linked into the kernel image
 * (phase 1) or from a PROTECTED WAX loaded by wax.c. */
#include "internal.h"
#include "wax.h"
#include <cpt32/bmc.h>

static struct fnd_proc procs[FND_MAX_PROCS];
/* proc_spawn's copy of the child's startup block (§9.3) before it is validated. */
static uint8_t spawn_startup[FND_STARTUP_MAX] __attribute__((aligned(4)));
static uint8_t startup_blocks[FND_MAX_PROCS][FND_STARTUP_MAX] __attribute__((aligned(4)));
/* proc_spawn copies the image here before validating it (§9.2): the kernel
 * heap (§5.1), which FNDBOOT only uses before the first process runs. */
#define STAGING 0x00100000u
#define STAGING_END 0x00400000u /* the kernel heap (§5.1) */
static uint32_t next_pid = 1;
static uint32_t boot_drive;

void fnd_set_boot_drive(uint32_t drive) { boot_drive = drive; }

static const uint32_t slot_bounds[FND_SLOTS + 1u] = {
    0x00400000u, 0x00480000u, 0x00500000u, 0x00580000u,
    0x00600000u, 0x00800000u, 0x00C00000u, 0x01000000u};

uint32_t fnd_slot_base(uint32_t slot) { return slot < FND_SLOTS ? slot_bounds[slot] : 0; }
uint32_t fnd_slot_end(uint32_t slot) { return slot < FND_SLOTS ? slot_bounds[slot + 1u] : 0; }

/* §5.2: a slot that does not lie wholly in the installed RAM is unusable.
 * With V the slots are only address ranges and need no RAM of their own. */
static int slot_present(uint32_t slot) { return mm_vm || slot_bounds[slot + 1u] <= mm_ram_bytes(); }

/* The vm backend's user address space (doc/foundation_vm_plan.md §3): images
 * and libraries are placed below VM_WINDOW_END, which keeps clear of the
 * MMIO windows that grants map at their own addresses; the heap may grow up
 * to VM_HEAP_TOP or the next reservation above it. Page 0 is never mapped. */
#define VM_WINDOW_START 0x00001000u
#define VM_WINDOW_END 0x10000000u
#define VM_HEAP_TOP 0x80000000u
#define VM_NO_SLOT FND_SLOTS /* proc->slot of a process that has none */

#define zero_range kmem_zero

static void maybe_free(struct fnd_proc *proc);

/* §9.4: threads DEAD, handles closed (PEER_GONE, CLIENT_GONE, services),
 * owned ranges zeroed, slot released, waiters woken. The record stays
 * readable (state EXITED) until its PROCESS handles are closed and the
 * record is reused. */
void proc_exit(struct fnd_proc *proc, int32_t code, uint32_t fault_epc) {
    if (proc->state != FND_PROC_LIVE)
        return;
    sched_kill_proc(proc);
    blk_proc_exited(proc);
    proc->threads = 0;
    proc->exit_code = code;
    proc->fault_epc = fault_epc;
    proc->state = FND_PROC_EXITED;
    handle_close_all(proc);
    mm_release(proc); /* the vm backend frees its frames here; they are zeroed when handed out */
    for (uint32_t i = 0; !mm_vm && i < proc->region_count; ++i) {
        if (proc->regions[i].owned)
            zero_range(proc->regions[i].start, proc->regions[i].end);
    }
    ipc_proc_exited(proc);
    maybe_free(proc);
}

/* A spawned process's record goes when it has exited and its last
 * PROCESS handle is closed (§9.4). Records of kernel-started modules, which
 * have no handles, stay until their slot is reused. */
static void maybe_free(struct fnd_proc *proc) {
    if (proc->state == FND_PROC_EXITED && proc->refs == 0 && proc->parent != 0)
        proc->state = FND_PROC_FREE;
}

void proc_release(struct fnd_proc *proc) {
    if (proc->refs)
        --proc->refs;
    maybe_free(proc);
}

struct fnd_proc *fnd_proc_get(uint32_t pid) {
    for (uint32_t i = 0; i < FND_MAX_PROCS; ++i) {
        if (procs[i].state != FND_PROC_FREE && procs[i].pid == pid)
            return &procs[i];
    }
    return 0;
}

/* A free process record for slot, or NULL with *error set. */
static struct fnd_proc *claim_record(uint32_t slot, int32_t *error) {
    struct fnd_proc *proc = 0;
    for (uint32_t i = 0; i < FND_MAX_PROCS; ++i) {
        if (!mm_vm && procs[i].state == FND_PROC_LIVE && procs[i].slot == slot) {
            *error = FND_E_ADDRESS_CONFLICT;
            return 0;
        }
        if (!proc && procs[i].state == FND_PROC_FREE)
            proc = &procs[i];
    }
    for (uint32_t i = 0; !proc && i < FND_MAX_PROCS; ++i) {
        if (procs[i].state == FND_PROC_EXITED && procs[i].refs == 0)
            proc = &procs[i]; /* nobody can wait for it any more */
    }
    *error = FND_E_LIMIT;
    if (proc) {
        proc->pt_root = 0;
        proc->vm_pages = 0;
    }
    return proc;
}

/* Fills in a claimed record whose regions are set and readies its thread:
 * U mode at entry with IE = 1, R1..R27 and R31 zero (§9.2). */
static int32_t start_proc(struct fnd_proc *proc, struct fnd_thread *thread, const char *name,
                          uint32_t slot, uint32_t priority, uint32_t entry, uint32_t usp, uint32_t ugp) {
    proc->pid = next_pid++;
    proc->slot = slot;
    proc->priority = priority;
    proc->threads = 1;
    proc->refs = 0;
    for (uint32_t i = 0; i < FND_MAX_HANDLES; ++i)
        proc->handles[i] = (struct fnd_handle_entry){FND_OBJ_NONE, 0};
    proc->exit_code = 0;
    proc->fault_epc = 0;
    proc->cpu_cycles = 0;
    unsigned n = 0;
    for (; name && name[n] && n < sizeof proc->name - 1u; ++n)
        proc->name[n] = name[n];
    for (; n < sizeof proc->name; ++n)
        proc->name[n] = 0;
    proc->state = FND_PROC_LIVE;
    for (unsigned i = 0; i < 32u; ++i)
        thread->frame.r[i] = 0;
    thread->frame.r[28] = usp;
    thread->frame.r[30] = ugp;
    thread->frame.epc = entry;
    thread->frame.status = CPT32_STATUS_PU | CPT32_STATUS_PIE;
    thread->priority = priority;
    thread->proc = proc;
    sched_make_ready(thread);
    return (int32_t)proc->pid;
}

int32_t fnd_proc_create_builtin(const char *name, uint32_t slot, uint32_t priority,
                                uint32_t ram_size, uint32_t code_start, uint32_t code_end,
                                uint32_t entry, uint32_t arg) {
    mm_init();
    if (slot >= FND_SLOTS || priority > FND_PRIO_APP)
        return FND_E_BAD_ARG;
    if (ram_size < 64u || (ram_size & 7u) || ram_size > slot_bounds[slot + 1u] - slot_bounds[slot])
        return FND_E_BAD_ARG;
    if (code_start >= code_end || entry < code_start || entry >= code_end || (entry & 3u))
        return FND_E_BAD_ARG;
    if (!slot_present(slot))
        return FND_E_NO_MEMORY;
    uint32_t end = slot_bounds[slot + 1u], ram = end - ram_size;
    if (code_start < end && slot_bounds[slot] < code_end)
        return FND_E_ADDRESS_CONFLICT; /* code must not lie in the writable slot */
    int32_t error;
    struct fnd_proc *proc = claim_record(slot, &error);
    if (!proc)
        return error;
    struct fnd_thread *thread = sched_alloc_thread();
    if (!thread)
        return FND_E_LIMIT;

    if (mm_vm) {
        /* The code stays where the kernel image put it (VA = PA); the data
         * is private frames at the slot's addresses. */
        int32_t status = mm_vm_create(proc);
        if (status == FND_OK)
            status = mm_vm_map(proc, ram, end, CPT32_PERM_R | CPT32_PERM_W);
        if (status == FND_OK)
            status = mm_vm_map_phys(proc, code_start, code_end, code_start, CPT32_PERM_R | CPT32_PERM_X, 0);
        if (status != FND_OK) {
            mm_release(proc);
            return status;
        }
    } else {
        zero_range(ram, end);
    }
    proc->parent = 0;
    proc->caps = 0;
    proc->heap_start = proc->heap_end = proc->heap_limit = 0;
    proc->startup_len = 0;
    /* Region 0: stack and data at the top of the slot (RW). Region 1: the
     * code (RX). */
    proc->regions[0] = (struct fnd_region){ram, end, CPT32_PERM_R | CPT32_PERM_W, 1, 0};
    proc->regions[1] = (struct fnd_region){code_start, code_end, CPT32_PERM_R | CPT32_PERM_X, 0, 0};
    proc->region_count = 2;
    uint32_t sp = end - 8u;
    mm_write32(proc, sp, arg);
    return start_proc(proc, thread, name, slot, priority, entry, sp, slot_bounds[slot]);
}

/* Loader categories to kernel codes (doc/spec_Foundation.md §9.2). */
static int32_t wax_status(enum wax_error error) {
    switch (error) {
    case WAX_OK:
        return FND_OK;
    case WAX_E_UNSUPPORTED:
    case WAX_E_ABI_MISMATCH:
        return FND_E_UNSUPPORTED;
    case WAX_E_DEPENDENCY_MISSING:
    case WAX_E_DEPENDENCY_CONFLICT:
    case WAX_E_DEPENDENCY_CYCLE:
    case WAX_E_SYMBOL_MISSING:
    case WAX_E_SYMBOL_MISMATCH:
        return FND_E_DEPENDENCY;
    case WAX_E_REGION_LIMIT:
        return FND_E_REGION_LIMIT;
    case WAX_E_RESOURCE_LIMIT:
        return FND_E_LIMIT;
    default:
        return FND_E_FORMAT;
    }
}

/* MMIO windows that may be granted (§10.3). VRAM and PCMRAM go to one
 * process at a time. */
static const struct {
    uint32_t start, end, exclusive;
} grant_windows[] = {
    {0x80000000u, 0x80010000u, 0}, /* VDP */
    {0x80010000u, 0x80020000u, 0}, /* SGC */
    {0x80020000u, 0x80030000u, 0}, /* SGU */
    {0x80040000u, 0x80040030u, 0}, /* PeC UART and RNG */
    {0x80040030u, 0x80040180u, 0}, /* PeC input */
    {0x10000000u, 0x18000000u, 1}, /* VRAM */
    {0x18000000u, 0x18100000u, 1}, /* PCMRAM */
};
#define GRANT_WINDOWS (sizeof grant_windows / sizeof grant_windows[0])

/* Index of the window that holds the whole grant, or -1. */
static int grant_window(const struct fnd_grant *g) {
    for (unsigned i = 0; i < GRANT_WINDOWS; ++i) {
        if (grant_windows[i].start <= g->start && g->end <= grant_windows[i].end)
            return (int)i;
    }
    return -1;
}

/* A live process already holds part of an exclusive window. */
static int window_taken(unsigned window) {
    for (uint32_t p = 0; p < FND_MAX_PROCS; ++p) {
        if (procs[p].state != FND_PROC_LIVE)
            continue;
        for (uint32_t i = 0; i < procs[p].region_count; ++i) {
            const struct fnd_region *r = &procs[p].regions[i];
            if (r->mmio && r->start < grant_windows[window].end && grant_windows[window].start < r->end)
                return 1;
        }
    }
    return 0;
}

static int32_t check_grants(const struct fnd_grant *grants, uint32_t count) {
    if (count > FND_MAX_GRANTS || (count && !grants))
        return FND_E_BAD_ARG;
    for (uint32_t i = 0; i < count; ++i) {
        const struct fnd_grant *g = &grants[i];
        if (g->start >= g->end || (g->perm != CPT32_PERM_R && g->perm != (CPT32_PERM_R | CPT32_PERM_W)))
            return FND_E_BAD_ARG;
        int window = grant_window(g);
        if (window < 0)
            return FND_E_ACCESS;
        if (grant_windows[window].exclusive && window_taken((unsigned)window))
            return FND_E_BUSY;
        if (mm_vm) {
            /* The hardware maps whole pages (§5.5): every window the page
             * range reaches must be grantable and, if exclusive, free. The
             * rest of a shared page (PeC) stays reachable; see the spec. */
            uint32_t first = g->start & ~MM_PAGE_MASK, last = (g->end + MM_PAGE_MASK) & ~MM_PAGE_MASK;
            for (uint32_t at = first; at < last;) {
                int found = -1;
                for (unsigned w = 0; w < GRANT_WINDOWS; ++w) {
                    if ((grant_windows[w].start & ~MM_PAGE_MASK) <= at && at < grant_windows[w].end) {
                        found = (int)w;
                        break;
                    }
                }
                if (found < 0)
                    return FND_E_ACCESS;
                if (grant_windows[found].exclusive && window_taken((unsigned)found))
                    return FND_E_BUSY;
                at = (grant_windows[found].end + MM_PAGE_MASK) & ~MM_PAGE_MASK;
            }
        }
    }
    return FND_OK;
}

/* One launch at a time: the kernel runs with interrupts off. */
static struct wax_launch launch;

/* Plans the heap of a loaded image (§5.3 step 3): it starts at the end of
 * the highest WAX block and may grow up to the next reservation above it
 * (stack or a library) or the end of the slot. */
static void plan_heap(struct fnd_proc *proc, uint32_t slot, const struct fnd_grant *grants,
                      uint32_t grant_count) {
    uint32_t top = launch.top, limit = mm_vm ? VM_HEAP_TOP : slot_bounds[slot + 1u];
    proc->heap_region = FND_PROC_REGIONS; /* none yet */
    for (uint32_t i = 0; i < launch.region_count; ++i) {
        const struct wax_region *r = &launch.region[i];
        if (r->start >= top && r->start < limit)
            limit = r->start;
        if (!mm_vm && i != 0 && r->end == top && r->flags == (CPT32_PERM_R | CPT32_PERM_W))
            proc->heap_region = i; /* grow this one */
    }
    if (mm_vm) {
        /* Whole pages of their own, so the heap never shares a page with a
         * block of other permissions; MMIO grants are in the way too. */
        for (uint32_t i = 0; i < grant_count; ++i) {
            if (grants[i].start >= top && grants[i].start < limit)
                limit = grants[i].start & ~MM_PAGE_MASK;
        }
        proc->heap_start = proc->heap_end = (top + MM_PAGE_MASK) & ~MM_PAGE_MASK;
        proc->heap_limit = limit & ~MM_PAGE_MASK;
        return;
    }
    /* 8-byte aligned, like every mem_grow delta: blocks end anywhere. */
    proc->heap_start = proc->heap_end = (top + 7u) & ~7u;
    proc->heap_limit = limit;
}

/* Where the loader finds process addresses of a vm launch. */
static uint8_t *vm_load_map(void *context, uint32_t address, uint32_t *run) {
    return mm_vm_ptr(context, address, run);
}

/* The common part of every WAX launch. startup is kernel memory, copied into
 * the new process's startup block. */
static int32_t spawn_image(const char *name, uint32_t priority, uint32_t caps, uint32_t parent,
                           const void *image, uint32_t size, const struct fnd_image *libs,
                           uint32_t lib_count, const struct fnd_grant *grants, uint32_t grant_count,
                           const void *startup, uint32_t startup_len) {
    struct wax_library table[FND_MAX_LIBRARIES];
    uint32_t base, slot = 0;
    mm_init();
    if (priority > FND_PRIO_APP || lib_count > FND_MAX_LIBRARIES || (lib_count && !libs) ||
        startup_len > FND_STARTUP_MAX)
        return FND_E_BAD_ARG;
    int32_t status = check_grants(grants, grant_count);
    if (status != FND_OK)
        return status;
    enum wax_error error = wax_base_addr(image, size, &base);
    if (error != WAX_OK)
        return wax_status(error);
    if (mm_vm) {
        slot = VM_NO_SLOT; /* any address in the window will do */
    } else {
        while (slot < FND_SLOTS && !(slot_bounds[slot] <= base && base < slot_bounds[slot + 1u]))
            ++slot;
        if (slot == FND_SLOTS)
            return FND_E_FORMAT; /* §5.2: the image must lie in one user slot */
        if (!slot_present(slot))
            return FND_E_NO_MEMORY;
    }
    struct fnd_proc *proc = claim_record(slot, &status);
    if (!proc)
        return status;
    for (uint32_t i = 0; i < lib_count; ++i) {
        table[i].data = libs[i].data;
        table[i].size = libs[i].size;
    }
    struct wax_target target = {"foundation", FND_ABI_MAJOR, FND_ABI_MINOR,
                                mm_vm ? VM_WINDOW_START : slot_bounds[slot],
                                mm_vm ? VM_WINDOW_END : slot_bounds[slot + 1u]};
    if ((error = wax_plan(&launch, image, size, table, lib_count, &target)) != WAX_OK)
        return wax_status(error);
    if (!mm_vm && launch.region_count + grant_count > FND_REGIONS)
        return FND_E_REGION_LIMIT; /* §5.3: the grants come after the image */
    struct fnd_thread *thread = sched_alloc_thread();
    if (!thread)
        return FND_E_LIMIT;

    if (mm_vm) {
        /* The whole address space is built before anything is loaded, so
         * running out of frames leaves no trace either. Grants map MMIO
         * pages at their own addresses; a grant is rounded out to whole pages. */
        status = mm_vm_create(proc);
        for (uint32_t i = 0; status == FND_OK && i < launch.region_count; ++i)
            status = mm_vm_map(proc, launch.region[i].start, launch.region[i].end, launch.region[i].flags);
        for (uint32_t i = 0; status == FND_OK && i < grant_count; ++i)
            status = mm_vm_map_phys(proc, grants[i].start, grants[i].end, grants[i].start, grants[i].perm, 1);
        if (status != FND_OK) {
            mm_release(proc);
            return status == FND_E_ADDRESS_CONFLICT ? FND_E_ACCESS : status;
        }
    }

    /* Nothing has changed until here, so every failure above leaves no trace. */
    if (mm_vm)
        wax_load_mapped(&launch, vm_load_map, proc);
    else
        wax_load(&launch, 0);
    for (uint32_t i = 0; i < launch.region_count; ++i) {
        proc->regions[i] = (struct fnd_region){launch.region[i].start, launch.region[i].end,
                                               launch.region[i].flags, 1, 0};
    }
    for (uint32_t i = 0; i < grant_count; ++i) {
        proc->regions[launch.region_count + i] =
            (struct fnd_region){grants[i].start, grants[i].end, grants[i].perm, 0, 1};
    }
    proc->region_count = launch.region_count + grant_count;
    proc->parent = parent;
    proc->caps = caps;
    plan_heap(proc, slot, grants, grant_count);
    uint32_t index = (uint32_t)(proc - procs);
    kmem_copy((uint32_t)(uintptr_t)startup_blocks[index], (uint32_t)(uintptr_t)startup, startup_len);
    proc->startup_len = startup_len;
    return start_proc(proc, thread, name, slot, priority, launch.entry, launch.stack_top, base);
}

/* The startup block of a bundled module (§11): argv = [name], cwd = "/",
 * no roles. */
static uint32_t module_startup(const char *name, uint32_t *block) {
    uint32_t n = 0;
    while (name && name[n] && n < 15u)
        ++n;
    uint8_t *bytes = (uint8_t *)block;
    uint32_t name_off = sizeof(fnd_startup) + 4u, cwd_off = name_off + n + 1u;
    uint32_t size = (cwd_off + 2u + 3u) & ~3u;
    for (uint32_t i = 0; i < size; ++i)
        bytes[i] = 0;
    fnd_startup *head = (fnd_startup *)block;
    *head = (fnd_startup){FND_STARTUP_MAGIC, FND_STARTUP_VERSION, size, 1u, sizeof(fnd_startup),
                          cwd_off, 0, size};
    block[sizeof(fnd_startup) / 4u] = name_off;
    for (uint32_t i = 0; i < n; ++i)
        bytes[name_off + i] = (uint8_t)name[i];
    bytes[cwd_off] = '/';
    return size;
}

int32_t fnd_proc_spawn_module(const char *name, uint32_t priority, uint32_t caps, const void *image,
                              uint32_t size, const struct fnd_image *libs, uint32_t lib_count,
                              const struct fnd_grant *grants, uint32_t grant_count) {
    uint32_t block[16];
    uint32_t len = module_startup(name, block);
    if (caps & ~FND_CAP_ALL)
        return FND_E_BAD_ARG;
    return spawn_image(name, priority, caps, 0, image, size, libs, lib_count, grants, grant_count, block, len);
}

int32_t fnd_proc_spawn_wax(const char *name, uint32_t priority, const void *image, uint32_t size,
                           const struct fnd_image *libs, uint32_t lib_count,
                           const struct fnd_grant *grants, uint32_t grant_count) {
    uint32_t block[16];
    uint32_t len = module_startup(name, block);
    return spawn_image(name, priority, 0, 0, image, size, libs, lib_count, grants, grant_count, block,
                       len);
}

/* proc_spawn (§9.2). Every user pointer is checked before anything changes;
 * the image is copied to STAGING first, so the caller cannot change it
 * while it is validated. Returns a PROCESS handle. */
int32_t proc_spawn(struct fnd_proc *caller, uint32_t addr) {
    struct fnd_grant grants[FND_MAX_GRANTS];
    char name[16];
    fnd_spawn sp;
    if (addr & 3u)
        return FND_E_BAD_ARG;
    if (!proc_user_range_ok(caller, addr, sizeof(fnd_spawn), CPT32_PERM_R))
        return FND_E_ACCESS;
    mm_copy_from_user(&sp, caller, addr, sizeof sp);
    if ((sp.libs & 3u) || sp.priority > FND_PRIO_APP || (sp.caps & ~FND_CAP_ALL) ||
        ((sp.handles | sp.grants) & 3u))
        return FND_E_BAD_ARG;
    if (sp.image_len > FND_SPAWN_IMAGE_MAX || sp.startup_len > FND_STARTUP_MAX ||
        sp.handle_count > FND_SPAWN_MAX_HANDLES || sp.grant_count > FND_MAX_GRANTS)
        return FND_E_LIMIT;
    if (!proc_user_range_ok(caller, sp.image, sp.image_len, CPT32_PERM_R) ||
        !proc_user_range_ok(caller, sp.startup, sp.startup_len, CPT32_PERM_R) ||
        !proc_user_range_ok(caller, sp.handles, 4u * sp.handle_count, CPT32_PERM_R) ||
        !proc_user_range_ok(caller, sp.grants, 12u * sp.grant_count, CPT32_PERM_R))
        return FND_E_ACCESS;
    if ((sp.caps & ~caller->caps) || (sp.priority < caller->priority && !(caller->caps & FND_CAP_PRIO)) ||
        (sp.grant_count && !(caller->caps & FND_CAP_MMIO)))
        return FND_E_ACCESS; /* §9.1 */
    for (uint32_t i = 0;; ++i) {
        if (i == sizeof name)
            return FND_E_BAD_ARG;
        if (!proc_user_range_ok(caller, sp.name + i, 1u, CPT32_PERM_R))
            return FND_E_ACCESS;
        name[i] = (char)mm_read8(caller, sp.name + i);
        if (!name[i])
            break;
    }
    uint32_t handles[FND_SPAWN_MAX_HANDLES];
    mm_copy_from_user(handles, caller, sp.handles, 4u * sp.handle_count);
    for (uint32_t i = 0; i < sp.handle_count; ++i) {
        uint32_t h = handles[i];
        if (h == 0 || h > FND_MAX_HANDLES || caller->handles[h - 1u].kind == FND_OBJ_NONE)
            return FND_E_BAD_HANDLE;
    }
    if (handle_free_count(caller) == 0)
        return FND_E_LIMIT; /* no room for the PROCESS handle */
    for (uint32_t i = 0; i < sp.grant_count; ++i) {
        uint32_t g[3];
        mm_copy_from_user(g, caller, sp.grants + 12u * i, 12u);
        grants[i] = (struct fnd_grant){g[0], g[1], g[2]};
    }
    /* The WALs follow the image in the staging area (§9.2). */
    struct fnd_image libs[FND_SPAWN_MAX_LIBS];
    uint32_t list[2u + 2u * FND_SPAWN_MAX_LIBS];
    uint32_t lib_count = 0, at = (STAGING + sp.image_len + 3u) & ~3u;
    if (sp.libs) {
        if (!proc_user_range_ok(caller, sp.libs, 8u, CPT32_PERM_R))
            return FND_E_ACCESS;
        mm_copy_from_user(list, caller, sp.libs, 8u);
        lib_count = list[0];
        if (lib_count > FND_SPAWN_MAX_LIBS || list[1])
            return FND_E_BAD_ARG;
        if (!proc_user_range_ok(caller, sp.libs + 8u, 8u * lib_count, CPT32_PERM_R))
            return FND_E_ACCESS;
        mm_copy_from_user(list + 2, caller, sp.libs + 8u, 8u * lib_count);
        for (uint32_t i = 0; i < lib_count; ++i) {
            uint32_t image = list[2u + 2u * i], len = list[3u + 2u * i];
            if (!proc_user_range_ok(caller, image, len, CPT32_PERM_R))
                return FND_E_ACCESS;
            if (len > STAGING_END - at)
                return FND_E_LIMIT;
            libs[i] = (struct fnd_image){(const void *)(uintptr_t)at, len};
            at = (at + len + 3u) & ~3u;
        }
        for (uint32_t i = 0; i < lib_count; ++i)
            mm_copy_from_user((void *)(uintptr_t)libs[i].data, caller, list[2u + 2u * i], libs[i].size);
    }
    mm_copy_from_user((void *)(uintptr_t)STAGING, caller, sp.image, sp.image_len);
    mm_copy_from_user(spawn_startup, caller, sp.startup, sp.startup_len);
    int32_t pid = spawn_image(name, sp.priority, sp.caps, caller->pid, (const void *)(uintptr_t)STAGING,
                              sp.image_len, libs, lib_count, grants, sp.grant_count, spawn_startup,
                              sp.startup_len);
    if (pid < 0)
        return pid;
    struct fnd_proc *child = fnd_proc_get((uint32_t)pid);
    for (uint32_t i = 0; i < sp.handle_count; ++i)
        handle_dup_into(caller, handles[i], child); /* 1 + index: the table is empty */
    return handle_add(caller, FND_OBJ_PROCESS, child);
}

int32_t proc_startup(const struct fnd_proc *proc, uint32_t buf, uint32_t cap) {
    uint32_t len = proc->startup_len;
    if (cap > len)
        cap = len;
    if (!proc_user_range_ok(proc, buf, cap, CPT32_PERM_W))
        return FND_E_ACCESS;
    mm_copy_to_user(proc, buf, startup_blocks[proc - procs], cap);
    return (int32_t)len;
}

/* mem_grow (§9, §5.3): extends the heap by delta bytes (a multiple of 8;
 * v1 never shrinks it) and returns the old end. */
int32_t proc_mem_grow(struct fnd_proc *proc, uint32_t delta) {
    if ((delta & 7u) || (int32_t)delta < 0)
        return FND_E_BAD_ARG;
    uint32_t old = proc->heap_end;
    if (delta == 0)
        return (int32_t)old;
    if (!proc->heap_limit || old > proc->heap_limit || delta > proc->heap_limit - old)
        return FND_E_NO_MEMORY;
    if (mm_vm) {
        /* Frames are zeroed when handed out, and need not be adjacent. */
        int32_t status = mm_vm_map(proc, old, old + delta, CPT32_PERM_R | CPT32_PERM_W);
        if (status != FND_OK)
            return status;
        proc->heap_end = old + delta;
        mm_reload(proc);
        return (int32_t)old;
    }
    if (proc->heap_region == FND_PROC_REGIONS) {
        if (proc->region_count == FND_REGIONS)
            return FND_E_REGION_LIMIT;
        proc->heap_region = proc->region_count++;
        proc->regions[proc->heap_region] =
            (struct fnd_region){old, old, CPT32_PERM_R | CPT32_PERM_W, 1, 0};
    }
    /* From the region's end: when it is the image's last block, the bytes
     * up to the aligned heap_start join it too. */
    uint32_t from = proc->regions[proc->heap_region].end;
    for (; from & 3u; ++from)
        *(volatile uint8_t *)(uintptr_t)from = 0;
    zero_range(from, old + delta);
    proc->heap_end = old + delta;
    proc->regions[proc->heap_region].end = proc->heap_end;
    mm_reload(proc);
    return (int32_t)old;
}

/* sys_info (§9): sel 0 is fnd_sysinfo, sel 1 the process list, sel 2 CPU
 * accounting, sel 3 fnd_meminfo. Copies up to cap bytes and returns the
 * whole answer's size. */
#define SYSINFO_BYTES 32u
#define PROCINFO_BYTES 48u
#define MEMINFO_BYTES 16u

static uint32_t proc_mem_bytes(const struct fnd_proc *proc) {
    if (mm_vm)
        return mm_vm_proc_bytes(proc);
    uint32_t bytes = 0;
    for (uint32_t i = 0; i < proc->region_count; ++i) {
        if (proc->regions[i].owned)
            bytes += proc->regions[i].end - proc->regions[i].start;
    }
    return bytes;
}

static void put_word(const struct fnd_proc *proc, uint32_t buf, uint32_t cap, uint32_t offset,
                     uint32_t value) {
    if (offset + 4u <= cap)
        mm_write32(proc, buf + offset, value);
}

int32_t proc_sys_info(const struct fnd_proc *caller, uint32_t sel, uint32_t buf, uint32_t cap) {
    uint32_t live = 0, used = 0, records = 0;
    for (uint32_t i = 0; i < FND_MAX_PROCS; ++i) {
        if (procs[i].state == FND_PROC_LIVE) {
            ++live;
            used += mm_vm ? 0 : slot_bounds[procs[i].slot + 1u] - slot_bounds[procs[i].slot];
        }
        if (procs[i].state != FND_PROC_FREE)
            ++records;
    }
    uint32_t size;
    if (sel == 0)
        size = SYSINFO_BYTES;
    else if (sel == 1)
        size = records * PROCINFO_BYTES;
    else if (sel == 2)
        size = 16u + records * 8u;
    else if (sel == 3)
        size = MEMINFO_BYTES;
    else
        return FND_E_BAD_ARG;
    if (cap > size)
        cap = size;
    if (cap && (buf & 3u))
        return FND_E_BAD_ARG;
    if (!proc_user_range_ok(caller, buf, cap, CPT32_PERM_W))
        return FND_E_ACCESS;
    if (sel == 2) {
        uint32_t info[4];
        sched_cpu_snapshot(info);
        for (uint32_t w = 0; w < 4u; ++w)
            put_word(caller, buf, cap, 4u * w, info[w]);
        uint32_t offset = 16u;
        for (uint32_t i = 0; i < FND_MAX_PROCS; ++i) {
            if (procs[i].state == FND_PROC_FREE)
                continue;
            put_word(caller, buf, cap, offset, procs[i].pid);
            put_word(caller, buf, cap, offset + 4u, procs[i].cpu_cycles);
            offset += 8u;
        }
        return (int32_t)size;
    }
    if (sel == 3) {
        uint32_t info[4] = {mm_ram_bytes(), bmc_vram_bytes(), bmc_pcmram_bytes(), 0};
        for (uint32_t w = 0; w < 4u; ++w)
            put_word(caller, buf, cap, 4u * w, info[w]);
        return (int32_t)size;
    }
    if (sel == 0) {
        uint32_t user = 0;
        for (uint32_t slot = 0; !mm_vm && slot < FND_SLOTS; ++slot) {
            if (slot_present(slot))
                user += slot_bounds[slot + 1u] - slot_bounds[slot];
        }
        uint32_t info[8] = {FND_ABI_MAJOR, FND_ABI_MINOR, FND_TICK_HZ, mm_ram_bytes(),
                            mm_vm ? mm_vm_free_bytes() : user - used, live,
                            boot_drive, 0};
        for (uint32_t w = 0; w < 8u; ++w)
            put_word(caller, buf, cap, 4u * w, info[w]);
        return (int32_t)size;
    }
    uint32_t offset = 0;
    for (uint32_t i = 0; i < FND_MAX_PROCS; ++i) {
        const struct fnd_proc *p = &procs[i];
        if (p->state == FND_PROC_FREE)
            continue;
        uint32_t state = p->state == FND_PROC_EXITED ? 2u : sched_proc_runnable(p) ? 0u : 1u;
        uint32_t info[8] = {p->pid, p->parent, state, p->priority,
                            (uint32_t)p->exit_code, p->state == FND_PROC_LIVE ? proc_mem_bytes(p) : 0,
                            p->threads, p->fault_epc};
        for (uint32_t w = 0; w < 8u; ++w)
            put_word(caller, buf, cap, offset + 4u * w, info[w]);
        for (uint32_t w = 0; w < 4u; ++w) {
            uint32_t word = 0;
            for (uint32_t b = 0; b < 4u; ++b)
                word = word << 8 | (uint8_t)p->name[4u * w + b];
            put_word(caller, buf, cap, offset + 32u + 4u * w, word);
        }
        offset += PROCINFO_BYTES;
    }
    return (int32_t)size;
}
