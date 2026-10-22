/* test.math: a WAL that imports test.base and uses internal function
 * pointers, a switch table and its own static state. */
#include <stdint.h>

uint32_t base_add(uint32_t a, uint32_t b);
extern uint32_t base_calls;

static uint32_t total;

static uint32_t twice(uint32_t x) { return base_add(x, x); }
static uint32_t square(uint32_t x) { return x * x; }

uint32_t math_apply(uint32_t op, uint32_t x) {
    uint32_t (*f)(uint32_t) = op ? square : twice;
    total += f(x);
    return f(x);
}

uint32_t math_total(void) { return total; }

/* The importer's view of test.base's object. */
uint32_t math_base_calls(void) { return base_calls; }

/* An address inside this instance of the library (R30-relative). */
uint32_t math_origin(void) { return (uint32_t)(uintptr_t)&math_origin; }
