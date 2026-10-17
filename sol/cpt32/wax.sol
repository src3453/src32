# WAX v2 executable loader (doc/spec_WAX.md).
#
# `wax_validate(buffer, file_size)` validates a complete .wax image and sets
# wax_entry, wax_table_offset and wax_block_count on success. `wax_load`
# validates, stages the file, then transfers control without returning.
# Input is limited to 64 KiB and must not overlap reserved RAM.
#
# The loader reserves 0x00FE0000..0x01000000 for an 8 KiB span bitmap, the
# staged image, and a copied transfer trampoline. Executable blocks may not
# overlap this range. The trampoline lets a WAX image replace DOS itself
# (including address 0) while the payload blocks are being copied.
!namespace wax -
!const WAX_OK 0
!const WAX_ERR_HEADER 1
!const WAX_ERR_LAYOUT 2
!const WAX_ERR_RANGE 3
!const WAX_ERR_OVERLAP 4
!const WAX_ERR_ENTRY 5
!const WAX_ERR_RESERVED 6
!const WAX_ERR_BUFFER 7
!const WAX_MAX_FILE 65536
!const WAX_STAGE 0x00FE0000
!const WAX_RESERVED_END 0x01000000
!const WAX_SPAN_MAP_BYTES 8192

!var wax_entry 0
!var wax_table_offset 0
!var wax_block_count 0

fn wax_be16 (buffer offset) :
    buffer offset add ldh ret
;

fn wax_be32 (buffer offset) :
    buffer offset add ld ret
;

# Return sol true (0) when a < b as unsigned 32-bit values.
fn wax_ult (a b) :
    a 0x80000000 xor b 0x80000000 xor lt ret
;

# Mark a file byte range in the 1-bit-per-byte staging map; return an error
# when any byte was already claimed by another payload or the debug block.
fn wax_mark_span (start size) :
    local i 0
    local pos 0
    local addr 0
    local mask 0
    while
        i size lt
        if
            start i add >pos
            WAX_STAGE pos 3 shr add >addr
            1 pos 7 and shl >mask
            addr ldb mask and 0 neq if WAX_ERR_OVERLAP ret end
            addr ldb mask or addr stb
            i 1 add >i
            0
        else
            1
        end
    end
    WAX_OK ret
;

# Validate a complete WAX v2 executable. Uses WAX_STAGE as a temporary span map.
fn wax_validate (buffer file_size) :
    local flags 0
    local base 0
    local entry_offset 0
    local table_offset 0
    local table_end 0
    local block_count 0
    local debug_offset 0
    local debug_size 0
    local index 0
    local j 0
    local record 0
    local load_address 0
    local load_last 0
    local file_offset 0
    local block_size 0
    local input_last 0
    local previous_last 0
    local span_status 0
    local entry 0
    local found 0

    file_size 64 lt file_size WAX_MAX_FILE gt and if WAX_ERR_HEADER ret end
    buffer file_size 1 sub add >input_last
    input_last buffer wax_ult 0 eq if WAX_ERR_BUFFER ret end
    buffer WAX_RESERVED_END wax_ult 0 eq
    input_last WAX_STAGE wax_ult 0 neq or if WAX_ERR_BUFFER ret end
    buffer 0 wax_be32 0x57415800 neq if WAX_ERR_HEADER ret end
    buffer 4 wax_be16 2 neq if WAX_ERR_HEADER ret end
    buffer 6 wax_be16 1 neq if WAX_ERR_HEADER ret end
    buffer 8 wax_be16 1 neq if WAX_ERR_HEADER ret end
    buffer 10 wax_be16 >flags
    flags 0xFFFE and 0 neq if WAX_ERR_HEADER ret end
    buffer 12 wax_be32 64 neq if WAX_ERR_HEADER ret end
    buffer 16 wax_be32 >base
    buffer 20 wax_be32 >entry_offset
    buffer 24 wax_be32 >table_offset
    buffer 28 wax_be32 >block_count
    buffer 32 wax_be32 12 neq if WAX_ERR_LAYOUT ret end
    buffer 36 wax_be32 >debug_offset
    buffer 40 wax_be32 >debug_size
    buffer 44 wax_be32 file_size neq if WAX_ERR_HEADER ret end

    0 >index
    while
        index 16 lt
        if
            buffer 48 add index add ldb 0 neq if WAX_ERR_HEADER ret end
            index 1 add >index
            0
        else
            1
        end
    end

    block_count 0 le table_offset 64 lt and table_offset file_size gt and if WAX_ERR_LAYOUT ret end
    table_offset 3 and 0 neq if WAX_ERR_LAYOUT ret end
    block_count file_size table_offset sub 12 div gt if WAX_ERR_LAYOUT ret end
    table_offset block_count 12 mul add >table_end
    table_end file_size gt if WAX_ERR_LAYOUT ret end

    debug_size 0 neq if
        flags 1 and 0 eq if WAX_ERR_LAYOUT ret end
        debug_size 0 lt debug_offset 0 lt and if WAX_ERR_RANGE ret end
        debug_offset 3 and 0 neq debug_offset table_end lt and if WAX_ERR_LAYOUT ret end
        debug_offset file_size gt if WAX_ERR_RANGE ret end
        debug_size file_size debug_offset sub gt if WAX_ERR_RANGE ret end
    else
        flags 1 and 0 neq debug_offset 0 neq and if WAX_ERR_LAYOUT ret end
    end
    buffer table_offset add 0 wax_be32 base neq if WAX_ERR_LAYOUT ret end
    0 >j
    while
        j WAX_SPAN_MAP_BYTES lt
        if
            0 WAX_STAGE j add stb
            j 1 add >j
            0
        else
            1
        end
    end
    0 >index
    while
        index block_count lt
        if
            buffer table_offset add index 12 mul add >record
            record 0 wax_be32 >load_address
            record 4 wax_be32 >file_offset
            record 8 wax_be32 >block_size
            block_size 0 le if WAX_ERR_RANGE ret end

            load_address block_size 1 sub add >load_last
            load_last load_address wax_ult 0 eq if WAX_ERR_RANGE ret end

            file_offset table_end lt file_offset 3 and 0 neq and if WAX_ERR_RANGE ret end
            file_offset file_size gt block_size file_size file_offset sub gt and if WAX_ERR_RANGE ret end

            # The DOS loader uses this high-RAM window for its staged file and
            # copied transfer routine. Reject any image that would destroy it.
            load_address WAX_RESERVED_END wax_ult 0 eq
            load_last WAX_STAGE wax_ult 0 neq or
            if WAX_ERR_RESERVED ret end

            file_offset block_size wax_mark_span >span_status
            span_status 0 neq if span_status ret end
            index 1 add >index
            0
        else
            1
        end
    end
    debug_size 0 neq if
        debug_offset debug_size wax_mark_span >span_status
        span_status 0 neq if span_status ret end
    end
    buffer table_offset add >record
    record 0 wax_be32 >load_address
    record 8 wax_be32 1 sub load_address add >previous_last
    1 >index
    while
        index block_count lt
        if
            buffer table_offset add index 12 mul add >record
            record 0 wax_be32 >load_address
            record 8 wax_be32 1 sub load_address add >load_last
            load_address previous_last wax_ult load_address previous_last eq and 0 eq if WAX_ERR_LAYOUT ret end
            load_last >previous_last
            index 1 add >index
            0
        else
            1
        end
    end

    # entry = base + offset must not wrap, then must lie in one block.
    base entry_offset add >entry
    entry_offset 0 neq entry base wax_ult 0 eq or if WAX_ERR_ENTRY ret end
    0 >found
    0 >index
    while
        index block_count lt
        if
            buffer table_offset add index 12 mul add >record
            record 0 wax_be32 >load_address
            record 8 wax_be32 1 sub load_address add >load_last
            entry load_address wax_ult 0 neq
            load_last entry wax_ult 0 neq or 0 eq if 1 >found end
            index 1 add >index
            0
        else
            1
        end
    end
    found 0 eq if WAX_ERR_ENTRY ret end

    entry >wax_entry
    table_offset >wax_table_offset
    block_count >wax_block_count
    WAX_OK ret
;

# Validate, copy the source file to staging RAM, then execute all blocks.
# Return value is an error code; success never returns to DOS.
fn wax_load (buffer file_size) :
    local i 0
    buffer file_size wax_validate >i
    i 0 neq if i ret end

    0 >i
    while
        i file_size lt
        if
            buffer i add ldb WAX_STAGE i add stb
            i 1 add >i
            0
        else
            1
        end
    end
    WAX_STAGE wax_table_offset wax_block_count wax_entry wax_transfer
    WAX_ERR_BUFFER ret
;

# This function copies its self-contained assembly trampoline into reserved
# high RAM, then jumps there. The trampoline reads no DOS code or stack after
# starting to load, so WAX blocks may safely replace the DOS image and stack.
fn wax_transfer (buffer table_offset block_count entry) :
!asm
    LDIH R1, __wax_trampoline_start
    LDIL R1, __wax_trampoline_start
    LDIH R2, __wax_trampoline_end
    LDIL R2, __wax_trampoline_end
    LDIH R3, 0x00FF
    LDIL R3, 0x0000
__wax_copy_trampoline:
    BEQ R1, R2, __wax_trampoline_copied
    LDB R4, [R1 + 0]
    STB [R3 + 0], R4
    ADDI R1, R1, 1
    ADDI R3, R3, 1
    JMP __wax_copy_trampoline
__wax_trampoline_copied:
    LD R1, [R26 + 8]
    LD R2, [R26 + 12]
    LD R3, [R26 + 16]
    LD R4, [R26 + 20]
    LDIH R15, 0x00FF
    LDIL R15, 0x0000
    JR R15
__wax_trampoline_start:
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
__wax_trampoline_end:
!end
    retn
;
!end
