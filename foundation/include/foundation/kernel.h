#ifndef FOUNDATION_KERNEL_H
#define FOUNDATION_KERNEL_H
#include <stdint.h>

#include <cpt32/cpu.h>
#include <foundation/abi.h>

/* Supervisor-side interface of the kernel, for the image that links
 * it (boot code, demo, tests). User programs use <foundation/abi.h>. */

#define FND_MAX_PROCS 7u    /* one per user slot */
#define FND_MAX_THREADS 8u  /* user threads; the idle thread is extra */
#define FND_REGIONS 8u
#define FND_TICK_HZ 1000u
#define FND_TICK_CYCLES 48000u /* PeC timer ch0 RELOAD at 48 MHz */
#define FND_SLICE_TICKS 10u

/* User slots (§5.2). */
#define FND_SLOTS 7u
uint32_t fnd_slot_base(uint32_t slot);
uint32_t fnd_slot_end(uint32_t slot);

enum fnd_proc_state { FND_PROC_FREE, FND_PROC_LIVE, FND_PROC_EXITED };

struct fnd_region {
    uint32_t start, end, flags;
    uint32_t owned; /* zeroed and released when the process ends */
};

struct fnd_proc {
    uint32_t state;
    uint32_t pid;
    uint32_t slot;
    uint32_t priority;
    uint32_t threads; /* live threads */
    int32_t exit_code;
    uint32_t fault_epc; /* EPC of the fault that ended it, else 0 */
    uint32_t region_count;
    struct fnd_region regions[FND_REGIONS];
    char name[16];
};

/* A process that runs user code linked into the kernel image. [code_start, code_end) is granted R/X, and the top ram_size
 * bytes of the slot ([fnd_slot_end(slot) - ram_size, fnd_slot_end(slot)),
 * zeroed) R/W for stack and data. The thread starts at entry in U mode with
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

/* Loads a PROTECTED WAX into the user slot that contains its base_addr and
 * starts it as §9.2 describes: v2, or v3 with the WAL graph it needs, chosen
 * from libs (doc/spec_WAX_v3.md §4.1, §7). The image and the libraries must
 * stay unchanged during the call; the kernel does not copy them first.
 * Atomic: on failure nothing is allocated or written. Returns the pid (>= 1)
 * or FND_E_FORMAT, FND_E_UNSUPPORTED, FND_E_DEPENDENCY, FND_E_REGION_LIMIT,
 * FND_E_ADDRESS_CONFLICT (slot in use), FND_E_LIMIT or FND_E_BAD_ARG. */
int32_t fnd_proc_spawn_wax(const char *name, uint32_t priority, const void *image, uint32_t size,
                           const struct fnd_image *libs, uint32_t lib_count);
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
