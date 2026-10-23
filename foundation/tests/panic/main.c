/* Kernel panic screen test: the first timer tick makes the kernel itself
 * touch an unmapped address. The bus error reaches kernel_panic, which must
 * draw the diagnosis on the PCG without any service. src32_testbench has no
 * UART, so the report to it faults again and must stop quietly. */
#include <foundation/kernel.h>

static uint32_t on_tick(uint32_t ticks) {
    if (ticks == 3u)
        return *(volatile uint32_t *)(uintptr_t)0x7F000000u;
    return 0;
}

int main(void) {
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
