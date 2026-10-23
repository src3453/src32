#ifndef FOUNDATION_CHIP8_OPTIONS_H
#define FOUNDATION_CHIP8_OPTIONS_H
#include <stdint.h>
#define CHIP8_CYCLES_MAX 10000u
#define CHIP8_USAGE "chip8 [-c N|--cycles N] ROM.ch8 (N=1..10000, Esc to quit)"
typedef struct { const char *rom; uint32_t cycles; } chip8_options;
static int chip8_option_equal(const char *a, const char *b) {
    while (*a && *a == *b) { ++a; ++b; }
    return *a == *b;
}

/* Zero cycles selects the legacy 700 instructions/second rate. Parse before
 * opening ROM/devices so malformed options cannot take a device lease. */
static int chip8_parse_options(uint32_t argc, const char *const *argv, chip8_options *out) {
    chip8_options got = {0, 0};
    uint32_t options = 1;
    for (uint32_t i = 1; i < argc; ++i) {
        const char *s = argv[i];
        if (options && chip8_option_equal(s, "--")) { options = 0; continue; }
        if (options && (chip8_option_equal(s, "-c") || chip8_option_equal(s, "--cycles"))) {
            if (got.cycles || ++i == argc) return 0;
            s = argv[i];
            uint32_t n = 0;
            if (!*s) return 0;
            for (; *s; ++s) {
                if (*s < '0' || *s > '9') return 0;
                uint32_t digit = (uint32_t)(*s - '0');
                if (n > (CHIP8_CYCLES_MAX - digit) / 10u) return 0;
                n = n * 10u + digit;
            }
            if (!n) return 0;
            got.cycles = n;
        } else {
            if ((options && *s == '-') || got.rom || !*s) return 0;
            got.rom = s;
        }
    }
    if (!got.rom) return 0;
    *out = got;
    return 1;
}

/* The scheduler clock is >=60 Hz. In -c mode one 60 Hz boundary supplies
 * exactly N instructions; otherwise retain the legacy fractional 700 Hz
 * budget. Bound CPU catch-up to 250 ms; timers use uncapped elapsed time.
 * With hz <= 1,000,000 all intermediate products fit u32. */
static uint32_t chip8_cpu_budget(uint32_t elapsed, uint32_t hz, uint32_t cycles, uint32_t *phase) {
    if (elapsed > hz / 4u) elapsed = hz / 4u;
    uint32_t total = *phase + elapsed * (cycles ? 60u : 700u);
    *phase = total % hz;
    return total / hz * (cycles ? cycles : 1u);
}
#endif
