#include "project.h"

static int local = 29;
static int subtract(int a, int b) { return a - b; }
static int six(int a, int b, int c, int d, int e, int f) {
    return a + 2*b + 3*c + 4*d + 5*e + 6*f;
}
int main(void) {
    int result = memory_checks();
    if (result) return 100 + result;
    result = integer_checks();
    if (result) return 200 + result;
    if (recurse((int)seed + 1) != 21) return 301;
    if (control(10) != 20) return 302;
    if (global_operation(19, 23) != 42) return 303;
    if (callback_checks(subtract) != 18) return 304;
    if (other_static() != 13 || local != 29) return 305;
    if (six(1, 2, 3, 4, 5, (int)seed) != 97) return 306;
    struct Record changed = transform(records[0]);
    if (changed.tag != 5 || changed.value != 36 || changed.step != -5)
        return 307;
    if (records[0].tag != 1 || records[0].value != 19 || records[0].step != -2)
        return 308;
    result = wide_checks();
    if (result) return 400 + result;
    if (variable_stack(seed) != 68) return 309;
    return 42;
}
