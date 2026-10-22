/* Processes, user slots and protection regions (doc/spec_Foundation.md §5,
 * §9.2, §9.4). Processes come from code linked into the kernel image
 * (phase 1) or from a PROTECTED WAX loaded by wax.c. */
#include "internal.h"
#include "wax.h"

static struct fnd_proc procs[FND_MAX_PROCS];
static struct fnd_proc *active; /* process whose regions are programmed */
static uint32_t next_pid = 1;
static uint32_t boot_drive;

void fnd_set_boot_drive(uint32_t drive) { boot_drive = drive; }

static const uint32_t slot_bounds[FND_SLOTS + 1u] = {
    0x00400000u, 0x00480000u, 0x00500000u, 0x00580000u,
    0x00600000u, 0x00800000u, 0x00C00000u, 0x01000000u};

uint32_t fnd_slot_base(uint32_t slot) { return slot < FND_SLOTS ? slot_bounds[slot] : 0; }
uint32_t fnd_slot_end(uint32_t slot) { return slot < FND_SLOTS ? slot_bounds[slot + 1u] : 0; }

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

/* §5.3: disable all eight, then write START/END, then FLAGS. A switch between
 * threads of the same process leaves the regions alone. */
void proc_activate(struct fnd_proc *proc) {
    if (proc == active)
        return;
    for (uint32_t i = 0; i < FND_REGIONS; ++i)
        region_write(i, 2, 0);
    for (uint32_t i = 0; i < proc->region_count; ++i) {
        region_write(i, 0, proc->regions[i].start);
        region_write(i, 1, proc->regions[i].end);
    }
    for (uint32_t i = 0; i < proc->region_count; ++i)
        region_write(i, 2, proc->regions[i].flags);
    active = proc;
}

/* §5.4: no 32-bit overflow, nothing at or above 0xFFFF0000, and every byte
 * inside a RAM region of the process that has all of perm. */
int proc_user_range_ok(const struct fnd_proc *proc, uint32_t addr, uint32_t len, uint32_t perm) {
    if (len == 0)
        return 1;
    if (addr + len < addr || addr + len > 0xFFFF0000u)
        return 0;
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

static void zero_range(uint32_t start, uint32_t end) {
    for (uint32_t a = start; a < end; a += 4u)
        *(volatile uint32_t *)(uintptr_t)a = 0;
}

/* §9.4 for phase 1: threads DEAD, owned ranges zeroed, slot released. The
 * record stays readable (state EXITED) until the slot is reused. */
void proc_exit(struct fnd_proc *proc, int32_t code, uint32_t fault_epc) {
    if (proc->state != FND_PROC_LIVE)
        return;
    sched_kill_proc(proc);
    proc->threads = 0;
    proc->exit_code = code;
    proc->fault_epc = fault_epc;
    proc->state = FND_PROC_EXITED;
    if (active == proc) {
        for (uint32_t i = 0; i < FND_REGIONS; ++i)
            region_write(i, 2, 0);
        active = 0;
    }
    for (uint32_t i = 0; i < proc->region_count; ++i) {
        if (proc->regions[i].owned)
            zero_range(proc->regions[i].start, proc->regions[i].end);
    }
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
        if (procs[i].state == FND_PROC_LIVE && procs[i].slot == slot) {
            *error = FND_E_ADDRESS_CONFLICT;
            return 0;
        }
        if (!proc && procs[i].state == FND_PROC_FREE)
            proc = &procs[i];
    }
    for (uint32_t i = 0; !proc && i < FND_MAX_PROCS; ++i) {
        if (procs[i].state == FND_PROC_EXITED)
            proc = &procs[i]; /* reuse the oldest kind of record: no handles yet */
    }
    *error = FND_E_LIMIT;
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
    if (slot >= FND_SLOTS || priority > FND_PRIO_APP)
        return FND_E_BAD_ARG;
    if (ram_size < 64u || (ram_size & 7u) || ram_size > slot_bounds[slot + 1u] - slot_bounds[slot])
        return FND_E_BAD_ARG;
    if (code_start >= code_end || entry < code_start || entry >= code_end || (entry & 3u))
        return FND_E_BAD_ARG;
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

    zero_range(ram, end);
    /* Region 0: stack and data at the top of the slot (RW). Region 1: the
     * code (RX). */
    proc->regions[0] = (struct fnd_region){ram, end, CPT32_PERM_R | CPT32_PERM_W, 1, 0};
    proc->regions[1] = (struct fnd_region){code_start, code_end, CPT32_PERM_R | CPT32_PERM_X, 0, 0};
    proc->region_count = 2;
    uint32_t sp = end - 8u;
    *(volatile uint32_t *)(uintptr_t)sp = arg;
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
    }
    return FND_OK;
}

/* One launch at a time: the kernel runs with interrupts off. */
static struct wax_launch launch;

int32_t fnd_proc_spawn_wax(const char *name, uint32_t priority, const void *image, uint32_t size,
                           const struct fnd_image *libs, uint32_t lib_count,
                           const struct fnd_grant *grants, uint32_t grant_count) {
    struct wax_library table[FND_MAX_LIBRARIES];
    uint32_t base, slot = 0;
    if (priority > FND_PRIO_APP || lib_count > FND_MAX_LIBRARIES || (lib_count && !libs))
        return FND_E_BAD_ARG;
    int32_t status = check_grants(grants, grant_count);
    if (status != FND_OK)
        return status;
    enum wax_error error = wax_base_addr(image, size, &base);
    if (error != WAX_OK)
        return wax_status(error);
    while (slot < FND_SLOTS && !(slot_bounds[slot] <= base && base < slot_bounds[slot + 1u]))
        ++slot;
    if (slot == FND_SLOTS)
        return FND_E_FORMAT; /* §5.2: the image must lie in one user slot */
    struct fnd_proc *proc = claim_record(slot, &status);
    if (!proc)
        return status;
    for (uint32_t i = 0; i < lib_count; ++i) {
        table[i].data = libs[i].data;
        table[i].size = libs[i].size;
    }
    struct wax_target target = {"foundation", FND_ABI_MAJOR, FND_ABI_MINOR, slot_bounds[slot],
                                slot_bounds[slot + 1u]};
    if ((error = wax_plan(&launch, image, size, table, lib_count, &target)) != WAX_OK)
        return wax_status(error);
    if (launch.region_count + grant_count > FND_REGIONS)
        return FND_E_REGION_LIMIT; /* §5.3: the grants come after the image */
    struct fnd_thread *thread = sched_alloc_thread();
    if (!thread)
        return FND_E_LIMIT;

    /* Nothing has changed until here, so every failure above leaves no trace. */
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
    return start_proc(proc, thread, name, slot, priority, launch.entry, launch.stack_top, base);
}

/* sys_info (§9): sel 0 is fnd_sysinfo, sel 1 the process list, sel 2 CPU
 * accounting. Copies up to cap bytes and returns the whole answer's size. */
#define SYSINFO_BYTES 32u
#define PROCINFO_BYTES 48u
#define RAM_BYTES 0x01000000u /* §5.1: v1 assumes the default 16 MiB */

static uint32_t proc_mem_bytes(const struct fnd_proc *proc) {
    uint32_t bytes = 0;
    for (uint32_t i = 0; i < proc->region_count; ++i) {
        if (proc->regions[i].owned)
            bytes += proc->regions[i].end - proc->regions[i].start;
    }
    return bytes;
}

static void put_word(uint32_t buf, uint32_t cap, uint32_t offset, uint32_t value) {
    if (offset + 4u <= cap)
        *(volatile uint32_t *)(uintptr_t)(buf + offset) = value;
}

int32_t proc_sys_info(const struct fnd_proc *caller, uint32_t sel, uint32_t buf, uint32_t cap) {
    uint32_t live = 0, used = 0, records = 0;
    for (uint32_t i = 0; i < FND_MAX_PROCS; ++i) {
        if (procs[i].state == FND_PROC_LIVE) {
            ++live;
            used += slot_bounds[procs[i].slot + 1u] - slot_bounds[procs[i].slot];
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
            put_word(buf, cap, 4u * w, info[w]);
        uint32_t offset = 16u;
        for (uint32_t i = 0; i < FND_MAX_PROCS; ++i) {
            if (procs[i].state == FND_PROC_FREE)
                continue;
            put_word(buf, cap, offset, procs[i].pid);
            put_word(buf, cap, offset + 4u, procs[i].cpu_cycles);
            offset += 8u;
        }
        return (int32_t)size;
    }
    if (sel == 0) {
        uint32_t user = slot_bounds[FND_SLOTS] - slot_bounds[0];
        uint32_t info[8] = {FND_ABI_MAJOR, FND_ABI_MINOR, FND_TICK_HZ, RAM_BYTES, user - used, live,
                            boot_drive, 0};
        for (uint32_t w = 0; w < 8u; ++w)
            put_word(buf, cap, 4u * w, info[w]);
        return (int32_t)size;
    }
    uint32_t offset = 0;
    for (uint32_t i = 0; i < FND_MAX_PROCS; ++i) {
        const struct fnd_proc *p = &procs[i];
        if (p->state == FND_PROC_FREE)
            continue;
        uint32_t state = p->state == FND_PROC_EXITED ? 2u : sched_proc_runnable(p) ? 0u : 1u;
        uint32_t info[8] = {p->pid, 0 /* parent: no proc_spawn yet */, state, p->priority,
                            (uint32_t)p->exit_code, p->state == FND_PROC_LIVE ? proc_mem_bytes(p) : 0,
                            p->threads, p->fault_epc};
        for (uint32_t w = 0; w < 8u; ++w)
            put_word(buf, cap, offset + 4u * w, info[w]);
        for (uint32_t w = 0; w < 4u; ++w) {
            uint32_t word = 0;
            for (uint32_t b = 0; b < 4u; ++b)
                word = word << 8 | (uint8_t)p->name[4u * w + b];
            put_word(buf, cap, offset + 32u + 4u * w, word);
        }
        offset += PROCINFO_BYTES;
    }
    return (int32_t)size;
}
