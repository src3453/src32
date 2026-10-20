#include <stdint.h>
#include <cpt32/pcg.h>
#include "softfloat.h"

static uint32_t failures;

static void hex32(uint32_t value) {
    static const char digits[] = "0123456789ABCDEF";
    for (int shift = 28; shift >= 0; shift -= 4)
        pcg_putcscr(digits[(value >> shift) & 15u]);
}

/* Check the actual library result and sticky flags before displaying them. */
static void check(const char *label, float32_t value,
                  uint32_t expected, uint_fast8_t flags) {
    uint32_t actual_flags = softfloat_exceptionFlags;
    uint32_t ok = value.v == expected && actual_flags == flags;
    failures += !ok;
    pcg_prnscr(label);
    pcg_prnscr("  ");
    hex32(value.v);
    pcg_prnscr(" flags=");
    hex32(actual_flags);
    pcg_prnlnscr(ok ? " OK" : " FAIL");
    softfloat_exceptionFlags = 0;
}

int main(void) {
    pcg_initPCG();
    pcg_col80();
    pcg_cls(0);
    pcg_colorfg(0x3f);
    pcg_colorbg(0);
    pcg_prnlnscr("Berkeley SoftFloat Release 3e / C / SRC32");
    pcg_prnlnscr("IEEE-754 binary32 bits + exception flags");
    softfloat_roundingMode = softfloat_round_near_even;
    softfloat_detectTininess = softfloat_tininess_afterRounding;
    softfloat_exceptionFlags = 0;

    float32_t zero = {0}, one = {0x3f800000}, two = {0x40000000};
    float32_t three = i32_to_f32(3);
    check("int32 3 -> f32", three, 0x40400000, 0);
    check("1 + 2 = 3", f32_add(one, two), 0x40400000, 0);
    check("3 - 1 = 2", f32_sub(three, one), 0x40000000, 0);
    check("3 * 2 = 6", f32_mul(three, two), 0x40c00000, 0);
    check("1 / 3 nearest", f32_div(one, three), 0x3eaaaaab, softfloat_flag_inexact);
    softfloat_roundingMode = softfloat_round_minMag;
    check("1 / 3 toward zero", f32_div(one, three), 0x3eaaaaaa, softfloat_flag_inexact);
    softfloat_roundingMode = softfloat_round_near_even;
    check("1 / 0 = +infinity", f32_div(one, zero), 0x7f800000, softfloat_flag_infinite);
    /* 8086 specialization chooses the negative quiet default NaN. */
    check("0 / 0 = quiet NaN", f32_div(zero, zero), 0xffc00000, softfloat_flag_invalid);
    float32_t subnormal = {1}, negative_zero = {0x80000000};
    check("min subnormal * 2", f32_mul(subnormal, two), 2, 0);
    check("-zero * 2", f32_mul(negative_zero, two), 0x80000000, 0);
    float32_t maximum = {0x7f7fffff};
    check("max * 2 overflow", f32_mul(maximum, two), 0x7f800000,
          softfloat_flag_overflow | softfloat_flag_inexact);
    check("min subnormal / 2", f32_div(subnormal, two), 0,
          softfloat_flag_underflow | softfloat_flag_inexact);
    float32_t halfway = {0x40200000}; /* 2.5 */
    int32_t rounded = f32_to_i32(halfway, softfloat_round_near_even, true);
    if (rounded != 2 || softfloat_exceptionFlags != softfloat_flag_inexact)
        ++failures;
    softfloat_exceptionFlags = 0;
    rounded = f32_to_i32(halfway, softfloat_round_near_maxMag, true);
    if (rounded != 3 || softfloat_exceptionFlags != softfloat_flag_inexact)
        ++failures;
    pcg_prnlnscr("f32 2.5 -> int32: nearest-even=2, ties-away=3");
    pcg_prnlnscr(failures ? "FAIL" : "PASS: all 14 checks");
    const volatile uint8_t *text = (const volatile uint8_t *)pcg_vram_base;
    if (text[0] != 'B' || text[80] != 'I')
        return 2;
    return failures ? 1 : 42;
}
