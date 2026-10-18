#include "project.h"

static volatile unsigned long long wide = 0xFEDCBA9880000007ULL;

int wide_checks(void) {
    unsigned long long a = wide;
    unsigned shift = seed + 28;
    if ((unsigned)(a >> 32) != 0xFEDCBA98u || (unsigned)a != 0x80000007u) return 1;
    if (a + 0xFFFFFFFFULL != 0xFEDCBA9980000006ULL) return 2;
    if (a - 0x80000008ULL != 0xFEDCBA97FFFFFFFFULL) return 3;
    if ((a >> shift) != 0x1FDB9753ULL || (a << shift) != 0x3800000000ULL) return 4;
    if ((long long)a >= 0 || a <= 0x7FFFFFFFFFFFFFFFULL) return 5;
    if (((long long)a >> 35) != -2386093LL) return 6;
    if (a / 17ULL != 1080279914391489355ULL || a % 17ULL != 12ULL) return 7;
    if ((long long)-1234567890123LL / (long long)(int)seed != -176366841446LL)
        return 8;
    if ((long long)-1234567890123LL % (long long)(int)seed != -1LL) return 9;
    if ((a & 0xFFFFFFFFULL) != 0x80000007ULL ||
        (a ^ 0xFEDCBA9800000000ULL) != 0x80000007ULL) return 10;
    wide = (unsigned long long)seed * 0x100000001ULL;
    if (wide != 0x700000007ULL) return 11;
    if (wide_transform(0x12345678FFFFFFFEULL, seed) != 0x1234567900000005ULL)
        return 12;
    if (a * (((unsigned long long)seed << 32) | 3) != 0x7C962FFA80000015ULL)
        return 13;
    if (0xFFFFFFFFFFFFFFFEULL / 0x8000000000000001ULL != 1ULL ||
        0xFFFFFFFFFFFFFFFEULL % 0x8000000000000001ULL != 0x7FFFFFFFFFFFFFFDULL)
        return 14;
    if ((long long)0xFFFFFFFFFFFFFFFEULL / (long long)0x8000000000000001ULL != 0 ||
        (long long)0xFFFFFFFFFFFFFFFEULL % (long long)0x8000000000000001ULL != -2)
        return 15;
    if ((a >> 63) != 1 || ((long long)a >> 63) != -1 ||
        (a << 63) != 0x8000000000000000ULL) return 16;
    return 0;
}
