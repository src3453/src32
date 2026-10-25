#ifndef FOUNDATION_KERNEL_H
#define FOUNDATION_KERNEL_H
#include <stdint.h>

#include <cpt32/cpu.h>
#include <foundation/abi.h>
#include <foundation/ipc.h>

/* Supervisor-side interface of the kernel, for the image that links
 * it (boot code, demo, tests). User programs use <foundation/abi.h>. */

#define FND_MAX_PROCS 7u    /* process records; no longer tied to memory slots */
#define FND_MAX_THREADS 8u  /* user threads; the idle thread is extra */
#define FND_REGIONS 8u      /* Extension P regions: the nommu backend's limit */
/* The regions a process record holds: an image's (at most 8) and the MMIO
 * grants (at most FND_MAX_GRANTS). The nommu backend keeps all in the 8 CPU
 * regions; the vm backend only records them. */
#define FND_PROC_REGIONS 16u
#define FND_TICK_HZ 100u
#define FND_TICK_CYCLES (48000000u / FND_TICK_HZ) /* PeC timer ch0 RELOAD at 48 MHz */
#define FND_SLICE_TICKS 1u /* 10 ms, one scheduler tick */

/* The fixed ranges of the old slot layout. Only processes that run code
 * linked into the kernel image (fnd_proc_create_builtin) and tests that link
 * executables to fixed addresses still use them; the nommu backend has no
 * slots (§5.2) and takes any free range. */
#define FND_SLOTS 7u
uint32_t fnd_slot_base(uint32_t slot);
uint32_t fnd_slot_end(uint32_t slot);

enum fnd_proc_state { FND_PROC_FREE, FND_PROC_LIVE, FND_PROC_EXITED };

struct fnd_region {
    uint32_t start, end, flags;
    uint32_t owned; /* zeroed and released when the process ends */
    uint32_t mmio;  /* an MMIO grant (§10.3): never a RAM region for §5.4 */
};

/* A handle table entry (§6): kind FND_OBJ_* and the kernel object. */
enum { FND_OBJ_NONE, FND_OBJ_PORT, FND_OBJ_CHANNEL, FND_OBJ_PROCESS, FND_OBJ_THREAD };
struct fnd_handle_entry {
    uint32_t kind;
    void *object;
};

struct fnd_proc {
    uint32_t state;
    uint32_t pid;
    uint32_t parent; /* pid of the spawner, 0 for the kernel */
    uint32_t arena_lo, arena_hi; /* nommu backend: the pages it owns (§5.2), else 0 */
    uint32_t priority;
    uint32_t caps;    /* FND_CAP_* (§9.1) */
    uint32_t threads; /* live threads */
    uint32_t refs;    /* PROCESS handles: the record stays until they close */
    int32_t exit_code;
    uint32_t fault_epc; /* EPC of the fault that ended it, else 0 */
    uint32_t cpu_cycles; /* elapsed user-context cycles, modulo 2^32 */
    uint32_t region_count;
    struct fnd_region regions[FND_PROC_REGIONS];
    uint32_t pt_root;  /* vm backend: physical address of the level-1 table, else 0 */
    uint32_t vm_pages; /* vm backend: frames of memory the process holds */
    uint32_t heap_start, heap_end, heap_limit, heap_region; /* mem_grow (§5.3) */
    uint32_t startup_len; /* bytes of its startup block (§9.3) */
    struct fnd_handle_entry handles[FND_MAX_HANDLES];
    char name[16];
};

/* A process that runs user code linked into the kernel image. [code_start, code_end) is granted R/X, and the top ram_size
 * bytes of the old slot range ([fnd_slot_end(slot) - ram_size, fnd_slot_end(slot)),
 * zeroed, claimed as the arena) R/W for stack and data. The thread starts at entry in U mode with
 * USP = slot end - 8, where the argument word arg is stored (C stack ABI:
 * first argument at SP), UFP = 0, UGP = slot base. Returns the pid (>= 1) or
 * a negative FND_E_* code. */
int32_t fnd_proc_create_builtin(const char *name, uint32_t slot, uint32_t priority,
                                uint32_t ram_size, uint32_t code_start, uint32_t code_end,
                                uint32_t entry, uint32_t arg);
/* A WAX/WAL image in kernel memory. */
struct fnd_image {
    const void *data;
    uint32_t size;
};
#define FND_MAX_LIBRARIES 16u

/* An MMIO grant of fnd_spawn (§9.2, §10.3): [start, end) with perm R (1)
 * or RW (3). */
struct fnd_grant {
    uint32_t start, end, perm;
};
#define FND_MAX_GRANTS 8u

/* Loads a PROTECTED WAX into free RAM (the pages it reserves, or anywhere
 * for a relocatable one) and starts it as §9.2 describes: v2, or v3 with the WAL graph it needs, chosen
 * from libs (doc/spec_WAX_v3.md §4.1, §7). The image and the libraries must
 * stay unchanged during the call; the kernel does not copy them first.
 * grants become the last regions (§5.3 step 4); a grant outside the §10.3
 * windows is FND_E_ACCESS, and VRAM or PCMRAM already granted to a live
 * process is FND_E_BUSY. Atomic: on failure nothing is allocated or written.
 * Returns the pid (>= 1) or FND_E_FORMAT, FND_E_UNSUPPORTED,
 * FND_E_DEPENDENCY, FND_E_REGION_LIMIT, FND_E_ADDRESS_CONFLICT (its fixed
 * range is taken), FND_E_NO_MEMORY, FND_E_LIMIT, FND_E_ACCESS, FND_E_BUSY or FND_E_BAD_ARG. */
int32_t fnd_proc_spawn_wax(const char *name, uint32_t priority, const void *image, uint32_t size,
                           const struct fnd_image *libs, uint32_t lib_count,
                           const struct fnd_grant *grants, uint32_t grant_count);
/* A bundled module started by the kernel (§11): like fnd_proc_spawn_wax,
 * with the capabilities caps. The startup block is argv = [name], cwd = "/"
 * and no roles. */
int32_t fnd_proc_spawn_module(const char *name, uint32_t priority, uint32_t caps, const void *image,
                              uint32_t size, const struct fnd_image *libs, uint32_t lib_count,
                              const struct fnd_grant *grants, uint32_t grant_count);
/* FNDBOOT bundled modules (§11). fnd_boot_check validates the header and
 * the module table of the image stage1 staged (or one linked into the
 * kernel image) and returns the module count or FND_E_FORMAT. Call
 * fnd_boot_spawn only after a successful check: it spawns module index with
 * its priority, caps and grants, with libs as its WAL list (§9.5), and returns
 * what fnd_proc_spawn_module returns. The image must stay in memory while the
 * modules may be spawned. */
int32_t fnd_boot_check(const void *fndboot);
int32_t fnd_boot_spawn(const void *fndboot, uint32_t index, const struct fnd_image *libs,
                       uint32_t lib_count);
/* The boot drive that stage1 passes in R1, reported by sys_info (§9.4). */
void fnd_set_boot_drive(uint32_t drive);

/* NULL for an unknown pid. Exited processes keep their record. */
struct fnd_proc *fnd_proc_get(uint32_t pid);

/* Called by the kernel on every tick with interrupts off. Return 0 to keep
 * running, or a nonzero code to stop the machine: the kernel then HALTs with
 * that code in R1 and interrupts off (used by tests). */
typedef uint32_t (*fnd_tick_hook)(uint32_t ticks);
void fnd_set_tick_hook(fnd_tick_hook hook);

/* Starts the tick timer and runs the scheduler; never returns. */
CPT32_NORETURN void fnd_kernel_run(void);

uint32_t fnd_ticks(void);
uint32_t fnd_context_switches(void);

/* Kernel debug console (UART). */
void fnd_debug_putc(uint32_t c);
void fnd_debug_puts(const char *s);
void fnd_debug_hex(uint32_t value);

#endif
