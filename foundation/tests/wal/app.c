/* test.app: a WAX v3 executable that uses test.math and test.base. main's
 * result is the exit code (the startup code calls proc_exit): 42 when every
 * check passes, otherwise the number of the first failed check. */
#include <stdint.h>

uint32_t base_add(uint32_t a, uint32_t b);
uint64_t base_add64(uint64_t a, uint64_t b);
const char *base_greeting(void);
extern uint32_t base_calls;
uint32_t math_apply(uint32_t op, uint32_t x);
uint32_t math_total(void);
uint32_t math_base_calls(void);
uint32_t math_origin(void);
uint32_t math_sum3(uint32_t a, uint32_t b, uint32_t c);
uint32_t math_float(uint32_t op, uint32_t a, uint32_t b);

static uint32_t local(uint32_t x) { return x + 1u; }

int main(void) {
    uint32_t (*f)(uint32_t) = local;
    if (math_apply(0, 5) != 10 || math_apply(1, 4) != 16)
        return 1;
    if (math_total() != 26)
        return 2;
    /* Two twice() calls went through test.base; this process only. */
    if (base_calls != 2 || math_base_calls() != 2)
        return 3;
    if (base_add(2, 3) != 5 || base_calls != 3)
        return 4;
    if (base_add64(0xFFFFFFFFull, 1) != 0x100000000ull)
        return 5;
    const char *text = base_greeting();
    if (text[0] != 'h' || text[4] != 'o' || text[5] != 0)
        return 6;
    /* test.math is placed after this executable, in the same slot. */
    uint32_t address = math_origin();
    if (address <= (uint32_t)(uintptr_t)&main || address - (uint32_t)(uintptr_t)&main > 0x00400000u)
        return 7;
    if (f(1) != 2)
        return 8;
    /* Variadic call and software floating point inside the library. */
    if (math_sum3(1, 20, 300) != 321)
        return 9;
    if (math_float(0, 0x3FC00000u, 0x40100000u) != 0x40880000u)  /* 1.5 + 2.25 + 0.5 = 4.25 */
        return 10;
    if (math_float(1, 0x40000000u, 0x40400000u) != 0x40D00000u)  /* 2 * 3 + 0.5 = 6.5 */
        return 11;
    return 42;
}
