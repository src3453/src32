cmake_minimum_required(VERSION 3.20)
project(cpt32_softfloat_c LANGUAGES NONE)
add_subdirectory("${CMAKE_CURRENT_LIST_DIR}/../../sdk/cpt32" cpt32)

# Explicit subset: binary32 arithmetic/conversion and its portable dependencies.
set(sf vendor/softfloat/source)
src32_add_library(berkeley_softfloat
    SOURCES
        ${sf}/f32_add.c ${sf}/f32_sub.c ${sf}/f32_mul.c ${sf}/f32_div.c
        ${sf}/i32_to_f32.c ${sf}/f32_to_i32.c ${sf}/softfloat_state.c
        ${sf}/s_addMagsF32.c ${sf}/s_subMagsF32.c
        ${sf}/s_normSubnormalF32Sig.c ${sf}/s_roundPackToF32.c
        ${sf}/s_normRoundPackToF32.c ${sf}/s_roundToI32.c
        ${sf}/s_shiftRightJam32.c ${sf}/s_shiftRightJam64.c
        ${sf}/s_shortShiftRightJam64.c
        ${sf}/s_countLeadingZeros8.c ${sf}/s_countLeadingZeros32.c
        ${sf}/s_approxRecip32_1.c ${sf}/s_approxRecip_1Ks.c
        ${sf}/8086/softfloat_raiseFlags.c ${sf}/8086/s_propagateNaNF32UI.c
    INCLUDE_DIRECTORIES port ${sf}/include ${sf}/8086
    DEFINITIONS INLINE_LEVEL=0 SOFTFLOAT_ROUND_ODD)

src32_add_executable(softfloat_demo
    SOURCES main.c
    LIBRARIES berkeley_softfloat cpt32_display
    OPTIMIZATION 2)
