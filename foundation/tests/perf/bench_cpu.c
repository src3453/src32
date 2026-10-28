/* Run as a normal Foundation WAX process on the counter-equipped FPGA SoC. */
#include <foundation/sys.h>

#define ROUNDS 200000u
#define COUNTERS 8u
#define WORDS 256u

static volatile uint32_t memory[WORDS];

static void warm_alu(void) {
    volatile uint32_t value = 1u;
    for (uint32_t i = 0; i < ROUNDS; ++i)
        value = value * 1664525u + 1013904223u;
    (void)value;
}

static void run_alu(void) {
    volatile uint32_t value = 1u;
    for (uint32_t i = 0; i < ROUNDS; ++i)
        value = value * 1664525u + 1013904223u;
    if (value == 0u)
        fnd_sys_debug_write("unexpected ALU result\n", 22u);
}

static void warm_load(void) {
    for (uint32_t i = 0; i < WORDS; ++i)
        memory[i] = i * 17u + 1u;
    volatile uint32_t sum = 0;
    for (uint32_t i = 0; i < ROUNDS; ++i)
        sum += memory[i & (WORDS - 1u)];
    (void)sum;
}

static void run_load(void) {
    volatile uint32_t sum = 0;
    for (uint32_t i = 0; i < ROUNDS; ++i)
        sum += memory[i & (WORDS - 1u)];
    if (sum == 0u)
        fnd_sys_debug_write("unexpected load result\n", 24u);
}

static void warm_store(void) {
    for (uint32_t i = 0; i < WORDS; ++i)
        memory[i] = i;
    for (uint32_t i = 0; i < ROUNDS; ++i) {
        uint32_t index = i & (WORDS - 1u);
        memory[index] = memory[index] + i + 1u;
    }
}

static void run_store(void) {
    for (uint32_t i = 0; i < ROUNDS; ++i) {
        uint32_t index = i & (WORDS - 1u);
        memory[index] = memory[index] + i + 1u;
    }
}

static void read_counters(uint32_t out[COUNTERS * 2u]) {
    int32_t result = fnd_sys_cpu_perf(out);
    if (result != (int32_t)FND_SYSINFO_CPU_PERF_BYTES) {
        static const char message[] = "CPU performance counters unavailable\n";
        fnd_sys_debug_write(message, sizeof(message) - 1u);
        fnd_sys_proc_exit(1);
        for (;;) {}
    }
}

static void put_text(char *out, uint32_t *length, const char *text) {
    while (*text)
        out[(*length)++] = *text++;
}

static void put_hex64(char *out, uint32_t *length, uint32_t low, uint32_t high) {
    static const char hex[] = "0123456789ABCDEF";
    for (int32_t shift = 28; shift >= 0; shift -= 4)
        out[(*length)++] = hex[(high >> shift) & 15u];
    for (int32_t shift = 28; shift >= 0; shift -= 4)
        out[(*length)++] = hex[(low >> shift) & 15u];
}

static void report(const char *name, const uint32_t before[COUNTERS * 2u],
                   const uint32_t after[COUNTERS * 2u]) {
    static const char *const names[COUNTERS] = {
        " CYCLE=", " INSTRET=", " IC_MISS=", " DC_MISS=",
        " MEM_STALL=", " DATA_STALL=", " BR_STALL=", " BUS_STALL="
    };
    char line[256];
    uint32_t length = 0;
    put_text(line, &length, "CPU_PERF ");
    put_text(line, &length, name);
    uint32_t cycles = 0, instret = 0;
    for (uint32_t i = 0; i < COUNTERS; ++i) {
        uint32_t lo = after[i * 2u] - before[i * 2u];
        uint32_t borrow = after[i * 2u] < before[i * 2u];
        uint32_t hi = after[i * 2u + 1u] - before[i * 2u + 1u] - borrow;
        if (i == 0u) cycles = lo;
        if (i == 1u) instret = lo;
        put_text(line, &length, names[i]);
        put_hex64(line, &length, lo, hi);
    }
    if (cycles != 0u) {
        uint32_t ipc_milli = (uint32_t)(((uint64_t)instret * 1000u) / cycles);
        put_text(line, &length, " IPC_MILLI=");
        line[length++] = (char)('0' + (ipc_milli / 1000u));
        line[length++] = '.';
        line[length++] = (char)('0' + ((ipc_milli / 100u) % 10u));
        line[length++] = (char)('0' + ((ipc_milli / 10u) % 10u));
        line[length++] = (char)('0' + (ipc_milli % 10u));
    }
    line[length++] = '\n';
    fnd_sys_debug_write(line, length);
}

static void measure(const char *name, void (*warm)(void), void (*run)(void)) {
    uint32_t before[COUNTERS * 2u], after[COUNTERS * 2u];
    warm();
    read_counters(before);
    run();
    read_counters(after);
    report(name, before, after);
}

int main(void) {
    warm_alu();
    measure("ALU", warm_alu, run_alu);
    measure("LOAD", warm_load, run_load);
    measure("STORE", warm_store, run_store);
    return 0;
}
