; Raw SRC32 trampoline copied to 0x00FF0000 before payload blocks are copied.
; It has no prologue: at entry R28 points at the four stack arguments
; (staged_buffer, table_offset, block_count, entry), each one word apart.
    LD R1, [R28 + 0]
    LD R2, [R28 + 4]
    LD R3, [R28 + 8]
    LD R4, [R28 + 12]
    ADD R2, R1, R2
__wax_load_block:
    BEQ R3, R0, __wax_launch_entry
    LD R5, [R2 + 0]
    LD R6, [R2 + 4]
    LD R7, [R2 + 8]
    ADD R6, R1, R6
__wax_copy_payload:
    BEQ R7, R0, __wax_next_block
    LDB R8, [R6 + 0]
    STB [R5 + 0], R8
    ADDI R5, R5, 1
    ADDI R6, R6, 1
    ADDI R7, R7, -1
    JMP __wax_copy_payload
__wax_next_block:
    ADDI R2, R2, 12
    ADDI R3, R3, -1
    JMP __wax_load_block
__wax_launch_entry:
    JR R4
