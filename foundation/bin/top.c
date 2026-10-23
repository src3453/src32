/* top [-n count] (doc/spec_Foundation_shell.md §10.3): CPU use from the
 * difference of two sys_info sel 2 snapshots a second apart, memory from
 * sel 0 and the processes from sel 1, busiest first. Without -n it redraws
 * until q or Ctrl+C. */
#include "cmd.h"

#define PS_MAX 16u
static uint32_t ps_buf[PS_MAX * FND_PROCINFO_BYTES / 4u];

/* CPU snapshots: four header words followed by (pid, cycles) pairs.
 * Subtraction is unsigned so clock/counter wrap is harmless within a refresh. */
#define CPU_WORDS (4u + PS_MAX * 2u)
static uint32_t top_before[CPU_WORDS], top_after[CPU_WORDS];

static char upper(uint32_t c) { return (char)(c >= 'a' && c <= 'z' ? c - 32u : c); }

static int parse_count(const char *s, int32_t *out) {
    int32_t v = 0;
    if (!*s)
        return 0;
    for (; *s; ++s) {
        if (*s < '0' || *s > '9' || v > 214748363)
            return 0;
        v = v * 10 + (*s - '0');
    }
    *out = v;
    return 1;
}

static void show_process(const uint32_t *p) {
    static const char *const states[3] = {"RUN  ", "BLOCK", "EXIT "};
    char name[17];
    for (uint32_t i = 0; i < 16u; ++i)
        name[i] = (char)(p[8u + i / 4u] >> (24u - 8u * (i % 4u)));
    name[16] = 0;
    cmd_out_dec((int32_t)p[0], 5);
    cmd_out_dec((int32_t)p[1], 7);
    cmd_out(" ");
    cmd_out(p[2] < 3u ? states[p[2]] : "?    ");
    cmd_out_dec((int32_t)p[3], 5);
    cmd_out_dec((int32_t)(p[5] / 1024u), 7);
    cmd_out("K ");
    cmd_out(name);
    cmd_out("\n");
}

static int32_t read_cpu(uint32_t *buf) {
    int32_t size = fnd_sys_info(FND_INFO_CPU, buf, CPU_WORDS * 4u);
    if (size < 0)
        return size;
    if (size < (int32_t)FND_CPUINFO_HEADER_BYTES || size > (int32_t)(CPU_WORDS * 4u) ||
        ((uint32_t)size - FND_CPUINFO_HEADER_BYTES) % FND_CPUINFO_RECORD_BYTES)
        return FND_E_FORMAT;
    return size / 4;
}

static uint32_t cpu_delta(uint32_t pid, uint32_t before_words, uint32_t after_words) {
    uint32_t previous = 0;
    for (uint32_t i = 4; i + 1u < before_words; i += 2u) {
        if (top_before[i] == pid)
            previous = top_before[i + 1u];
    }
    for (uint32_t i = 4; i + 1u < after_words; i += 2u) {
        if (top_after[i] == pid)
            return top_after[i + 1u] - previous;
    }
    return 0;
}

/* One decimal place, without overflowing a 32-bit cycles * 1000 product. */
static uint32_t cpu_percent(uint32_t cycles, uint32_t total) {
    uint32_t unit = total / 1000u;
    uint32_t value = unit ? cycles / unit : 0;
    return value > 1000u ? 1000u : value;
}

static void out_percent(uint32_t value) {
    cmd_out_dec((int32_t)(value / 10u), 3);
    cmd_out(".");
    cmd_out_dec((int32_t)(value % 10u), 0);
    cmd_out("%");
}

/* Waits one second; 1 if q or Ctrl+C was pressed meanwhile. */
static int32_t wait_or_quit(uint32_t refresh) {
    uint32_t begin = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO);
    while ((uint32_t)fnd_sys_time_get(FND_TIME_TICK_LO) - begin < refresh) {
        fnd_key_t key;
        int32_t status = fnd_con_pollkey(&key);
        if (status < 0)
            return status;
        if (status == FND_OK && key.code == FND_KEY_CHAR &&
            (upper(key.ch) == 'Q' || ((key.mods & FND_MOD_CTRL) && upper(key.ch) == 'C')))
            return 1;
        fnd_sys_sleep(1u); /* poll keys every 10 ms at 100 Hz */
    }
    return 0;
}

static int32_t top(uint32_t argc) {
    int32_t count = 0; /* zero means interactive */
    if (argc != 1u && (argc != 3u || cmd_len(fnd_argv(1)) != 2u || fnd_argv(1)[0] != '-' ||
                       upper((uint8_t)fnd_argv(1)[1]) != 'N' || !parse_count(fnd_argv(2), &count) || count <= 0))
        return cmd_usage("top [-n count]");
    int32_t before_words = read_cpu(top_before);
    if (before_words < 0)
        return cmd_report("top", 0, before_words);
    uint32_t refresh = (uint32_t)fnd_sys_time_get(FND_TIME_TICK_HZ);
    if (!refresh || refresh > 1000000u)
        return cmd_report("top", 0, FND_E_BAD_ARG);
    for (;;) {
        int32_t quit = wait_or_quit(refresh);
        if (quit < 0)
            return cmd_report("top", 0, quit);
        if (quit)
            return FND_OK;
        int32_t size = fnd_sys_info(FND_INFO_PROCESSES, ps_buf, sizeof ps_buf);
        if (size < 0 || (uint32_t)size > sizeof ps_buf)
            return cmd_report("top", 0, size < 0 ? size : FND_E_LIMIT);
        int32_t after_words = read_cpu(top_after);
        if (after_words < 0)
            return cmd_report("top", 0, after_words);
        uint32_t total = top_after[0] - top_before[0];
        uint32_t kernel = top_after[1] - top_before[1];
        uint32_t idle = top_after[2] - top_before[2];
        uint32_t user = total - kernel - idle;
        if (!count)
            fnd_con_clear();
        cmd_out("top - CPU (q/Ctrl+C to quit)\nuser ");
        out_percent(cpu_percent(user, total));
        cmd_out("  kernel~ ");
        out_percent(cpu_percent(kernel, total));
        cmd_out("  idle ");
        out_percent(cpu_percent(idle, total));
        cmd_out("\n");
        uint32_t info[8];
        int32_t status = fnd_sys_info(FND_INFO_SYSTEM, info, FND_SYSINFO_BYTES);
        if (status < 0)
            return cmd_report("top", 0, status);
        cmd_out("Mem: ");
        cmd_out_dec((int32_t)(info[3] / 1024u), 0);
        cmd_out("K total, ");
        cmd_out_dec((int32_t)(info[4] / 1024u), 0);
        cmd_out("K free\n");
        cmd_out("  CPU%   PID PARENT STATE PRIO     MEM NAME\n");
        uint32_t rows = (uint32_t)size / FND_PROCINFO_BYTES;
        uint32_t order[PS_MAX], cycles[PS_MAX];
        for (uint32_t i = 0; i < rows; ++i) {
            order[i] = i;
            cycles[i] = cpu_delta(ps_buf[i * FND_PROCINFO_BYTES / 4u], (uint32_t)before_words,
                                  (uint32_t)after_words);
        }
        for (uint32_t i = 0; i < rows; ++i) {
            for (uint32_t j = i + 1u; j < rows; ++j) {
                if (cycles[order[j]] > cycles[order[i]]) {
                    uint32_t tmp = order[i];
                    order[i] = order[j];
                    order[j] = tmp;
                }
            }
            out_percent(cpu_percent(cycles[order[i]], total));
            cmd_out(" ");
            show_process(ps_buf + order[i] * FND_PROCINFO_BYTES / 4u);
        }
        if (count && --count == 0)
            return FND_OK;
        for (uint32_t i = 0; i < (uint32_t)after_words; ++i)
            top_before[i] = top_after[i];
        before_words = after_words;
    }
}

CMD_MAIN(top)
