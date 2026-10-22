; test.base 1.0 (WAL): position-independent, image-relative to R30 (WAX v3
; call ABI 1). build.py adds the BSS block at COUNTER.
.CONST COUNTER 0x100

.ORG 0
; add: fn(u32,u32)->u32. Also counts its calls in the exported object.
add:
    LD R1, [R28 + 0]
    LD R2, [R28 + 4]
    ADD R1, R1, R2
    LD R3, [R30 + COUNTER]
    ADDI R3, R3, 1
    ST R3, [R30 + COUNTER]
    JR R31
    NOP
; add64: fn(u64,u64)->u64. Arguments are high word, then low word; the
; result is low in R1, high in R2.
add64:
    LD R3, [R28 + 4]
    LD R4, [R28 + 12]
    ADD R1, R3, R4
    SLTU R5, R1, R3
    LD R2, [R28 + 0]
    LD R6, [R28 + 8]
    ADD R2, R2, R6
    ADD R2, R2, R5
    JR R31
    NOP
base_end:
