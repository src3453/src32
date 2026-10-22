/* test.base: a WAL built from C (doc/spec_WAX_v3.md, call ABI 1). Its
 * static data is private to each process that loads it. */
#include <stdint.h>

uint32_t base_calls;              /* exported object */
static const char greeting[] = "hello";

uint32_t base_add(uint32_t a, uint32_t b) {
    ++base_calls;
    return a + b;
}

uint64_t base_add64(uint64_t a, uint64_t b) { return a + b; }

const char *base_greeting(void) { return greeting; }
