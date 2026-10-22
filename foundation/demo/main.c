/* Foundation phase 1 demo: three U-mode processes counting without yielding
 * share the CPU through the 10-tick time slice, one of them crashes on a
 * protection fault, and a SERVICE process wakes every 250 ticks. The kernel
 * shows every process on the PCG screen ten times a second. */
#include <cpt32/pcg.h>
#include <foundation/kernel.h>

#include "tasks.h"

struct shown {
    const char *name;
    uint32_t slot;
    uint32_t priority;
    void (*entry)(volatile uint32_t *);
    int32_t pid;
};

static struct shown shown[] = {
    {"count-a", 4u, FND_PRIO_APP, task_count, 0},
    {"count-b", 5u, FND_PRIO_APP, task_count, 0},
    {"crash", 6u, FND_PRIO_APP, task_crash, 0},
    {"ticker", 3u, FND_PRIO_SERVICE, task_ticker, 0},
};
#define SHOWN (sizeof shown / sizeof shown[0])

static volatile uint32_t *box(uint32_t slot) {
    return (volatile uint32_t *)(uintptr_t)(fnd_slot_end(slot) - TASK_RAM);
}

static void put_hex(uint32_t value) {
    char text[9];
    for (int digit = 0; digit < 8; ++digit) {
        uint32_t nibble = (value >> (28 - 4 * digit)) & 0xFu;
        text[digit] = (char)(nibble < 10u ? '0' + nibble : 'A' + nibble - 10u);
    }
    text[8] = 0;
    pcg_prnscr(text);
}

static void label(uint32_t x, uint32_t y, const char *text) {
    pcg_locate(x, y);
    pcg_prnscr(text);
}

static uint32_t show(uint32_t ticks) {
    if (ticks % 100u)
        return 0;
    pcg_colorfg(0x3Fu);
    label(0, 2, "ticks    ");
    put_hex(ticks);
    label(0, 3, "switches ");
    put_hex(fnd_context_switches());
    for (uint32_t i = 0; i < SHOWN; ++i) {
        struct fnd_proc *proc = fnd_proc_get((uint32_t)shown[i].pid);
        uint32_t row = 5u + i;
        pcg_colorfg(proc && proc->state == FND_PROC_LIVE ? 0x3Cu : 0x30u);
        label(0, row, shown[i].name);
        label(9, row, "");
        put_hex(proc && proc->state == FND_PROC_LIVE ? box(shown[i].slot)[0] : 0u);
        if (proc && proc->state == FND_PROC_EXITED) {
            label(18, row, "exit ");
            put_hex((uint32_t)proc->exit_code);
        }
    }
    return 0;
}

int main(void) {
    pcg_initPCG();
    pcg_cls(0);
    pcg_colorfg(0x3Fu);
    pcg_colorbg(0);
    pcg_prnlnscr("Foundation phase 1");
    for (uint32_t i = 0; i < SHOWN; ++i) {
        shown[i].pid = fnd_proc_create_builtin(
            shown[i].name, shown[i].slot, shown[i].priority, TASK_RAM,
            (uint32_t)(uintptr_t)user_code_begin, (uint32_t)(uintptr_t)user_code_end,
            (uint32_t)(uintptr_t)shown[i].entry, (uint32_t)(uintptr_t)box(shown[i].slot));
        if (shown[i].pid < 0)
            return 1;
    }
    fnd_set_tick_hook(show);
    fnd_kernel_run();
}
