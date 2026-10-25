/* Benchmark fixture only: the normal kernel sources and syscall ABI are used. */
#include "internal.h"
#include "ipc_bench.h"

static struct fnd_proc *client, *server;
static uint32_t client_box;
static uint32_t column;
static void put(uint32_t c) {
    *(volatile uint8_t *)(uintptr_t)(0x10000000u + column++) = (uint8_t)c;
}
static void number(uint32_t n) {
    char digits[10];
    uint32_t count = 0;
    do { digits[count++] = '0' + n % 10u; n /= 10u; } while (n);
    while (count) put(digits[--count]);
    put(' ');
}
static uint32_t on_tick(uint32_t ticks) {
    if (client->state != FND_PROC_LIVE || server->state != FND_PROC_LIVE) return 2;
    if (mm_read32(client, client_box + 4u)) {
        /* Output only after measurement; raw PCG text is read by --dump-pcg. */
        *(volatile uint8_t *)(uintptr_t)0x8000F003u = 1;
        for (uint32_t c = 0; c < BENCH_CASES; ++c) {
            column = c * 80u;
            put('I'); put('P'); put('C'); put(' ');
            number(c);
            for (uint32_t i = 0; i < 3; ++i)
                number(mm_read32(client, client_box + 8u + (c * 3u + i) * 4u));
        }
        return 42;
    }
    return ticks > 2000 ? 3 : 0;
}
static struct fnd_proc *spawn(uint32_t slot, uint32_t priority, void (*fn)(struct bench_box *)) {
    uint32_t box = fnd_slot_end(slot) - BENCH_RAM;
    int32_t pid = fnd_proc_create_builtin("ipcbench", slot, priority, BENCH_RAM,
        (uint32_t)(uintptr_t)bench_user_begin, (uint32_t)(uintptr_t)bench_user_end,
        (uint32_t)(uintptr_t)fn, box);
    return pid > 0 ? fnd_proc_get(pid) : 0;
}
int main(void) {
    server = spawn(4, BENCH_SERVER_PRIORITY, bench_server);
    client = spawn(5, FND_PRIO_APP, bench_client);
    if (!server || !client) return 2;
#ifdef FND_MM_NOMMU_ONLY
    if (mm_vm) return 4;
#else
    if (!mm_vm) return 4; /* refuse to mislabel a CPU without Extension V */
#endif
    int32_t port = sys_port_create(server);
    int32_t chan = sys_chan_create(server, port, 123);
    int32_t inherited = handle_dup_into(server, chan, client);
    if (port < 0 || chan < 0 || inherited < 0) return 3;
    mm_write32(server, fnd_slot_end(4) - BENCH_RAM, port);
    client_box = fnd_slot_end(5) - BENCH_RAM;
    mm_write32(client, client_box, inherited);
    fnd_set_tick_hook(on_tick);
    fnd_kernel_run();
}
