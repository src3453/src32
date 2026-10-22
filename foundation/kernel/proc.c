/* Processes, user slots and protection regions (doc/spec_Foundation.md §5,
 * §9.2, §9.4). Processes come from code linked into the kernel image
 * (phase 1) or from a PROTECTED WAX loaded by wax.c. */
#include "internal.h"
#include "wax.h"

static struct fnd_proc procs[FND_MAX_PROCS];
static struct fnd_proc *active; /* process whose regions are programmed */
static uint32_t next_pid = 1;

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
            if ((r->flags & perm) == perm && r->start <= addr && addr < r->end) {
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
    proc->regions[0] = (struct fnd_region){ram, end, CPT32_PERM_R | CPT32_PERM_W, 1};
    proc->regions[1] = (struct fnd_region){code_start, code_end, CPT32_PERM_R | CPT32_PERM_X, 0};
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

/* One launch at a time: the kernel runs with interrupts off. */
static struct wax_launch launch;

int32_t fnd_proc_spawn_wax(const char *name, uint32_t priority, const void *image, uint32_t size,
                           const struct fnd_image *libs, uint32_t lib_count) {
    struct wax_library table[FND_MAX_LIBRARIES];
    uint32_t base, slot = 0;
    if (priority > FND_PRIO_APP || lib_count > FND_MAX_LIBRARIES || (lib_count && !libs))
        return FND_E_BAD_ARG;
    enum wax_error error = wax_base_addr(image, size, &base);
    if (error != WAX_OK)
        return wax_status(error);
    while (slot < FND_SLOTS && !(slot_bounds[slot] <= base && base < slot_bounds[slot + 1u]))
        ++slot;
    if (slot == FND_SLOTS)
        return FND_E_FORMAT; /* §5.2: the image must lie in one user slot */
    int32_t status;
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
    struct fnd_thread *thread = sched_alloc_thread();
    if (!thread)
        return FND_E_LIMIT;

    /* Nothing has changed until here, so every failure above leaves no trace. */
    wax_load(&launch, 0);
    for (uint32_t i = 0; i < launch.region_count; ++i) {
        proc->regions[i] = (struct fnd_region){launch.region[i].start, launch.region[i].end,
                                               launch.region[i].flags, 1};
    }
    proc->region_count = launch.region_count;
    return start_proc(proc, thread, name, slot, priority, launch.entry, launch.stack_top, base);
}
