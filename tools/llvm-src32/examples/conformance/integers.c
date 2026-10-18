#include "project.h"

int integer_checks(void) {
    unsigned x = 0x80000000u + seed;
    int negative = -(int)seed;
    volatile signed char c = -123;
    volatile unsigned char uc = 251;
    volatile short s = -12345;
    volatile unsigned short us = 65000;
    if (x <= 0x7FFFFFFFu || !(negative < 1)) return 1;
    if (x / 3u != 715827885u || x % 3u != 0) return 2;
    if (negative / 3 != -2 || negative % 3 != -1) return 3;
    if ((x >> 28) != 8 || (negative >> 2) != -2) return 4;
    if ((x << 2) != 28 || (x ^ 0x80000000u) != seed) return 5;
    if ((x & 255) != 7 || (x | 255) != 0x800000FFu) return 6;
    if ((int)c != -123 || (unsigned)uc != 251 || (int)s != -12345 ||
        (unsigned)us != 65000) return 7;
    if ((unsigned char)(seed + 256) != 7 || (short)(seed + 65536) != 7) return 8;
    if ((seed == 7 ? negative : 11) != -7) return 9;
    return 0;
}
