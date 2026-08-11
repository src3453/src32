# solstd_prng.sol
# 32-bit Galois LFSR pseudo-random number generator

!var lfsr 0xACE1

fn rand_lfsr () :
    local bit

    # Get LSB
    lfsr 1 and >bit

    # Shift right
    lfsr 1 shr >lfsr

    # If old LSB was 1, apply polynomial
    bit 0 eq if
        # LSB was 0: nothing
    else
        lfsr 0x80200003 xor >lfsr
    end

    lfsr ret
;