!const VRAM_BASE 0x10000000
!const VRAM_SIZE 0x12c00

!include "sol/cpt32/pec_rng.sol"

initPRNG

fn step (i) :
    local scan 0
    local ptr 0
    while
        scan VRAM_BASE add >ptr
        rand ptr stb
        scan 1 add >scan
        scan VRAM_SIZE lt
    end
    retn
;

!var c 0
while
    c step
    c 1 add >c
    0
end

