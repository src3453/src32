!const VRAM_BASE 0x10000000
!const VRAM_SIZE 2400

!include "sol/solstd/solstd_prng.sol"

fn step () :
    local scan 0
    local ptr 0
    while
        scan >ptr
        ptr VRAM_BASE add >ptr
        rand_lfsr ptr stb
        0x3F ptr 0x1000 add stb
        scan 1 add >scan
        scan VRAM_SIZE lt
    end
    retn
;

1 0x80030001 stb
1 0x8003F003 stb

!var c 0
while
    step
    0
end

