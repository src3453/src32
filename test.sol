!const VRAM_BASE 0x10000000
!const VRAM_SIZE 0x12c00

fn step (i) :
    local scan 0
    local ptr 0
    while
        scan >ptr
        ptr VRAM_BASE add >ptr
        ptr i mod ptr stb
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

