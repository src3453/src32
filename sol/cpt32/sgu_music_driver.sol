# SGUB v1 player. The stream is loaded at the fixed 2 MiB main-RAM address.
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
        stream_frame_index 1 add >stream_frame_index
    else
        stop_all
        0 IRQC_ENABLE sth
    end
    1 IRQC_PENDING sth
    retn
;

fn main () :
    validate_stream
    stream_valid 1 eq if
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
