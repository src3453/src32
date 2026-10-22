; test.app (WAX): checks the v3 entry state and calls into test.math and
; test.base through its import descriptors (WAX v3 §6). Exits with 42, or
; with the number of the first failed step. build.py prepends APP_BASE and
; the S_* / C_* constants: image-relative offsets of each descriptor's
; address and context words, all in the BSS block it adds.

.ORG APP_BASE
start:
    ; 1: entry state (Foundation §9.2): UGP = base_addr, R1 = R31 = 0,
    ; zeroed stack, bound descriptors.
    LDIH R7, start
    LDIL R7, start
    BNE R30, R7, fail1
    BNE R1, R0, fail1
    BNE R31, R0, fail1
    LD R7, [R28 - 4]
    BNE R7, R0, fail1
    LD R7, [R30 + S_TWICE]
    BEQ R7, R0, fail1

    ; 2: math.twice(21) = base.add(21, 21) = 42; the callee preserves the
    ; module context it received.
    ADDI R28, R28, -16
    ADDI R7, R0, 21
    ST R7, [R28 + 0]
    ST R30, [R28 + 8]
    LD R5, [R30 + S_TWICE]
    LD R6, [R30 + C_TWICE]
    ADD R30, R6, R0
    JALR R5
    ADD R9, R30, R0
    LD R30, [R28 + 8]
    ADDI R28, R28, 16
    ADDI R7, R0, 42
    BNE R1, R7, fail2
    LD R7, [R30 + C_TWICE]
    BNE R9, R7, fail2

    ; 3: math.apply((inc, APP_BASE), 20) = inc(20) + 1 = 22. A callback
    ; runs with the context of its own descriptor.
    ADDI R28, R28, -24
    LDIH R7, inc
    LDIL R7, inc
    ST R7, [R28 + 0]
    ST R30, [R28 + 4]
    ADDI R7, R0, 20
    ST R7, [R28 + 8]
    ST R30, [R28 + 16]
    LD R5, [R30 + S_APPLY]
    LD R6, [R30 + C_APPLY]
    ADD R30, R6, R0
    JALR R5
    LD R30, [R28 + 16]
    ADDI R28, R28, 24
    ADDI R7, R0, 22
    BNE R1, R7, fail3

    ; 4: base.add64(0x1_FFFFFFFF, 1) = 0x2_00000000 (high word first).
    ADDI R28, R28, -24
    ADDI R7, R0, 1
    ST R7, [R28 + 0]
    ADDI R7, R0, -1
    ST R7, [R28 + 4]
    ST R0, [R28 + 8]
    ADDI R7, R0, 1
    ST R7, [R28 + 12]
    ST R30, [R28 + 16]
    LD R5, [R30 + S_ADD64]
    LD R6, [R30 + C_ADD64]
    ADD R30, R6, R0
    JALR R5
    LD R30, [R28 + 16]
    ADDI R28, R28, 24
    BNE R1, R0, fail4
    ADDI R7, R0, 2
    BNE R2, R7, fail4

    ; 5: base.add through two slots that import the same symbol.
    ADDI R28, R28, -16
    ADDI R7, R0, 1
    ST R7, [R28 + 0]
    ADDI R7, R0, 2
    ST R7, [R28 + 4]
    ST R30, [R28 + 8]
    LD R5, [R30 + S_ADD_A]
    LD R6, [R30 + C_ADD_A]
    ADD R30, R6, R0
    JALR R5
    LD R30, [R28 + 8]
    ADDI R7, R0, 3
    BNE R1, R7, fail5
    ST R1, [R28 + 0]
    ADDI R7, R0, 4
    ST R7, [R28 + 4]
    LD R5, [R30 + S_ADD_B]
    LD R6, [R30 + C_ADD_B]
    ADD R30, R6, R0
    JALR R5
    LD R30, [R28 + 8]
    ADDI R28, R28, 16
    ADDI R7, R0, 7
    BNE R1, R7, fail5

    ; 6: the counter object of this process's test.base instance saw the
    ; three calls to add; its descriptor names the owning module.
    LD R8, [R30 + S_COUNTER]
    LD R7, [R8 + 0]
    ADDI R9, R0, 3
    BNE R7, R9, fail6
    LD R7, [R30 + C_COUNTER]
    LD R9, [R30 + C_ADD_A]
    BNE R7, R9, fail6

    ; 7: test.math's own import of counter is the same object.
    ADDI R28, R28, -16
    ST R8, [R28 + 0]
    ST R30, [R28 + 8]
    LD R5, [R30 + S_CPTR]
    LD R6, [R30 + C_CPTR]
    ADD R30, R6, R0
    JALR R5
    LD R30, [R28 + 8]
    LD R8, [R28 + 0]
    ADDI R28, R28, 16
    BNE R1, R8, fail7

    ; 8: test.math runs with its own origin as R30.
    ADDI R28, R28, -8
    ST R30, [R28 + 0]
    LD R5, [R30 + S_ORIGIN]
    LD R6, [R30 + C_ORIGIN]
    ADD R30, R6, R0
    JALR R5
    LD R30, [R28 + 0]
    ADDI R28, R28, 8
    LD R7, [R30 + C_TWICE]
    BNE R1, R7, fail8

    ; 9: a system call keeps the module context and every register but R1.
    ADDI R9, R0, 99
    SYSCALL 0x0B
    BNE R1, R0, fail9
    ADDI R7, R0, 99
    BNE R9, R7, fail9
    LDIH R7, start
    LDIL R7, start
    BNE R30, R7, fail9

    ADDI R1, R0, 42
    JMP exit
fail1:
    ADDI R1, R0, 1
    JMP exit
fail2:
    ADDI R1, R0, 2
    JMP exit
fail3:
    ADDI R1, R0, 3
    JMP exit
fail4:
    ADDI R1, R0, 4
    JMP exit
fail5:
    ADDI R1, R0, 5
    JMP exit
fail6:
    ADDI R1, R0, 6
    JMP exit
fail7:
    ADDI R1, R0, 7
    JMP exit
fail8:
    ADDI R1, R0, 8
    JMP exit
fail9:
    ADDI R1, R0, 9
exit:
    SYSCALL 0x11
    JMP exit

; inc: fn(u32)->u32 = x + 1, or 0 when R30 is not this module's origin.
inc:
    LD R1, [R28 + 0]
    ADDI R1, R1, 1
    LDIH R7, start
    LDIL R7, start
    BEQ R30, R7, inc_ok
    ADD R1, R0, R0
inc_ok:
    JR R31
