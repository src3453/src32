# softfloat.sol - IEEE 754 binary32 arithmetic using sol integer operators
#
# Values are raw binary32 bit patterns (the same representation produced by
# sol float literals such as 1.0f). Public operations:
#   sf32_add(a b), sf32_sub(a b), sf32_mul(a b), sf32_div(a b)
#   sf32_neg(x), sf32_abs(x), sf32_eq(a b), sf32_lt(a b)
# Comparisons use sol's boolean convention: 0 is true and 1 is false.
# Arithmetic rounds to nearest, ties to even, and supports subnormals,
# infinities, and NaNs. NaN arithmetic returns the canonical quiet NaN.

# Internal classification: 0 finite nonzero, 1 zero, 2 infinity, 3 NaN.
fn sf32_kind (x) :
    local exp
    local frac
    x 23 shr 255 and >exp
    x 0x007FFFFF and >frac
    exp 255 eq if
        frac 0 neq if
            3 ret
        else
            2 ret
        end
    else
        exp 0 eq if
            frac 0 eq if
                1 ret
            end
        end
    end
    0 ret
;

# Shift a nonnegative extended significand right, jamming discarded bits into
# bit zero. The operands used here are at most 28 bits wide.
fn sf32_shift_jam (value distance) :
    local result
    local mask
    local lost
    distance 0 eq if
        value ret
    end
    distance 32 ge if
        value 0 neq if
            1 ret
        end
        0 ret
    end
    value distance shr >result
    1 distance shl 1 sub >mask
    value mask and >lost
    lost 0 neq if
        result 1 or >result
    end
    result ret
;

# Pack sign, unbiased exponent, and a significand with three rounding bits.
# The significand's normal hidden bit is bit 26.
fn sf32_pack (sign exponent significand) :
    local sign_value
    local e
    local sig
    local main
    local remainder
    local exp_field
    local bits
    sign 0x80000000 and >sign_value
    exponent >e
    significand >sig

    sig 0 eq if
        sign_value ret
    end

    while
        sig 0x08000000 ge if
            sig 1 sf32_shift_jam >sig
            e 1 add >e
            sig 0x08000000 ge
        else
            1
        end
    end

    while
        sig 0x04000000 lt e -126 gt or if
            sig 1 shl >sig
            e 1 sub >e
            sig 0x04000000 lt e -126 gt or
        else
            1
        end
    end

    e -126 lt if
        -126 e sub sig swap sf32_shift_jam >sig
        -126 >e
    end
    e 127 gt if
        sign_value 0x7F800000 or ret
    end

    sig 3 shr >main
    sig 7 and >remainder
    remainder 4 gt if
        main 1 add >main
    else
        remainder 4 eq if
            main 1 and 0 neq if
                main 1 add >main
            end
        end
    end

    main 0x01000000 ge if
        main 1 shr >main
        e 1 add >e
    end
    e 127 gt if
        sign_value 0x7F800000 or ret
    end

    main 0x00800000 ge if
        e 127 add 23 shl >exp_field
    else
        0 >exp_field
    end
    sign_value exp_field or main 0x007FFFFF and or >bits
    bits ret
;

fn sf32_neg (x) :
    x 0x80000000 xor ret
;

fn sf32_abs (x) :
    x 0x7FFFFFFF and ret
;

fn sf32_add (a b) :
    local kind_a
    local kind_b
    local sign_a
    local sign_b
    local exp_a
    local exp_b
    local e
    local sig_a
    local sig_b
    local sig
    local sign

    a sf32_kind >kind_a
    b sf32_kind >kind_b
    kind_a 3 eq kind_b 3 eq and if
        0x7FC00000 ret
    end
    kind_a 2 eq kind_b 2 eq or if
        a b xor 0x80000000 and 0 neq if
            0x7FC00000 ret
        end
        a ret
    end
    kind_a 2 eq if
        a ret
    end
    kind_b 2 eq if
        b ret
    end
    kind_a 1 eq kind_b 1 eq or if
        a b xor 0x80000000 and 0 neq if
            0 ret
        end
        a ret
    end

    a 0x80000000 and >sign_a
    b 0x80000000 and >sign_b
    a 23 shr 255 and >exp_a
    b 23 shr 255 and >exp_b
    exp_a 0 eq if
        -126 >exp_a
    else
        exp_a 127 sub >exp_a
    end
    exp_b 0 eq if
        -126 >exp_b
    else
        exp_b 127 sub >exp_b
    end

    a 0x007FFFFF and >sig_a
    b 0x007FFFFF and >sig_b
    a 23 shr 255 and 0 neq if
        sig_a 0x00800000 or >sig_a
    end
    b 23 shr 255 and 0 neq if
        sig_b 0x00800000 or >sig_b
    end
    sig_a 3 shl >sig_a
    sig_b 3 shl >sig_b

    exp_a exp_b gt if
        exp_a >e
        exp_a exp_b sub sig_b swap sf32_shift_jam >sig_b
        sign_a >sign
    else
        exp_b exp_a gt if
            exp_b >e
            exp_b exp_a sub sig_a swap sf32_shift_jam >sig_a
            sign_b >sign
        else
            exp_a >e
            sign_a >sign
        end
    end
    sign_a sign_b neq if
        sig_a sig_b lt if
            sig_a >sig
            sig_b >sig_a
            sig >sig_b
            sign_b >sign
        end
    end

    sign_a sign_b eq if
        sig_a sig_b add >sig
    else
        sig_a sig_b sub >sig
    end
    sig 0 eq if
        0 >sign
    end
    sign e sig sf32_pack ret
;

fn sf32_sub (a b) :
    a b 0x80000000 xor sf32_add ret
;

fn sf32_mul (a b) :
    local kind_a
    local kind_b
    local sign
    local exp_a
    local exp_b
    local ma
    local mb
    local a0
    local a1
    local b0
    local b1
    local p0
    local p1
    local p2
    local t
    local limb0
    local limb1
    local limb2
    local limb3
    local sig
    local e

    a sf32_kind >kind_a
    b sf32_kind >kind_b
    kind_a 3 eq kind_b 3 eq and if
        0x7FC00000 ret
    end
    kind_a 2 eq kind_b 1 eq or kind_a 1 eq kind_b 2 eq or and if
        0x7FC00000 ret
    end
    a b xor 0x80000000 and >sign
    kind_a 2 eq kind_b 2 eq and if
        sign 0x7F800000 or ret
    end
    kind_a 1 eq kind_b 1 eq and if
        sign ret
    end

    a 23 shr 255 and >exp_a
    b 23 shr 255 and >exp_b
    exp_a 0 eq if
        -126 >exp_a
    else
        exp_a 127 sub >exp_a
    end
    exp_b 0 eq if
        -126 >exp_b
    else
        exp_b 127 sub >exp_b
    end
    a 0x007FFFFF and >ma
    b 0x007FFFFF and >mb
    a 23 shr 255 and 0 neq if
        ma 0x00800000 or >ma
    end
    b 23 shr 255 and 0 neq if
        mb 0x00800000 or >mb
    end
    while
        ma 0x00800000 lt if
            ma 1 shl >ma
            exp_a 1 sub >exp_a
            ma 0x00800000 lt
        else
            1
        end
    end
    while
        mb 0x00800000 lt if
            mb 1 shl >mb
            exp_b 1 sub >exp_b
            mb 0x00800000 lt
        else
            1
        end
    end

    ma 0xFFF and >a0
    ma 12 shr >a1
    mb 0xFFF and >b0
    mb 12 shr >b1
    a0 b0 mul >p0
    a0 b1 mul a1 b0 mul add >p1
    a1 b1 mul >p2
    p0 12 shr p1 add >t
    p0 0xFFF and >limb0
    t 0xFFF and >limb1
    t 12 shr p2 add >t
    t 0xFFF and >limb2
    t 12 shr >limb3

    limb3 0x800 and 0 neq if
        limb1 9 shr limb2 3 shl or limb3 15 shl or >sig
        limb0 0 neq limb1 0x1FF and 0 neq and if
            sig 1 or >sig
        end
        exp_a exp_b add 1 add >e
    else
        limb1 8 shr limb2 4 shl or limb3 16 shl or >sig
        limb0 0 neq limb1 0xFF and 0 neq and if
            sig 1 or >sig
        end
        exp_a exp_b add >e
    end
    sign e sig sf32_pack ret
;

fn sf32_div (a b) :
    local kind_a
    local kind_b
    local sign
    local exp_a
    local exp_b
    local ma
    local mb
    local remainder
    local quotient
    local i
    local e

    a sf32_kind >kind_a
    b sf32_kind >kind_b
    kind_a 3 eq kind_b 3 eq and if
        0x7FC00000 ret
    end
    kind_a 1 eq kind_b 1 eq or kind_a 2 eq kind_b 2 eq or and if
        0x7FC00000 ret
    end
    a b xor 0x80000000 and >sign
    kind_a 2 eq if
        sign 0x7F800000 or ret
    end
    kind_b 2 eq if
        sign ret
    end
    kind_b 1 eq if
        sign 0x7F800000 or ret
    end
    kind_a 1 eq if
        sign ret
    end

    a 23 shr 255 and >exp_a
    b 23 shr 255 and >exp_b
    exp_a 0 eq if
        -126 >exp_a
    else
        exp_a 127 sub >exp_a
    end
    exp_b 0 eq if
        -126 >exp_b
    else
        exp_b 127 sub >exp_b
    end
    a 0x007FFFFF and >ma
    b 0x007FFFFF and >mb
    a 23 shr 255 and 0 neq if
        ma 0x00800000 or >ma
    end
    b 23 shr 255 and 0 neq if
        mb 0x00800000 or >mb
    end
    while
        ma 0x00800000 lt if
            ma 1 shl >ma
            exp_a 1 sub >exp_a
            ma 0x00800000 lt
        else
            1
        end
    end
    while
        mb 0x00800000 lt if
            mb 1 shl >mb
            exp_b 1 sub >exp_b
            mb 0x00800000 lt
        else
            1
        end
    end

    exp_a exp_b sub >e
    ma mb lt if
        ma 1 shl >ma
        e 1 sub >e
    end
    ma mb sub >remainder
    1 >quotient
    0 >i
    while
        quotient 1 shl >quotient
        remainder 1 shl >remainder
        remainder mb ge if
            remainder mb sub >remainder
            quotient 1 or >quotient
        end
        i 1 add >i
        i 26 lt
    end
    remainder 0 neq if
        quotient 1 or >quotient
    end
    sign e quotient sf32_pack ret
;

# IEEE comparisons; NaN is unordered, so both predicates return false for it.
fn sf32_eq (a b) :
    local kind_a
    local kind_b
    a sf32_kind >kind_a
    b sf32_kind >kind_b
    kind_a 3 eq kind_b 3 eq and if
        1 ret
    end
    kind_a 1 eq kind_b 1 eq or if
        0 ret
    end
    a b eq if
        0 ret
    end
    1 ret
;

fn sf32_lt (a b) :
    local kind_a
    local kind_b
    local sign_a
    local sign_b
    local mag_a
    local mag_b
    a sf32_kind >kind_a
    b sf32_kind >kind_b
    kind_a 3 eq kind_b 3 eq and if
        1 ret
    end
    kind_a 1 eq kind_b 1 eq or if
        1 ret
    end
    a 0x80000000 and >sign_a
    b 0x80000000 and >sign_b
    sign_a sign_b neq if
        sign_a 0 neq if
            0 ret
        end
        1 ret
    end
    a 0x7FFFFFFF and >mag_a
    b 0x7FFFFFFF and >mag_b
    sign_a 0 neq if
        mag_a mag_b gt ret
    end
    mag_a mag_b lt ret
;
