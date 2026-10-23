/* test.math: a WAL that imports test.base and uses internal function
 * pointers, a switch table and its own static state. */
#include <stdarg.h>
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

/* Variadic functions and software floating point inside a position-independent
 * library: the runtime (Berkeley SoftFloat) is linked into this WAL, and float
 * operations reach it through internal calls relative to R30. */
static uint32_t sum(uint32_t count, ...) {
    va_list ap;
    uint32_t total = 0;
    va_start(ap, count);
    while (count--)
        total += va_arg(ap, uint32_t);
    va_end(ap);
    return total;
}

uint32_t math_sum3(uint32_t a, uint32_t b, uint32_t c) { return sum(3, a, b, c); }

static float fadd_(float a, float b) { return a + b; }
static float fmul_(float a, float b) { return a * b; }

/* IEEE-754 binary32 bit patterns in and out: op 0 adds, 1 multiplies, then
 * 0.5 is added; the operation is picked through a function pointer. */
uint32_t math_float(uint32_t op, uint32_t a, uint32_t b) {
    union { float f; uint32_t u; } x, y, r;
    float (*f)(float, float) = op ? fmul_ : fadd_;
    x.u = a;
    y.u = b;
    r.f = f(x.f, y.f) + 0.5f;
    return r.u;
}
