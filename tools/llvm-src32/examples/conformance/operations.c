#include "project.h"
#include <string.h>

volatile unsigned seed = 7;
struct Record records[3] = {{1, 19, -2}, {2, -31, 5}, {3, 53, 7}};
int zero_area[7];
const char *message = "SRC32 C\n";
int *selected = &records[1].value;
operation global_operation = add;
static int local = 13;

int add(int a, int b) { return a + b; }
int recurse(int n) {
    int marker = n + 3;
    if (n < 2) return n;
    return recurse(n - 1) + recurse(n - 2) + marker - n - 3;
}
int control(int x) {
    int total = 0;
    for (int i = 0; i < x; ++i) {
        if (i == 2) continue;
        if (i > 8) break;
        switch (i & 3) {
        case 0: total += 3; break;
        case 1: total -= 2; break;
        case 2: total += 5; break;
        default: total += i; break;
        }
    }
    return total;
}
int callback_checks(operation op) { return op(local, -5); }
int other_static(void) { return local; }

int memory_checks(void) {
    struct Record copy[3];
    struct Packed packed = {0xA5, 0x89ABCDEFu};
    unsigned char *bytes = (unsigned char *)&packed;
    char buffer[16];
    for (unsigned i = 0; i < 7; ++i)
        if (zero_area[i] != 0) return 1;
    if (sizeof(struct Record) != 12 || offsetof(struct Record, value) != 4)
        return 2;
    if (sizeof(struct Packed) != 5 || bytes[0] != 0xA5 || bytes[1] != 0x89 ||
        bytes[2] != 0xAB || bytes[3] != 0xCD || bytes[4] != 0xEF) return 3;
    memcpy(copy, records, sizeof(copy));
    copy[1].value += copy[2].step;
    if (copy[1].value != -24 || *selected != -31) return 4;
    memset(buffer, 0, sizeof(buffer));
    memcpy(buffer, message, 8);
    if (strlen(buffer) != 8 || memcmp(buffer, "SRC32 C\n", 8)) return 5;
    memmove(buffer + 2, buffer, 8);
    if (memcmp(buffer, "SRSRC32 C\n", 10)) return 6;
    memmove(buffer, buffer + 2, 8);
    if (memcmp(buffer, message, 8)) return 7;
    unsigned char a[3] = {0xFF, 0, 1}, b[3] = {1, 0, 1};
    if (memcmp(a, b, 3) <= 0) return 8;
    zero_area[6] = copy[0].value;
    if (zero_area[6] != 19) return 9;
    return 0;
}

struct Record transform(struct Record input) {
    input.tag += 4;
    input.value += 17;
    input.step -= 3;
    return input;
}

unsigned long long wide_transform(unsigned long long input, unsigned step) {
    return input + step;
}

int variable_stack(unsigned count) {
    volatile int values[count];
    for (unsigned i = 0; i < count; ++i) values[i] = (int)(i * 3);
    int total = recurse(5);
    for (unsigned i = 0; i < count; ++i) total += values[i];
    return total;
}
