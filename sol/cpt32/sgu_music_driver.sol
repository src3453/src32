# SGUB v1/v2 player. The stream is loaded at the fixed 2 MiB main-RAM address.
!const IRQC_PENDING 0xFFFF0040
!const IRQC_ENABLE 0xFFFF0044
!const STREAM_BASE 0x00200000
!const STREAM_LIMIT 0x00E00000
!const MAIN_RAM_END 0x01000000
!const SGU0_BASE 0x80020000
!const SGU1_BASE 0x80021000
!const SGU0_REGS 0x80020800
!const SGU1_REGS 0x80021800
!const PCMRAM_BASE 0x18000000

!var stream_valid 0
!var stream_end 0
!var stream_repeat 0
!var stream_frame_count 0
!var stream_frame_index 0
!var stream_initial_count 0
!var stream_frame_start 0
!var stream_frame_cursor 0
!var stream_total_size 0
!var stream_version 1
!var stream_v2_pool_count 0
!var stream_v2_init_count 0
# These are file-relative offsets. Rebase each access/cursor at STREAM_BASE.
!var stream_v2_directory 0
!var stream_v2_init_start 0
!var stream_v2_frame_start 0
!var stream_operation_limit 0

fn read_u16 (ptr) :
    ptr ldb 8 shl
    ptr 1 add ldb or
    ret
;

fn read_u24 (ptr) :
    ptr ldb 16 shl
    ptr 1 add ldb 8 shl or
    ptr 2 add ldb or
    ret
;

fn read_u32 (ptr) :
    ptr ldb 24 shl
    ptr 1 add ldb 16 shl or
    ptr 2 add ldb 8 shl or
    ptr 3 add ldb or
    ret
;

# Return Sol true (zero) only for valid targets and addresses.
fn record_valid (ptr) :
    local target 0
    local address 0
    ptr ldb >target
    ptr 1 add read_u24 >address
    target 2 eq if
        address 0x000FFFFF le if
            0 ret
        end
        1 ret
    end
    target 1 le if
        address 0x000008FF le if
            0 ret
        end
    end
    1 ret
;

# Execute a record that was checked during the startup scan.
fn apply_record (ptr) :
    local target 0
    local address 0
    local value 0
    ptr ldb >target
    ptr 1 add read_u24 >address
    ptr 4 add ldb >value
    target 0 eq if
        value SGU0_BASE address add stb
    else
        target 1 eq if
            value SGU1_BASE address add stb
        else
            value PCMRAM_BASE address add stb
        end
    end
;

fn write_target (target address value) :
    target 0 eq if
        value SGU0_BASE address add stb
    else
        target 1 eq if
            value SGU1_BASE address add stb
        else
            value PCMRAM_BASE address add stb
        end
    end
;

# Return the end pointer, or -1 if this complete operation is invalid.
fn validate_v2_operation (ptr) :
    local opcode 0
    local target 0
    local address 0
    local length 0
    local pool_id 0
    local source 0
    local kind 0
    local pool_length 0
    local pool_ptr 0
    ptr stream_operation_limit lt if
        ptr ldb >opcode
        ptr 5 add stream_operation_limit le if
            ptr 1 add ldb >target
            ptr 2 add read_u24 >address
            opcode 0 eq if
                ptr 6 add stream_operation_limit le if
                    target 0 eq target 1 eq and target 2 eq and if
                        target 2 eq if
                            address 0x000FFFFF le if ptr 6 add ret end
                        else
                            address 0x000008FF le if ptr 6 add ret end
                        end
                    end
                end
                -1 ret
            end
            opcode 1 eq if
                ptr 10 add stream_operation_limit le if
                    ptr 5 add read_u32 >length
                    length 0 gt if
                        target 2 eq if
                            address 0x000FFFFF le
                            length 0x00100000 address sub le or if ptr 10 add ret end
                        else
                            target 0 eq target 1 eq and if
                                address 0x000007FF le
                                length 0x00000800 address sub le or if ptr 10 add ret end
                            end
                        end
                    end
                end
                -1 ret
            end
            opcode 2 eq if
                ptr 15 add stream_operation_limit le if
                    target 0 eq target 1 eq and target 2 eq and if
                        ptr 5 add read_u16 >pool_id
                        pool_id stream_v2_pool_count lt if
                            STREAM_BASE stream_v2_directory add pool_id 16 mul add >pool_ptr
                            pool_ptr 2 add ldb >kind
                            pool_ptr 4 add read_u32 >pool_length
                            ptr 7 add read_u32 >source
                            ptr 11 add read_u32 >length
                            length 0 gt
                            source 0 ge or
                            source pool_length le or
                            length pool_length source sub le or if
                                target 2 eq if
                                    kind 2 eq if
                                        address length add 1 sub 0x000FFFFF le if ptr 15 add ret end
                                    end
                                else
                                    kind 1 eq if
                                        address 0x000007FF le
                                        address length add 1 sub 0x000007FF le or
                                        address 0x100 mod 0 eq or
                                        source 256 mod 0 eq or
                                        length 256 mod 0 eq or
                                        if ptr 15 add ret end
                                    end
                                end
                            end
                        end
                    end
                end
                -1 ret
            end
        end
    end
    -1 ret
;

fn validate_stream_v2 () :
    local total_size 0
    local frame_count 0
    local directory_end 0
    local expected_payload 0
    local entry 0
    local pool_index 0
    local pool_id 0
    local kind 0
    local codec 0
    local raw_size 0
    local stored_size 0
    local payload_offset 0
    local entry_valid 0
    local directory_valid 0
    local scan_ptr 0
    local frame_index 0
    local op_index 0
    local op_count 0
    local header_valid 0
    local section_valid 0
    0 >stream_valid
    2 >stream_version
    1 >header_valid
    STREAM_BASE ldb 0x53 neq if 0 >header_valid end
    STREAM_BASE 1 add ldb 0x47 neq if 0 >header_valid end
    STREAM_BASE 2 add ldb 0x55 neq if 0 >header_valid end
    STREAM_BASE 3 add ldb 0x42 neq if 0 >header_valid end
    STREAM_BASE 4 add ldb 2 neq if 0 >header_valid end
    STREAM_BASE 5 add ldb 16 neq if 0 >header_valid end
    STREAM_BASE 6 add ldb 1 gt if 0 >header_valid end
    STREAM_BASE 7 add ldb 0 neq if 0 >header_valid end
    STREAM_BASE 36 add read_u16 40 neq if 0 >header_valid end
    STREAM_BASE 38 add read_u16 0 neq if 0 >header_valid end
    header_valid 1 eq if
        STREAM_BASE 8 add read_u32 >frame_count
        STREAM_BASE 8 add read_u32 >stream_frame_count
        STREAM_BASE 12 add read_u32 >stream_v2_pool_count
        STREAM_BASE 16 add read_u32 >stream_v2_init_count
        STREAM_BASE 20 add read_u32 >stream_v2_directory
        STREAM_BASE 24 add read_u32 >stream_v2_init_start
        STREAM_BASE 28 add read_u32 >stream_v2_frame_start
        STREAM_BASE 32 add read_u32 >total_size
        total_size >stream_total_size
        STREAM_BASE 6 add ldb 1 and >stream_repeat
        1 >section_valid
        total_size 40 lt if 0 >section_valid end
        total_size STREAM_LIMIT gt if 0 >section_valid end
        frame_count 0 le if 0 >section_valid end
        stream_v2_pool_count 0 lt if 0 >section_valid end
        stream_v2_init_count 0 lt if 0 >section_valid end
        stream_v2_pool_count 65536 gt if 0 >section_valid end
        stream_v2_pool_count total_size 40 sub 16 div gt if 0 >section_valid end
        stream_v2_directory 40 neq if 0 >section_valid end
        section_valid 1 eq if
            STREAM_BASE total_size add >stream_end
            stream_end MAIN_RAM_END le if
                1 >directory_valid
                stream_v2_pool_count 16 mul 40 add >directory_end
                directory_end >expected_payload
                0 >pool_index
                while
                    pool_index stream_v2_pool_count lt
                    if
                        STREAM_BASE stream_v2_directory add pool_index 16 mul add >entry
                        entry read_u16 >pool_id
                        entry 2 add ldb >kind
                        entry 3 add ldb >codec
                        entry 4 add read_u32 >raw_size
                        entry 8 add read_u32 >stored_size
                        entry 12 add read_u32 >payload_offset
                        pool_id pool_index eq
                        codec 0 eq or
                        raw_size stored_size eq or
                        raw_size 0 gt or
                        payload_offset expected_payload eq or
                        if
                            1 >entry_valid
                            kind 1 eq if raw_size 256 neq if 0 >entry_valid end
                            else
                                kind 2 neq raw_size 0x00100000 gt and if 0 >entry_valid end
                            end
                            stored_size total_size payload_offset sub gt if 0 >entry_valid end
                            entry_valid 1 eq if
                                payload_offset stored_size add >expected_payload
                            else 0 >directory_valid end
                        else
                            0 >directory_valid
                        end
                    end
                    pool_index 1 add >pool_index
                    pool_index stream_v2_pool_count lt
                end
                1 >section_valid
                directory_end total_size gt if 0 >section_valid end
                expected_payload total_size gt if 0 >section_valid end
                directory_valid 1 neq if 0 >section_valid end
                stream_v2_init_start expected_payload neq if 0 >section_valid end
                stream_v2_frame_start stream_v2_init_start lt if 0 >section_valid end
                stream_v2_frame_start total_size gt if 0 >section_valid end
                stream_v2_init_count stream_v2_frame_start stream_v2_init_start sub 6 div gt if 0 >section_valid end
                frame_count total_size stream_v2_frame_start sub 2 div gt if 0 >section_valid end
                section_valid 1 eq if
                    1 >stream_valid
                    STREAM_BASE stream_v2_frame_start add >stream_operation_limit
                    STREAM_BASE stream_v2_init_start add >scan_ptr
                    0 >op_index
                    while
                        op_index stream_v2_init_count lt
                        stream_valid 1 eq or
                        if
                            scan_ptr validate_v2_operation >scan_ptr
                            scan_ptr 0 lt if 0 >stream_valid end
                            op_index 1 add >op_index
                        end
                        op_index stream_v2_init_count lt
                        stream_valid 1 eq or
                    end
                    scan_ptr STREAM_BASE stream_v2_frame_start add eq
                    if
                        0 >frame_index
                        while
                            frame_index frame_count lt
                            stream_valid 1 eq or
                            if
                                scan_ptr 2 add stream_end le
                                if
                                    scan_ptr read_u16 >op_count
                                    scan_ptr 2 add >scan_ptr
                                    0 >op_index
                                    while
                                        op_index op_count lt
                                        stream_valid 1 eq or
                                        if
                                            stream_end >stream_operation_limit
                                            scan_ptr validate_v2_operation >scan_ptr
                                            scan_ptr 0 lt if 0 >stream_valid end
                                            op_index 1 add >op_index
                                        end
                                        op_index op_count lt
                                        stream_valid 1 eq or
                                    end
                                    frame_index 1 add >frame_index
                                else 0 >stream_valid end
                            end
                            frame_index frame_count lt
                            stream_valid 1 eq or
                        end
                        1 >section_valid
                        stream_valid 1 neq if 0 >section_valid end
                        scan_ptr stream_end neq if 0 >section_valid end
                        section_valid 1 eq if
                            STREAM_BASE stream_v2_frame_start add >stream_frame_start
                            stream_frame_start >stream_frame_cursor
                            0 >stream_frame_index
                        else
                            0 >stream_valid
                        end
                    else
                        0 >stream_valid
                    end
                else
                    0 >stream_valid
                end
            end
        end
    end
;

fn apply_v2_operation (ptr) :
    local opcode 0
    local target 0
    local address 0
    local value 0
    local length 0
    local pool_id 0
    local source 0
    local pool_ptr 0
    local payload 0
    local index 0
    local destination 0
    local packed_value 0
    ptr ldb >opcode
    ptr 1 add ldb >target
    ptr 2 add read_u24 >address
    opcode 0 eq if
        ptr 5 add ldb >value
        target address value write_target
        ptr 6 add ret
    end
    target 2 eq if PCMRAM_BASE >destination
    else target 0 eq if SGU0_BASE >destination else SGU1_BASE >destination end end
    destination address add >destination
    opcode 1 eq if
        ptr 5 add read_u32 >length
        ptr 9 add ldb >value
        value 0x01010101 mul >packed_value
        0 >index
        # Bulk memory operations never touch control registers. Store four
        # words per iteration so clearing the 1 MiB PCM RAM completes at boot.
        while
            index 16 add length le if
                packed_value destination index add st
                packed_value destination index add 4 add st
                packed_value destination index add 8 add st
                packed_value destination index add 12 add st
                index 16 add >index
            end
            index 16 add length le
        end
        while
            index length lt if
                value destination index add stb
                index 1 add >index
            end
            index length lt
        end
        ptr 10 add ret
    end
    ptr 5 add read_u16 >pool_id
    STREAM_BASE stream_v2_directory add pool_id 16 mul add >pool_ptr
    pool_ptr 12 add read_u32 STREAM_BASE add >payload
    ptr 7 add read_u32 >source
    ptr 11 add read_u32 >length
    payload source add >source
    0 >index
    while
        index 4 add length le if
            source index add ld destination index add st
            index 4 add >index
        end
        index 4 add length le
    end
    while
        index length lt
        if
            source index add ldb destination index add stb
            index 1 add >index
        end
        index length lt
    end
    ptr 15 add ret
;

fn stop_all () :
    local voice 0
    local base 0
    while
        voice 16 lt
        if
            voice 8 lt if
                SGU0_REGS voice 8 mod 0x20 mul add >base
            else
                SGU1_REGS voice 8 mod 0x20 mul add >base
            end
            0 base 3 add stb
            0 base 0x19 add stb
            voice 1 add >voice
        end
        voice 16 lt
    end
;

# Validate header, every initial record, and every frame before any packet is run.
fn validate_stream () :
    local initial_ptr 0
    local initial_end 0
    local scan_ptr 0
    local frame_index 0
    local record_index 0
    local record_count 0
    local records_ptr 0
    local remaining 0
    STREAM_BASE 4 add ldb 2 eq if
        validate_stream_v2
    else
    1 >stream_version
    0 >stream_valid
    STREAM_BASE ldb 0x53 eq
    STREAM_BASE 1 add ldb 0x47 eq or
    STREAM_BASE 2 add ldb 0x55 eq or
    STREAM_BASE 3 add ldb 0x42 eq or
    STREAM_BASE 4 add ldb 1 eq or
    STREAM_BASE 5 add ldb 16 eq or
    STREAM_BASE 6 add ldb 1 le or
    STREAM_BASE 7 add ldb 0 eq or
    if
        STREAM_BASE 8 add read_u32 >stream_frame_count
        STREAM_BASE 12 add read_u32 >stream_initial_count
        STREAM_BASE 16 add read_u32 >stream_total_size
        stream_total_size 20 ge
        stream_total_size STREAM_LIMIT le or
        stream_frame_count 0 gt or
        stream_frame_count stream_total_size 2 div le or
        stream_initial_count 0 ge or
        if
            STREAM_BASE stream_total_size add >stream_end
            stream_end MAIN_RAM_END le
            if
                1 >stream_valid
                STREAM_BASE 20 add >initial_ptr
                stream_total_size 20 sub 5 div >remaining
                stream_initial_count remaining le
                if
                    initial_ptr stream_initial_count 5 mul add >initial_end
                    initial_ptr >scan_ptr
                    while
                        scan_ptr initial_end lt
                        stream_valid 1 eq or
                        if
                            scan_ptr record_valid
                            if
                                scan_ptr 5 add >scan_ptr
                            else
                                0 >stream_valid
                            end
                        end
                        scan_ptr initial_end lt
                        stream_valid 1 eq or
                    end
                    initial_end >scan_ptr
                    0 >frame_index
                    while
                        frame_index stream_frame_count lt
                        stream_valid 1 eq or
                        if
                            scan_ptr 2 add stream_end le
                            if
                                scan_ptr read_u16 >record_count
                                scan_ptr 2 add >records_ptr
                                stream_end records_ptr sub 5 div >remaining
                                record_count remaining le
                                if
                                    records_ptr >scan_ptr
                                    0 >record_index
                                    while
                                        record_index record_count lt
                                        stream_valid 1 eq or
                                        if
                                            scan_ptr record_valid
                                            if
                                                scan_ptr 5 add >scan_ptr
                                                record_index 1 add >record_index
                                            else
                                                0 >stream_valid
                                            end
                                        end
                                        record_index record_count lt
                                        stream_valid 1 eq or
                                    end
                                    frame_index 1 add >frame_index
                                else
                                    0 >stream_valid
                                end
                            else
                                0 >stream_valid
                            end
                        end
                        frame_index stream_frame_count lt
                        stream_valid 1 eq or
                    end
                    stream_valid 1 eq
                    scan_ptr stream_end eq or
                    if
                        STREAM_BASE 6 add ldb 1 and >stream_repeat
                        initial_end >stream_frame_start
                        stream_frame_start >stream_frame_cursor
                        0 >stream_frame_index
                    else
                        0 >stream_valid
                    end
                else
                    0 >stream_valid
                end
            else
                0 >stream_valid
            end
        end
    end

    stream_valid 1 eq if
        initial_ptr >scan_ptr
        while
            scan_ptr initial_end lt
            if
                scan_ptr apply_record
                scan_ptr 5 add >scan_ptr
            end
            scan_ptr initial_end lt
        end
    end
    end
;

fn irq0 () :
    1 IRQC_PENDING sth
    local record_count 0
    local records_ptr 0
    local record_index 0
    stream_valid 1 eq if
        stream_frame_index stream_frame_count ge if
            stream_repeat 1 eq if
                0 >stream_frame_index
                stream_frame_start >stream_frame_cursor
            else
                stop_all
                0 IRQC_ENABLE sth
                retn
            end
        end
        stream_version 2 eq if
            stream_frame_cursor read_u16 >record_count
            stream_frame_cursor 2 add >records_ptr
            0 >record_index
            while
                record_index record_count lt
                if
                    records_ptr apply_v2_operation >records_ptr
                    record_index 1 add >record_index
                end
                record_index record_count lt
            end
            records_ptr >stream_frame_cursor
        else
            stream_frame_cursor read_u16 >record_count
            stream_frame_cursor 2 add >records_ptr
            0 >record_index
            while
                record_index record_count lt
                if
                    records_ptr apply_record
                    records_ptr 5 add >records_ptr
                    record_index 1 add >record_index
                end
                record_index record_count lt
            end
            records_ptr >stream_frame_cursor
        end
        stream_frame_index 1 add >stream_frame_index
    else
        stop_all
        0 IRQC_ENABLE sth
    end
    1 IRQC_PENDING sth
    retn
;

fn main () :
    local record_index 0
    validate_stream
    stream_valid 1 eq if
        stream_version 2 eq if
            STREAM_BASE stream_v2_init_start add >stream_frame_cursor
            0 >record_index
            while
                record_index stream_v2_init_count lt
                if
                    stream_frame_cursor apply_v2_operation >stream_frame_cursor
                    record_index 1 add >record_index
                end
                record_index stream_v2_init_count lt
            end
            STREAM_BASE stream_v2_frame_start add >stream_frame_start
            stream_frame_start >stream_frame_cursor
        end
        1 IRQC_PENDING sth
        1 IRQC_ENABLE sth
        while
            halt
            0
        end
    else
        stop_all
        halt
    end
;

main
