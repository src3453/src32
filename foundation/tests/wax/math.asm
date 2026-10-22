; test.math 1.2 (WAL): imports test.base's add and counter. Calls between
; modules follow WAX v3 §6: save R30 and R31, load both descriptor words,
; set R30 to the provider origin, JALR, then restore R30.
.CONST ADD_SLOT 0x100
.CONST ADD_CTX 0x104
.CONST COUNTER_SLOT 0x108

.ORG 0
; twice: fn(u32)->u32 = base.add(x, x)
twice:
    LD R7, [R28 + 0]
    ADDI R28, R28, -16
    ST R7, [R28 + 0]
    ST R7, [R28 + 4]
    ST R31, [R28 + 8]
    ST R30, [R28 + 12]
    LD R5, [R30 + ADD_SLOT]
    LD R6, [R30 + ADD_CTX]
    ADD R30, R6, R0
    JALR R5
    LD R30, [R28 + 12]
    LD R31, [R28 + 8]
    ADDI R28, R28, 16
    JR R31
; apply: fn(fnptr,u32)->u32 = callback(x) + 1. The callback descriptor is
; (address, origin) by value.
apply:
    LD R5, [R28 + 0]
    LD R6, [R28 + 4]
    LD R7, [R28 + 8]
    ADDI R28, R28, -16
    ST R7, [R28 + 0]
    ST R31, [R28 + 8]
    ST R30, [R28 + 12]
    ADD R30, R6, R0
    JALR R5
    LD R30, [R28 + 12]
    LD R31, [R28 + 8]
    ADDI R28, R28, 16
    ADDI R1, R1, 1
    JR R31
; counter: fn()->ptr, the address bound into this module's object slot.
counter:
    LD R1, [R30 + COUNTER_SLOT]
    JR R31
; origin: fn()->u32, the module context this call received.
origin:
    ADD R1, R30, R0
    JR R31
math_end:
