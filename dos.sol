# CPT32 DOS - a small DSFS / floppy shell for the CPT32 fantasy console.
#
# Features: file list, create, rename, delete, cross-drive copy, a line
# editor, a hex editor, DSFS format and free-space reporting. Text is
# rendered through the PCG console (sol/cpt32/pcg.sol) and the keyboard is
# read from the PeC event FIFO with IRQC source 3: the shell HALT s until
# the PeC input IRQ arrives (fn irq3), then drains the FIFO.
#
# Build (from the repository root):
#   python3 tools/solc/solc.py compile dos.sol -o dos.sol.a
#   python3 tools/asm/asm.py dos.sol.a -o dos.bin
#
# Run it with a DSFS volume in a disk slot, e.g.
#   python3 tools/dsfs/dsfs.py mkfs disk0.img --sectors 2880
#   python3 tools/dsfs/dsfs.py put disk0.img HELLO.TXT hello.txt
# then boot dos.bin (src32_testbench run from a directory whose config.yaml
# has no `bios` section, or the GUI with the image attached to slot 0).
#
# Commands (case insensitive, NAME.EXT or N:NAME.EXT for drive N):
#   DIR                 list files with size and block count
#   TYPE NAME           print a file as text
#   RUN NAME            load and execute a WAX v2 program
#   NEW NAME            create an empty file
#   REN OLD NEW         rename (name only, timestamps kept)
#   DEL NAME            delete a file
#   COPY SRC DST        copy between drives (COPY 0:A.TXT 1:A.TXT)
#   EDIT NAME           line editor: L list, <n> text replace, I n text insert,
#                       D n delete, A text append, W save, Q quit
#   HEX NAME            hex editor (N/P/G/E/W/Q)
#   FORMAT [N]          format drive N as DSFS
#   DRV N               select the current drive
#   MODE 40 / MODE 80   select text width
#   CLS / HELP / EXIT
#
# Buffers live in main RAM: the DSFS directory needs 8192 bytes and file
# operations are limited to 64 KiB per file (DOS_MAX_FILE).
#
# Modules: every function lives in a sol namespace named after its section
# banner, and calls that cross a namespace are written qualified so the
# module boundary is visible at the call site.
#   con    console output         kbd   keyboard input
#   names  names and words        disk  DSFS access
#   cmds   typed commands         edit  line editor
#   hex    hex editor             shell REPL and dispatch
#   wax    WAX validation/loading
# irq3 and main stay in the global namespace: solc only builds the IRQ
# dispatcher and the entry call for globally named functions.

!include "sol/cpt32/pcg.sol"
!include "sol/cpt32/idc.sol"
!include "sol/cpt32/dsfs.sol"
!include "sol/cpt32/wax.sol"
!include "sol/cpt32/irqc.sol"

!rodata_base 0x00040000

# PeC input / RTC and the VDP display enable bit.
!const DOS_PEC_DATA 0x80040030
!const DOS_PEC_STATUS 0x80040034
!const DOS_PEC_POP 0x80040038
!const DOS_RTC_SECONDS 0x80040208
!const DOS_VDP_ENABLE 0x80000000

# Scratch and file buffers in main RAM.
!const DOS_DIR_BUF 0x00300000
!const DOS_BM_BUF 0x00302000
!const DOS_FILE_BUF 0x00310000
!const DOS_EDIT_IDX 0x00320000
!const DOS_EDIT_TMP 0x00330000
!const DOS_CMD_BUF 0x00200000
!const DOS_WORD_BUF 0x00200100
!const DOS_NAME_A 0x00200200
!const DOS_NAME_B 0x00200210
!const DOS_HEX_RAW 0x00200220
!const DOS_HEX_TXT 0x00200230
!const DOS_EDIT_RAW 0x00200240
!const DOS_EDIT_TXT 0x00200250
!const DOS_TMP_TXT 0x00200260
!const DOS_MSGBUF 0x00200280

!const DOS_CMD_CAP 120
!const DOS_WORD_CAP 32
!const DOS_MAX_WORDS 5
!const DOS_MAX_LINES 1024
!const DOS_MAX_FILE 65536
!const DOS_EDIT_MAX 60000
!const DOS_TEXT_ROWS 2400

# Current drive and input state.
!var dos_drv 0
!var dos_caps 0
!var dos_shift 0
!var dos_spec_drv 0
!var dos_quit 0
!var dos_key 0
# Editor message pending display (0 = none); shown under the editor header
# so that errors and save results survive the next screen clear.
!var dos_msg 0
# File sizes / editor state.
!var dos_text_len 0
!var dos_lines 0
!var dos_hex_len 0
!var dos_hex_pos 0
!var dos_hex_dirty 0

# ---------------- IRQ ----------------

# IRQC source 3 is the PeC input FIFO. Acknowledge the latched IRQC
# pending bit so the next event latches a fresh rising edge; the shell
# drains the FIFO after IRET, which deasserts the source level.
fn irq3 () :
    8 irqc.IRQC_PENDING sth
    retn
;

# ---------------- console ----------------
!namespace con -

# Clear the whole 80x30 PCG window and home the cursor.
fn dos_cls () :
    local i 0
    0 >pcg.pcg_scroll_y
    0 >pcg.pcg_vram_row
    0 0x8000f00e stb
    0 0x8000f00f stb
    while
        i DOS_TEXT_ROWS lt
        if
            32 0x10000000 i add stb
            0x3f 0x10001000 i add stb
            0 0x10002000 i add stb
            i 1 add >i
            0
        else
            1
        end
    end
    0 0 pcg.locate
;

fn dos_newline () :
    10 dos_putc
;

# Console character output: CR is dropped, LF scrolls, BS erases.
fn dos_putc (ch) :
    ch 13 eq if retn end
    ch 10 eq if pcg._pcg_ln retn end
    ch 8 eq if
        pcg.pcg_cur_x 0 gt if
            pcg.pcg_cur_x 1 sub pcg.pcg_cur_y pcg.locate
            32 pcg.putcscr
            pcg.pcg_cur_x 1 sub pcg.pcg_cur_y pcg.locate
        end
        retn
    end
    ch pcg.putcscr
;

fn dos_puts (ptr) :
    local i 0
    local ch 0
    while
        ptr i add ldb >ch
        ch 0 neq
        if
            ch dos_putc
            i 1 add >i
            0
        else
            1
        end
    end
;

fn dos_putsln (ptr) :
    ptr dos_puts
    dos_newline
;

fn dos_pad (n) :
    local i 0
    while
        i n lt
        if
            32 dos_putc
            i 1 add >i
            0
        else
            1
        end
    end
;

fn dos_putnum (number) :
    local value 0
    number >value
    value 0 lt if
        45 dos_putc
        value neg >value
    end
    value 10 ge if
        value 10 div dos_putnum
    end
    value 10 mod 48 add dos_putc
;

# Decimal text of `value` at `dest`; returns the number of bytes written.
fn dos_num_text (value dest) :
    local v 0
    local n 0
    value >v
    v 0 lt if
        45 dest stb
        v neg dest 1 add dos_num_text 1 add ret
    end
    v 10 ge if
        v 10 div dest dos_num_text >n
        v 10 mod 48 add dest n add stb
        n 1 add ret
    end
    v 48 add dest stb
    1 ret
;

fn dos_hexdigit (v) :
    v 10 lt if
        v 48 add ret
    end
    v 55 add ret
;

# Print the low `digits` nibbles of `v` in uppercase hex.
fn dos_puthex (v digits) :
    local i 0
    local x 0
    v >x
    digits 1 sub >i
    while
        i 0 ge
        if
            x i 4 mul shr 15 and dos_hexdigit dos_putc
            i 1 sub >i
            0
        else
            1
        end
    end
;

fn dos_err (ptr) :
    "ERROR: " dos_puts
    ptr dos_putsln
;

# Paged output: print the marker and wait for a key.
fn dos_more () :
    "  -- MORE --" dos_puts
    kbd.dos_waitkey drop
    dos_newline
;

fn dos_strlen (ptr) :
    local i 0
    while
        ptr i add ldb 0 neq
        if
            i 1 add >i
            0
        else
            1
        end
    end
    i ret
;

# Copy the NUL-terminated string at src to dst; returns the length.
fn dos_strcpy (src dst) :
    local i 0
    local ch 0
    while
        src i add ldb >ch
        ch 0 neq
        if
            ch dst i add stb
            i 1 add >i
            0
        else
            1
        end
    end
    i ret
;

# 0 when the NUL-terminated strings are equal (sol truth).
fn dos_streq (a b) :
    local i 0
    local ca 0
    local cb 0
    while
        a i add ldb >ca
        b i add ldb >cb
        ca cb eq
        if
            ca 0 eq if 0 ret end
            i 1 add >i
            0
        else
            1
        end
    end
    1 ret
;

fn dos_memcpy (src dst len) :
    local i 0
    while
        i len lt
        if
            src i add ldb dst i add stb
            i 1 add >i
            0
        else
            1
        end
    end
;

fn dos_memzero (dst len) :
    local i 0
    while
        i len lt
        if
            0 dst i add stb
            i 1 add >i
            0
        else
            1
        end
    end
;

fn dos_memfill (dst len val) :
    local i 0
    while
        i len lt
        if
            val dst i add stb
            i 1 add >i
            0
        else
            1
        end
    end
;

fn dos_upper (ch) :
    local c 0
    ch >c
    c 97 ge c 122 le or if
        c 32 sub >c
    end
    c ret
;
!end

# ---------------- keyboard ----------------
!namespace kbd -

# Consume PeC keyboard events until Enter; echoes and returns the length.
# HALT s while the FIFO is empty so IRQ3 wakes the shell on the next event.
fn dos_read_line (dest cap) :
    local count 0
    local event 0
    local usage 0
    local value 0
    local ch 0
    0 >count
    while
        DOS_PEC_STATUS ld 1 and 0 gt
        if
            DOS_PEC_DATA ld >event
            event 28 shr 15 and 0 eq if
                event 24 shr 15 and 0 eq if
                    event 16 shr 255 and >usage
                    event 1 and >value
                    usage 225 eq usage 229 eq and if
                        value >dos_shift
                    else
                        usage 57 eq value 1 eq or if
                            dos_caps 0 eq if 1 >dos_caps else 0 >dos_caps end
                        else
                            value 1 eq if
                                usage dos_shift dos_caps dos_key_ascii >ch
                                ch 13 eq if
                                    1 DOS_PEC_POP st
                                    0 dest count add stb
                                    con.dos_newline
                                    count ret
                                end
                                ch 8 eq if
                                    count 0 gt if
                                        count 1 sub >count
                                        0 dest count add stb
                                        8 con.dos_putc
                                    end
                                else
                                    ch 0 neq count cap 1 sub lt or if
                                        ch dest count add stb
                                        count 1 add >count
                                        ch con.dos_putc
                                    end
                                end
                            end
                        end
                    end
                end
            end
            1 DOS_PEC_POP st
        else
            halt
        end
        0
    end
    count ret
;

# Wait for a key press without echoing; the usage id is in dos_key.
fn dos_waitkey () :
    local event 0
    local usage 0
    local value 0
    local done 0
    0 >done
    0 >dos_key
    while
        done 0 eq
        if
            DOS_PEC_STATUS ld 1 and 0 gt
            if
                DOS_PEC_DATA ld >event
                event 28 shr 15 and 0 eq if
                    event 24 shr 15 and 0 eq if
                        event 16 shr 255 and >usage
                        event 1 and >value
                        value 1 eq usage 0 neq or if
                            usage >dos_key
                            1 >done
                        end
                    end
                end
                1 DOS_PEC_POP st
                0
            else
                halt
                0
            end
        else
            1
        end
    end
    dos_key ret
;

# US keyboard HID usage -> ASCII (0 means the key is ignored).
fn dos_key_ascii (usage shifted caps) :
    local ch 0
    local upper 0
    usage 4 ge usage 29 le or if
        usage 97 add 4 sub >ch
        shifted caps xor >upper
        upper 0 neq if ch 32 sub >ch end
        ch ret
    end
    usage 30 ge usage 39 le or if
        usage 30 eq if 49 >ch end
        usage 31 eq if 50 >ch end
        usage 32 eq if 51 >ch end
        usage 33 eq if 52 >ch end
        usage 34 eq if 53 >ch end
        usage 35 eq if 54 >ch end
        usage 36 eq if 55 >ch end
        usage 37 eq if 56 >ch end
        usage 38 eq if 57 >ch end
        usage 39 eq if 48 >ch end
        shifted 0 neq if
            ch 49 eq if 33 >ch end
            ch 50 eq if 64 >ch end
            ch 51 eq if 35 >ch end
            ch 52 eq if 36 >ch end
            ch 53 eq if 37 >ch end
            ch 54 eq if 94 >ch end
            ch 55 eq if 38 >ch end
            ch 56 eq if 42 >ch end
            ch 57 eq if 40 >ch end
            ch 48 eq if 41 >ch end
        end
        ch ret
    end
    usage 40 eq if 13 ret end
    usage 42 eq if 8 ret end
    usage 44 eq if 32 ret end
    usage 45 eq if shifted 0 eq if 45 ret else 95 ret end end
    usage 46 eq if shifted 0 eq if 61 ret else 43 ret end end
    usage 47 eq if shifted 0 eq if 91 ret else 123 ret end end
    usage 48 eq if shifted 0 eq if 93 ret else 125 ret end end
    usage 49 eq if shifted 0 eq if 92 ret else 124 ret end end
    usage 51 eq if shifted 0 eq if 59 ret else 58 ret end end
    usage 52 eq if shifted 0 eq if 39 ret else 34 ret end end
    usage 53 eq if shifted 0 eq if 96 ret else 126 ret end end
    usage 54 eq if shifted 0 eq if 44 ret else 60 ret end end
    usage 55 eq if shifted 0 eq if 46 ret else 62 ret end end
    usage 56 eq if shifted 0 eq if 47 ret else 63 ret end end
    0 ret
;
!end

# ---------------- names and words ----------------
!namespace names -

fn dos_name_char_ok (ch) :
    local c 0
    ch >c
    c 65 ge c 90 le or if 0 ret end
    c 48 ge c 57 le or if 0 ret end
    c 95 eq if 0 ret end
    c 45 eq if 0 ret end
    1 ret
;

# "NAME.EXT" (or "NAME") -> 11 space-padded on-disk bytes. 0 = ok.
fn dos_name_build (src len dest) :
    local i 0
    local dot -1
    local ch 0
    local c 0
    local base_len 0
    local ext_len 0
    len 0 le if 1 ret end
    -1 >dot
    0 >i
    while
        i len lt
        if
            src i add ldb 46 eq if
                i >dot
            end
            i 1 add >i
            0
        else
            1
        end
    end
    len >base_len
    0 >ext_len
    dot 0 ge if
        dot >base_len
        len dot 1 add sub >ext_len
    end
    base_len 1 lt if 1 ret end
    base_len 8 gt if 1 ret end
    ext_len 3 gt if 1 ret end
    dest 11 32 con.dos_memfill
    0 >i
    while
        i base_len lt
        if
            src i add ldb >ch
            ch con.dos_upper >c
            c dos_name_char_ok 0 neq if 1 ret end
            c dest i add stb
            i 1 add >i
            0
        else
            1
        end
    end
    dot 0 ge if
        0 >i
        while
            i ext_len lt
            if
                src dot 1 add add i add ldb >ch
                ch con.dos_upper >c
                c dos_name_char_ok 0 neq if 1 ret end
                c dest 8 add i add stb
                i 1 add >i
                0
            else
                1
            end
        end
    end
    0 ret
;

# 11 on-disk name bytes -> "NAME.EXT" (NUL terminated) at dest.
fn dos_name_text (name dest) :
    local i 0
    local ch 0
    local out 0
    local has 0
    0 >out
    0 >i
    while
        i 8 lt
        if
            name i add ldb >ch
            ch 32 neq if
                ch dest out add stb
                out 1 add >out
            end
            i 1 add >i
            0
        else
            1
        end
    end
    0 >has
    0 >i
    while
        i 3 lt
        if
            name 8 add i add ldb 32 neq if
                1 >has
            end
            i 1 add >i
            0
        else
            1
        end
    end
    has 0 neq if
        46 dest out add stb
        out 1 add >out
        0 >i
        while
            i 3 lt
            if
                name 8 add i add ldb >ch
                ch 32 neq if
                    ch dest out add stb
                    out 1 add >out
                end
                i 1 add >i
                0
            else
                1
            end
        end
    end
    0 dest out add stb
;

# Typed word -> 11-byte name in DOS_NAME_A; 0 = ok.
fn dos_name_from_word (w) :
    local len 0
    w con.dos_strlen >len
    w len DOS_NAME_A dos_name_build ret
;

# "[N:]NAME.EXT" -> dos_spec_drv plus DOS_NAME_A; returns the name length
# or -1 when the name is malformed.
fn dos_parse_spec (src len) :
    local off 0
    local nlen 0
    local ch 0
    dos_drv >dos_spec_drv
    0 >off
    len 2 ge if
        src 1 add ldb 58 eq if
            src ldb >ch
            ch 48 ge ch 51 le or if
                ch 48 sub >dos_spec_drv
                2 >off
            else
                -1 ret
            end
        end
    end
    len off sub >nlen
    src off add nlen DOS_NAME_A dos_name_build 0 neq if -1 ret end
    nlen ret
;

# Split the command line into uppercase words in DOS_WORD_BUF; returns the
# word count (at most DOS_MAX_WORDS).
fn dos_words (ptr len) :
    local i 0
    local w 0
    local n 0
    local ch 0
    local started 0
    0 >n
    0 >w
    0 >started
    while
        i len lt
        if
            ptr i add ldb >ch
            ch 32 eq ch 9 eq and if
                started 0 neq if
                    0 DOS_WORD_BUF w DOS_WORD_CAP mul add n add stb
                    w 1 add >w
                    0 >n
                    0 >started
                end
            else
                w DOS_MAX_WORDS lt n 31 lt or if
                    ch con.dos_upper DOS_WORD_BUF w DOS_WORD_CAP mul add n add stb
                    n 1 add >n
                    1 >started
                end
            end
            i 1 add >i
            0
        else
            1
        end
    end
    started 0 neq if
        0 DOS_WORD_BUF w DOS_WORD_CAP mul add n add stb
        w 1 add >w
    end
    w ret
;

# Address of the remainder of the command line after `count` words.
fn dos_tail (ptr count) :
    local i 0
    local w 0
    local ch 0
    0 >i
    0 >w
    while
        w count lt
        if
            while
                ptr i add ldb 32 eq
                if
                    i 1 add >i
                    0
                else
                    1
                end
            end
            while
                ptr i add ldb 0 neq ptr i add ldb 32 neq or ptr i add ldb 9 neq or
                if
                    i 1 add >i
                    0
                else
                    1
                end
            end
            w 1 add >w
            0
        else
            1
        end
    end
    while
        ptr i add ldb 32 eq
        if
            i 1 add >i
            0
        else
            1
        end
    end
    ptr i add ret
;

fn dos_parse_dec (ptr) :
    local i 0
    local ch 0
    local v 0
    local n 0
    0 >v
    0 >n
    while
        ptr i add ldb >ch
        ch 0 neq
        if
            ch 48 lt ch 57 gt or if -1 ret end
            v 10 mul ch 48 sub add >v
            i 1 add >i
            n 1 add >n
            0
        else
            1
        end
    end
    n 0 eq if -1 ret end
    v ret
;

fn dos_hexval (ch) :
    local c 0
    ch con.dos_upper >c
    c 48 ge c 57 le or if c 48 sub ret end
    c 65 ge c 70 le or if c 55 sub ret end
    -1 ret
;

fn dos_parse_hex (ptr) :
    local i 0
    local ch 0
    local v 0
    local d 0
    local n 0
    0 >v
    0 >n
    while
        ptr i add ldb >ch
        ch 0 neq
        if
            ch dos_hexval >d
            d 0 lt if -1 ret end
            v 4 shl d add >v
            i 1 add >i
            n 1 add >n
            0
        else
            1
        end
    end
    n 0 eq if -1 ret end
    v ret
;
!end

# ---------------- disk ----------------
!namespace disk -

# Seconds since the Unix epoch from the PeC RTC (0 when it is not wired).
fn dos_time () :
    DOS_RTC_SECONDS ld ret
;

fn dos_mount (drv) :
    drv DOS_DIR_BUF dsfs.dsfs_mount ret
;

# 0 when the current drive is mounted; prints an error otherwise.
fn dos_ensure () :
    dos_drv dos_mount 0 eq if 0 ret end
    "DRIVE NOT MOUNTED - FORMAT IT FIRST" con.dos_err
    1 ret
;

# Size of `name` (11-byte on-disk form) in the current volume, or -1.
fn dos_file_size (name) :
    local idx 0
    DOS_DIR_BUF dsfs.dsfs_dir_load 0 neq if -1 ret end
    DOS_DIR_BUF name dsfs.dsfs_find >idx
    idx 0 lt if -1 ret end
    DOS_DIR_BUF idx 5 shl add 20 add ld ret
;
!end

# ---------------- commands ----------------
!namespace cmds -

fn dos_cmd_dir () :
    local i 0
    local addr 0
    local rows 0
    local size 0
    local digits 0
    local used 0
    disk.dos_ensure 0 neq if retn end
    DOS_DIR_BUF dsfs.dsfs_dir_load 0 neq if "DIRECTORY READ ERROR" con.dos_err retn end
    DOS_DIR_BUF dsfs.dsfs_count >used
    "DRIVE " con.dos_puts
    dos_drv con.dos_putnum
    "  " con.dos_puts
    dsfs.dsfs_total con.dos_putnum
    " BLOCKS  " con.dos_puts
    used con.dos_putnum
    " FILE(S)" con.dos_putsln
    "  #   NAME             SIZE    BLOCKS" con.dos_putsln
    1 >rows
    0 >i
    while
        i 256 lt
        if
            DOS_DIR_BUF i 5 shl add >addr
            addr ldb 1 eq if
                "  " con.dos_puts
                i 2 con.dos_puthex
                "  " con.dos_puts
                addr 1 add DOS_TMP_TXT names.dos_name_text
                DOS_TMP_TXT con.dos_puts
                DOS_TMP_TXT con.dos_strlen 17 lt if
                    17 DOS_TMP_TXT con.dos_strlen sub con.dos_pad
                end
                addr 20 add ld >size
                size DOS_TMP_TXT con.dos_num_text >digits
                0 DOS_TMP_TXT digits add stb
                DOS_TMP_TXT con.dos_puts
                8 digits sub con.dos_pad
                size 0 eq if
                    0 con.dos_putnum
                else
                    size 511 add 9 shr con.dos_putnum
                end
                con.dos_newline
                rows 1 add >rows
                rows 26 ge if
                    0 >rows
                    con.dos_more
                end
            end
            i 1 add >i
            0
        else
            1
        end
    end
;

fn dos_cmd_type (name) :
    local i 0
    local rows 0
    local size 0
    local ch 0
    disk.dos_ensure 0 neq if retn end
    name disk.dos_file_size >size
    size 0 lt if "NOT FOUND" con.dos_err retn end
    size DOS_MAX_FILE gt if "FILE TOO LARGE" con.dos_err retn end
    DOS_DIR_BUF name DOS_FILE_BUF dsfs.dsfs_read_file 0 neq if "READ ERROR" con.dos_err retn end
    0 >i
    0 >rows
    while
        i size lt
        if
            DOS_FILE_BUF i add ldb >ch
            ch 10 eq if
                con.dos_newline
                rows 1 add >rows
                rows 24 ge if
                    0 >rows
                    con.dos_more
                end
            else
                ch con.dos_putc
            end
            i 1 add >i
            0
        else
            1
        end
    end
    con.dos_newline
;
fn dos_cmd_run (name) :
    local size 0
    local err 0
    disk.dos_ensure 0 neq if retn end
    name disk.dos_file_size >size
    size 0 lt if "NOT FOUND" con.dos_err retn end
    size DOS_MAX_FILE gt if "FILE TOO LARGE" con.dos_err retn end
    DOS_DIR_BUF name DOS_FILE_BUF dsfs.dsfs_read_file 0 neq if "READ ERROR" con.dos_err retn end
    "STARTING " con.dos_puts
    name DOS_TMP_TXT names.dos_name_text
    DOS_TMP_TXT con.dos_putsln
    DOS_FILE_BUF size wax.wax_load >err
    err wax.WAX_ERR_HEADER eq if "INVALID WAX HEADER" con.dos_err retn end
    err wax.WAX_ERR_LAYOUT eq if "INVALID WAX LAYOUT" con.dos_err retn end
    err wax.WAX_ERR_RANGE eq if "INVALID WAX RANGE" con.dos_err retn end
    err wax.WAX_ERR_OVERLAP eq if "OVERLAPPING WAX DATA" con.dos_err retn end
    err wax.WAX_ERR_ENTRY eq if "INVALID WAX ENTRY" con.dos_err retn end
    err wax.WAX_ERR_RESERVED eq if "WAX USES RESERVED RAM" con.dos_err retn end
    err wax.WAX_ERR_BUFFER eq if "INVALID WAX BUFFER" con.dos_err retn end
    "WAX LOAD ERROR" con.dos_err
;

fn dos_cmd_new (name) :
    disk.dos_ensure 0 neq if retn end
    name disk.dos_file_size 0 ge if "ALREADY EXISTS" con.dos_err retn end
    disk.dos_time dsfs.dsfs_set_time
    DOS_DIR_BUF DOS_BM_BUF name DOS_FILE_BUF 0 dsfs.dsfs_write_file 0 neq if "CREATE FAILED" con.dos_err retn end
    "CREATED " con.dos_puts
    name DOS_TMP_TXT names.dos_name_text
    DOS_TMP_TXT con.dos_putsln
;

fn dos_cmd_delete (name) :
    disk.dos_ensure 0 neq if retn end
    name disk.dos_file_size 0 lt if "NOT FOUND" con.dos_err retn end
    DOS_DIR_BUF DOS_BM_BUF name dsfs.dsfs_delete_file 0 neq if "DELETE FAILED" con.dos_err retn end
    "DELETED " con.dos_puts
    name DOS_TMP_TXT names.dos_name_text
    DOS_TMP_TXT con.dos_putsln
;

fn dos_cmd_rename (old new) :
    local idx 0
    local addr 0
    local i 0
    disk.dos_ensure 0 neq if retn end
    DOS_DIR_BUF dsfs.dsfs_dir_load 0 neq if "DIRECTORY READ ERROR" con.dos_err retn end
    DOS_DIR_BUF old dsfs.dsfs_find >idx
    idx 0 lt if "NOT FOUND" con.dos_err retn end
    DOS_DIR_BUF new dsfs.dsfs_find 0 ge if "NAME ALREADY IN USE" con.dos_err retn end
    DOS_DIR_BUF idx 5 shl add >addr
    0 >i
    while
        i 11 lt
        if
            new i add ldb addr 1 add i add stb
            i 1 add >i
            0
        else
            1
        end
    end
    DOS_DIR_BUF idx dsfs.dsfs_dir_save_sector 0 neq if "WRITE ERROR" con.dos_err retn end
    "RENAMED " con.dos_puts
    old DOS_TMP_TXT names.dos_name_text
    DOS_TMP_TXT con.dos_puts
    " -> " con.dos_puts
    new DOS_TMP_TXT names.dos_name_text
    DOS_TMP_TXT con.dos_putsln
;

fn dos_cmd_copy (w1 w2) :
    local srclen 0
    local dstlen 0
    local sdrv 0
    local ddrv 0
    local idx 0
    local size 0
    local sectors 0
    local tail 0
    w1 con.dos_strlen >srclen
    w2 con.dos_strlen >dstlen
    w1 srclen names.dos_parse_spec >srclen
    srclen 0 lt if "BAD SOURCE NAME" con.dos_err retn end
    dos_spec_drv >sdrv
    DOS_NAME_A DOS_NAME_B 11 con.dos_memcpy
    w2 dstlen names.dos_parse_spec >dstlen
    dstlen 0 lt if "BAD DEST NAME" con.dos_err retn end
    dos_spec_drv >ddrv
    sdrv disk.dos_mount 0 neq if "SOURCE DRIVE ERROR" con.dos_err retn end
    DOS_DIR_BUF dsfs.dsfs_dir_load 0 neq if "SOURCE DIRECTORY ERROR" con.dos_err retn end
    DOS_DIR_BUF DOS_NAME_B dsfs.dsfs_find >idx
    idx 0 lt if "SOURCE NOT FOUND" con.dos_err retn end
    DOS_DIR_BUF idx 5 shl add 20 add ld >size
    size DOS_MAX_FILE gt if "FILE TOO LARGE" con.dos_err retn end
    DOS_DIR_BUF DOS_NAME_B DOS_FILE_BUF dsfs.dsfs_read_file 0 neq if "READ ERROR" con.dos_err retn end
    size 511 add 9 shr >sectors
    sectors 512 mul size sub >tail
    DOS_FILE_BUF size add tail con.dos_memzero
    ddrv disk.dos_mount 0 neq if "DEST DRIVE ERROR" con.dos_err retn end
    disk.dos_time dsfs.dsfs_set_time
    DOS_DIR_BUF DOS_BM_BUF DOS_NAME_A DOS_FILE_BUF size dsfs.dsfs_write_file 0 neq if "WRITE ERROR" con.dos_err retn end
    "COPIED " con.dos_puts
    size con.dos_putnum
    " BYTES" con.dos_putsln
;

# DSFS format (doc/spec_DSFS.md §2/§8) written by the guest itself.
fn dos_cmd_format (drv) :
    local total 0
    local bb 0
    local ds 0
    local i 0
    drv idc.idc_identify >total
    total 0 lt if "NO MEDIA IN DRIVE" con.dos_err retn end
    total 19 lt if "DEVICE TOO SMALL FOR DSFS" con.dos_err retn end
    total 32768 gt if "VOLUME TOO LARGE (16MB MAX)" con.dos_err retn end
    total 4095 add 12 shr >bb
    bb 1 neq if "VOLUME NEEDS A LARGER BITMAP" con.dos_err retn end
    bb 2 add 16 add >ds
    # Boot sector: zeroed code area plus the 0x55AA signature.
    DOS_DIR_BUF 512 con.dos_memzero
    0x55 DOS_DIR_BUF 510 add stb
    0xAA DOS_DIR_BUF 511 add stb
    drv 0 1 DOS_DIR_BUF idc.idc_write 0 neq if "WRITE ERROR (BOOT)" con.dos_err retn end
    # Superblock.
    DOS_DIR_BUF 512 con.dos_memzero
    0x44534653 DOS_DIR_BUF st
    1 DOS_DIR_BUF 4 add sth
    0 DOS_DIR_BUF 6 add sth
    total DOS_DIR_BUF 8 add st
    2 DOS_DIR_BUF 12 add st
    bb DOS_DIR_BUF 16 add st
    bb 2 add DOS_DIR_BUF 20 add st
    16 DOS_DIR_BUF 24 add st
    ds DOS_DIR_BUF 28 add st
    disk.dos_time DOS_DIR_BUF 32 add st
    drv 1 1 DOS_DIR_BUF idc.idc_write 0 neq if "WRITE ERROR (SUPER)" con.dos_err retn end
    # Bitmap: metadata blocks 0..ds-1 marked used, the rest free.
    DOS_BM_BUF 512 con.dos_memzero
    0 >i
    while
        i ds lt
        if
            DOS_BM_BUF i 3 shr add ldb 128 i 7 and shr or DOS_BM_BUF i 3 shr add stb
            i 1 add >i
            0
        else
            1
        end
    end
    drv 2 bb DOS_BM_BUF idc.idc_write 0 neq if "WRITE ERROR (BITMAP)" con.dos_err retn end
    # Empty directory.
    DOS_DIR_BUF 8192 con.dos_memzero
    drv bb 2 add 16 DOS_DIR_BUF idc.idc_write 0 neq if "WRITE ERROR (DIR)" con.dos_err retn end
    "FORMATTED " con.dos_puts
    total con.dos_putnum
    " BLOCKS, DATA AT " con.dos_puts
    ds con.dos_putnum
    " FREE " con.dos_puts
    total ds sub con.dos_putnum
    " BLOCKS" con.dos_putsln
;

fn dos_cmd_free () :
    local i 0
    local used 0
    local entries 0
    disk.dos_ensure 0 neq if retn end
    DOS_BM_BUF dsfs.dsfs_bitmap_load 0 neq if "BITMAP READ ERROR" con.dos_err retn end
    0 >used
    dsfs.dsfs_data_start >i
    while
        i dsfs.dsfs_total lt
        if
            i dsfs.dsfs_bm_get 0 neq if
                used 1 add >used
            end
            i 1 add >i
            0
        else
            1
        end
    end
    DOS_DIR_BUF dsfs.dsfs_dir_load drop
    DOS_DIR_BUF dsfs.dsfs_count >entries
    "TOTAL BLOCKS  " con.dos_puts
    dsfs.dsfs_total con.dos_putnum con.dos_newline
    "DATA BLOCKS   " con.dos_puts
    dsfs.dsfs_total dsfs.dsfs_data_start sub con.dos_putnum con.dos_newline
    "USED BLOCKS   " con.dos_puts
    used con.dos_putnum con.dos_newline
    "FREE BLOCKS   " con.dos_puts
    dsfs.dsfs_total dsfs.dsfs_data_start sub used sub con.dos_putnum con.dos_newline
    "FILES         " con.dos_puts
    entries con.dos_putnum con.dos_newline
;

fn dos_cmd_help () :
    "DIR              list files" con.dos_putsln
    "TYPE NAME        print a text file" con.dos_putsln
    "RUN NAME         load and execute a WAX v2 program" con.dos_putsln
    "NEW NAME         create an empty file" con.dos_putsln
    "REN OLD NEW      rename a file" con.dos_putsln
    "DEL NAME         delete a file" con.dos_putsln
    "COPY SRC DST     copy between drives (0:NAME.TXT)" con.dos_putsln
    "EDIT NAME        line editor" con.dos_putsln
    "HEX NAME         hex editor" con.dos_putsln
    "FORMAT [N]       format drive N as DSFS" con.dos_putsln
    "DRV N            select the current drive" con.dos_putsln
    "MODE 40 / 80     set text width" con.dos_putsln
    "FREE             block and file statistics" con.dos_putsln
    "CLS / HELP / EXIT" con.dos_putsln
;

fn dos_cmd_exit () :
    "BYE" con.dos_putsln
    halt
;
!end

# ---------------- line editor ----------------
!namespace edit -

# Queue an editor message; it is printed under the header on the next redraw
# (a direct print would be wiped by the redraw in the same loop iteration).
fn dos_msg_err (msg) :
    "ERROR: " DOS_MSGBUF con.dos_strcpy drop
    msg DOS_MSGBUF 7 add con.dos_strcpy drop
    1 >dos_msg
;

# Queue plain text for the next interactive redraw.
fn dos_msg_text (ptr) :
    ptr DOS_MSGBUF con.dos_strcpy drop
    1 >dos_msg
;

# Rebuild the line index from the NUL-separated text in DOS_FILE_BUF.
fn dos_edit_scan () :
    local i 0
    local n 0
    local start 0
    0 >n
    0 >start
    0 >i
    while
        i dos_text_len lt
        if
            DOS_FILE_BUF i add ldb 10 eq if
                0 DOS_FILE_BUF i add stb
                n DOS_MAX_LINES lt if
                    start DOS_EDIT_IDX n 2 shl add st
                    n 1 add >n
                end
                i 1 add >start
            end
            i 1 add >i
            0
        else
            1
        end
    end
    start dos_text_len lt if
        n DOS_MAX_LINES lt if
            start DOS_EDIT_IDX n 2 shl add st
            n 1 add >n
        end
    end
    n >dos_lines
;

# Build the on-disk text ('\n' separated) in DOS_EDIT_TMP; returns its length.
fn dos_edit_render () :
    local i 0
    local off 0
    local o 0
    local ch 0
    0 >o
    0 >i
    while
        i dos_lines lt
        if
            DOS_EDIT_IDX i 2 shl add ld >off
            while
                DOS_FILE_BUF off add ldb >ch
                ch 0 neq
                if
                    ch DOS_EDIT_TMP o add stb
                    o 1 add >o
                    off 1 add >off
                    0
                else
                    1
                end
            end
            10 DOS_EDIT_TMP o add stb
            o 1 add >o
            i 1 add >i
            0
        else
            1
        end
    end
    o ret
;

# Rebuild the text with line `index` replaced (mode 0), inserted before
# (mode 1) or deleted (mode 2); `ptr` is the replacement text.
fn dos_edit_splice (index mode ptr) :
    local i 0
    local off 0
    local o 0
    local ch 0
    0 >o
    0 >i
    while
        i dos_lines le
        if
            # The replacement text: written in place of line `index` (mode 0)
            # or before it (mode 1); mode 2 deletes without text.
            i index eq if
                mode 2 neq if
                    ptr DOS_EDIT_TMP o add con.dos_strcpy o add >o
                    10 DOS_EDIT_TMP o add stb
                    o 1 add >o
                end
            end
            i dos_lines lt if
                # Keep the old line unless it is replaced (mode 0) or
                # deleted (mode 2); mode 1 inserts before it.
                i index neq mode 1 eq and if
                    DOS_EDIT_IDX i 2 shl add ld >off
                    while
                        DOS_FILE_BUF off add ldb >ch
                        ch 0 neq
                        if
                            ch DOS_EDIT_TMP o add stb
                            o 1 add >o
                            off 1 add >off
                            0
                        else
                            1
                        end
                    end
                    10 DOS_EDIT_TMP o add stb
                    o 1 add >o
                end
            end
            i 1 add >i
            0
        else
            1
        end
    end
    DOS_EDIT_TMP DOS_FILE_BUF o con.dos_memcpy
    o >dos_text_len
    dos_edit_scan
;

# Load the file into the editor text buffer (missing file = empty text).
fn dos_edit_load () :
    local idx 0
    local size 0
    DOS_DIR_BUF dsfs.dsfs_dir_load 0 neq if 1 ret end
    DOS_DIR_BUF DOS_EDIT_RAW dsfs.dsfs_find >idx
    idx 0 lt if
        0 >dos_text_len
        dos_edit_scan
        0 ret
    end
    DOS_DIR_BUF idx 5 shl add 20 add ld >size
    size DOS_EDIT_MAX gt if 2 ret end
    DOS_DIR_BUF DOS_EDIT_RAW DOS_FILE_BUF dsfs.dsfs_read_file 0 neq if 3 ret end
    size >dos_text_len
    dos_edit_scan
    0 ret
;

fn dos_edit_show () :
    local i 0
    local off 0
    local rows 0
    con.dos_cls
    "LINE EDITOR  " con.dos_puts
    DOS_EDIT_TXT con.dos_puts
    "  " con.dos_puts
    dos_lines con.dos_putnum
    " LINE(S)" con.dos_putsln
    "N text=replace  I N text=insert  D N=delete  A text=append" con.dos_putsln
    "L=list  W=save  Q=quit" con.dos_putsln
    dos_msg 0 neq if
        DOS_MSGBUF con.dos_putsln
        0 >dos_msg
    end
    0 >rows
    0 >i
    while
        i dos_lines lt
        if
            rows 21 ge if
                0 >rows
                con.dos_more
            end
            DOS_EDIT_IDX i 2 shl add ld >off
            i 1 add con.dos_putnum
            "  " con.dos_puts
            DOS_FILE_BUF off add con.dos_putsln
            rows 1 add >rows
            i 1 add >i
            0
        else
            1
        end
    end
;

fn dos_edit_save_cmd () :
    local len 0
    local sectors 0
    local tail 0
    local off 0
    dos_edit_render >len
    len 511 add 9 shr >sectors
    sectors 512 mul len sub >tail
    DOS_EDIT_TMP len add tail con.dos_memzero
    disk.dos_time dsfs.dsfs_set_time
    DOS_DIR_BUF DOS_BM_BUF DOS_EDIT_RAW DOS_EDIT_TMP len dsfs.dsfs_write_file 0 neq if "SAVE FAILED" dos_msg_err retn end
    "SAVED " DOS_MSGBUF con.dos_strcpy >off
    len DOS_MSGBUF off add con.dos_num_text off add >off
    " BYTES" DOS_MSGBUF off add con.dos_strcpy drop
    1 >dos_msg
;

fn dos_edit_list () :
    local i 0
    local off 0
    local rows 0
    0 >rows
    0 >i
    while
        i dos_lines lt
        if
            rows 22 ge if
                0 >rows
                con.dos_more
            end
            DOS_EDIT_IDX i 2 shl add ld >off
            i 1 add con.dos_putnum
            "  " con.dos_puts
            DOS_FILE_BUF off add con.dos_putsln
            rows 1 add >rows
            i 1 add >i
            0
        else
            1
        end
    end
;

fn dos_edit_dispatch (ptr n) :
    local w0 0
    local num 0
    n 0 eq if retn end
    DOS_WORD_BUF >w0
    w0 "L" con.dos_streq if dos_edit_list retn end
    w0 "LIST" con.dos_streq if dos_edit_list retn end
    w0 "W" con.dos_streq if dos_edit_save_cmd retn end
    w0 "SAVE" con.dos_streq if dos_edit_save_cmd retn end
    w0 "Q" con.dos_streq if 1 >dos_quit retn end
    w0 "QUIT" con.dos_streq if 1 >dos_quit retn end
    w0 "A" con.dos_streq if dos_lines 1 ptr 1 names.dos_tail dos_edit_splice retn end
    w0 "APPEND" con.dos_streq if dos_lines 1 ptr 1 names.dos_tail dos_edit_splice retn end
    w0 "I" con.dos_streq if
        n 2 lt if "USAGE: I N TEXT" dos_msg_err retn end
        DOS_WORD_BUF 32 add names.dos_parse_dec >num
        num 1 lt if "BAD LINE NUMBER" dos_msg_err retn end
        num 1 sub >num
        num dos_lines gt if "LINE NUMBER TOO LARGE" dos_msg_err retn end
        num 1 ptr 2 names.dos_tail dos_edit_splice
        retn
    end
    w0 "INSERT" con.dos_streq if
        n 2 lt if "USAGE: INSERT N TEXT" dos_msg_err retn end
        DOS_WORD_BUF 32 add names.dos_parse_dec >num
        num 1 lt if "BAD LINE NUMBER" dos_msg_err retn end
        num 1 sub >num
        num dos_lines gt if "LINE NUMBER TOO LARGE" dos_msg_err retn end
        num 1 ptr 2 names.dos_tail dos_edit_splice
        retn
    end
    w0 "D" con.dos_streq if
        n 2 lt if "USAGE: D N" dos_msg_err retn end
        DOS_WORD_BUF 32 add names.dos_parse_dec >num
        num 1 lt if "BAD LINE NUMBER" dos_msg_err retn end
        num dos_lines gt if "LINE NUMBER TOO LARGE" dos_msg_err retn end
        num 1 sub 2 0 dos_edit_splice
        retn
    end
    w0 "DELETE" con.dos_streq if
        n 2 lt if "USAGE: DELETE N" dos_msg_err retn end
        DOS_WORD_BUF 32 add names.dos_parse_dec >num
        num 1 lt if "BAD LINE NUMBER" dos_msg_err retn end
        num dos_lines gt if "LINE NUMBER TOO LARGE" dos_msg_err retn end
        num 1 sub 2 0 dos_edit_splice
        retn
    end
    w0 names.dos_parse_dec >num
    num 0 ge if
        num 1 lt if "BAD LINE NUMBER" dos_msg_err retn end
        num dos_lines gt if "LINE NUMBER TOO LARGE" dos_msg_err retn end
        num 1 sub 0 ptr 1 names.dos_tail dos_edit_splice
        retn
    end
    "UNKNOWN EDITOR COMMAND" dos_msg_err
;

fn dos_line_editor () :
    local n 0
    local w 0
    0 >dos_quit
    while
        dos_quit 0 eq
        if
            dos_edit_show
            ":" con.dos_puts
            DOS_CMD_BUF DOS_CMD_CAP kbd.dos_read_line >n
            DOS_CMD_BUF n names.dos_words >w
            DOS_CMD_BUF w dos_edit_dispatch
            0
        else
            1
        end
    end
;

fn dos_cmd_edit (name) :
    name DOS_EDIT_RAW 11 con.dos_memcpy
    DOS_EDIT_RAW DOS_EDIT_TXT names.dos_name_text
    disk.dos_ensure 0 neq if retn end
    dos_edit_load 0 neq if "CANNOT LOAD FILE FOR EDITING" con.dos_err retn end
    dos_line_editor
    "EDITOR CLOSED" con.dos_putsln
;
!end

# ---------------- hex editor ----------------
!namespace hex -

fn dos_hex_show () :
    local row 0
    local col 0
    local addr 0
    local b 0
    con.dos_cls
    "HEX EDITOR  " con.dos_puts
    DOS_HEX_TXT con.dos_puts
    "  " con.dos_puts
    dos_hex_len con.dos_putnum
    " BYTES  OFFSET " con.dos_puts
    dos_hex_pos 4 con.dos_puthex
    con.dos_newline
    "N=next P=prev G addr E addr v.. W=save Q=quit" con.dos_putsln
    dos_msg 0 neq if
        DOS_MSGBUF con.dos_putsln
        0 >dos_msg
    end
    "      " con.dos_puts
    "00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F" con.dos_putsln
    0 >row
    while
        row 16 lt
        if
            dos_hex_pos row 16 mul add >addr
            addr 4 con.dos_puthex
            ": " con.dos_puts
            0 >col
            while
                col 16 lt
                if
                    addr col add dos_hex_len lt if
                        addr col add DOS_FILE_BUF add ldb >b
                        b 2 con.dos_puthex
                    else
                        "  " con.dos_puts
                    end
                    " " con.dos_puts
                    col 1 add >col
                    0
                else
                    1
                end
            end
            "|" con.dos_puts
            0 >col
            while
                col 16 lt
                if
                    addr col add dos_hex_len lt if
                        addr col add DOS_FILE_BUF add ldb >b
                        b 32 ge b 126 le or if
                            b con.dos_putc
                        else
                            46 con.dos_putc
                        end
                    else
                        32 con.dos_putc
                    end
                    col 1 add >col
                    0
                else
                    1
                end
            end
            "|" con.dos_putsln
            row 1 add >row
            0
        else
            1
        end
    end
;

fn dos_hex_save () :
    local sectors 0
    local tail 0
    local off 0
    dos_hex_len 511 add 9 shr >sectors
    sectors 512 mul dos_hex_len sub >tail
    DOS_FILE_BUF dos_hex_len add tail con.dos_memzero
    disk.dos_time dsfs.dsfs_set_time
    DOS_DIR_BUF DOS_BM_BUF DOS_HEX_RAW DOS_FILE_BUF dos_hex_len dsfs.dsfs_write_file 0 neq if "SAVE FAILED" edit.dos_msg_err retn end
    "SAVED " DOS_MSGBUF con.dos_strcpy >off
    dos_hex_len DOS_MSGBUF off add con.dos_num_text off add >off
    " BYTES" DOS_MSGBUF off add con.dos_strcpy drop
    1 >dos_msg
    0 >dos_hex_dirty
;

fn dos_hex_dispatch (ptr n) :
    local w0 0
    local addr 0
    local val 0
    local k 0
    local i 0
    n 0 eq if
        dos_hex_pos 256 add >dos_hex_pos
        retn
    end
    DOS_WORD_BUF >w0
    w0 "N" con.dos_streq if
        dos_hex_pos 256 add >dos_hex_pos
        retn
    end
    w0 "NEXT" con.dos_streq if
        dos_hex_pos 256 add >dos_hex_pos
        retn
    end
    w0 "P" con.dos_streq if
        dos_hex_pos 256 sub >dos_hex_pos
        dos_hex_pos 0 lt if 0 >dos_hex_pos end
        retn
    end
    w0 "PREV" con.dos_streq if
        dos_hex_pos 256 sub >dos_hex_pos
        dos_hex_pos 0 lt if 0 >dos_hex_pos end
        retn
    end
    w0 "Q" con.dos_streq if 1 >dos_quit retn end
    w0 "QUIT" con.dos_streq if 1 >dos_quit retn end
    w0 "W" con.dos_streq if dos_hex_save retn end
    w0 "SAVE" con.dos_streq if dos_hex_save retn end
    w0 "G" con.dos_streq if
        n 2 lt if "USAGE: G ADDRESS" edit.dos_msg_err retn end
        DOS_WORD_BUF 32 add names.dos_parse_hex >addr
        addr 0 lt if "BAD ADDRESS" edit.dos_msg_err retn end
        addr dos_hex_len gt if "ADDRESS OUT OF RANGE" edit.dos_msg_err retn end
        addr 256 and addr sub >dos_hex_pos
        retn
    end
    w0 "GOTO" con.dos_streq if
        n 2 lt if "USAGE: GOTO ADDRESS" edit.dos_msg_err retn end
        DOS_WORD_BUF 32 add names.dos_parse_hex >addr
        addr 0 lt if "BAD ADDRESS" edit.dos_msg_err retn end
        addr dos_hex_len gt if "ADDRESS OUT OF RANGE" edit.dos_msg_err retn end
        addr 256 and addr sub >dos_hex_pos
        retn
    end
    w0 "E" con.dos_streq if
        n 3 lt if "USAGE: E ADDRESS V1 [V2 ...]" edit.dos_msg_err retn end
        DOS_WORD_BUF 32 add names.dos_parse_hex >addr
        addr 0 lt if "BAD ADDRESS" edit.dos_msg_err retn end
        2 >k
        while
            k n lt
            if
                DOS_WORD_BUF k 32 mul add names.dos_parse_hex >val
                val 0 lt if "BAD VALUE" edit.dos_msg_err retn end
                val 255 gt if "VALUE MUST BE A BYTE" edit.dos_msg_err retn end
                addr dos_hex_len ge if "WRITE PAST END OF FILE" edit.dos_msg_err retn end
                val DOS_FILE_BUF addr add stb
                1 >dos_hex_dirty
                addr 1 add >addr
                k 1 add >k
                0
            else
                1
            end
        end
        retn
    end
    w0 "SET" con.dos_streq if
        n 3 lt if "USAGE: SET ADDRESS V1 [V2 ...]" edit.dos_msg_err retn end
        DOS_WORD_BUF 32 add names.dos_parse_hex >addr
        addr 0 lt if "BAD ADDRESS" edit.dos_msg_err retn end
        2 >k
        while
            k n lt
            if
                DOS_WORD_BUF k 32 mul add names.dos_parse_hex >val
                val 0 lt if "BAD VALUE" edit.dos_msg_err retn end
                val 255 gt if "VALUE MUST BE A BYTE" edit.dos_msg_err retn end
                addr dos_hex_len ge if "WRITE PAST END OF FILE" edit.dos_msg_err retn end
                val DOS_FILE_BUF addr add stb
                1 >dos_hex_dirty
                addr 1 add >addr
                k 1 add >k
                0
            else
                1
            end
        end
        retn
    end
    "UNKNOWN HEX COMMAND" edit.dos_msg_err
;

fn dos_cmd_hex (name) :
    local size 0
    name DOS_HEX_RAW 11 con.dos_memcpy
    DOS_HEX_RAW DOS_HEX_TXT names.dos_name_text
    disk.dos_ensure 0 neq if retn end
    name disk.dos_file_size >size
    size 0 lt if "NOT FOUND" con.dos_err retn end
    size 0 eq if "EMPTY FILE HAS NOTHING TO EDIT" con.dos_err retn end
    size DOS_MAX_FILE gt if "FILE TOO LARGE" con.dos_err retn end
    DOS_DIR_BUF name DOS_FILE_BUF dsfs.dsfs_read_file 0 neq if "READ ERROR" con.dos_err retn end
    size >dos_hex_len
    0 >dos_hex_pos
    0 >dos_hex_dirty
    0 >dos_quit
    while
        dos_quit 0 eq
        if
            dos_hex_show
            ":" con.dos_puts
            DOS_CMD_BUF DOS_CMD_CAP kbd.dos_read_line >size
            DOS_CMD_BUF size names.dos_words >size
            DOS_CMD_BUF size dos_hex_dispatch
            0
        else
            1
        end
    end
    dos_hex_dirty 0 neq if
        "UNSAVED CHANGES DISCARDED (W TO SAVE)" con.dos_putsln
    end
;
!end

# ---------------- shell ----------------
!namespace shell -

fn dos_dispatch (ptr n) :
    local w0 0
    local w1 0
    local w2 0
    local num 0
    n 0 eq if retn end
    DOS_WORD_BUF >w0
    DOS_WORD_BUF 32 add >w1
    DOS_WORD_BUF 64 add >w2
    w0 "DIR" con.dos_streq if cmds.dos_cmd_dir retn end
    w0 "LS" con.dos_streq if cmds.dos_cmd_dir retn end
    w0 "FILES" con.dos_streq if cmds.dos_cmd_dir retn end
    w0 "TYPE" con.dos_streq if
        n 2 lt if "USAGE: TYPE NAME" con.dos_err retn end
        w1 names.dos_name_from_word 0 neq if "BAD FILE NAME" con.dos_err retn end
        DOS_NAME_A cmds.dos_cmd_type
        retn
    end
    w0 "CAT" con.dos_streq if
        n 2 lt if "USAGE: CAT NAME" con.dos_err retn end
        w1 names.dos_name_from_word 0 neq if "BAD FILE NAME" con.dos_err retn end
        DOS_NAME_A cmds.dos_cmd_type
        retn
    end
    w0 "RUN" con.dos_streq if
        n 2 lt if "USAGE: RUN NAME" con.dos_err retn end
        w1 names.dos_name_from_word 0 neq if "BAD FILE NAME" con.dos_err retn end
        DOS_NAME_A cmds.dos_cmd_run
        retn
    end
    w0 "NEW" con.dos_streq if
        n 2 lt if "USAGE: NEW NAME" con.dos_err retn end
        w1 names.dos_name_from_word 0 neq if "BAD FILE NAME" con.dos_err retn end
        DOS_NAME_A cmds.dos_cmd_new
        retn
    end
    w0 "CREATE" con.dos_streq if
        n 2 lt if "USAGE: CREATE NAME" con.dos_err retn end
        w1 names.dos_name_from_word 0 neq if "BAD FILE NAME" con.dos_err retn end
        DOS_NAME_A cmds.dos_cmd_new
        retn
    end
    w0 "DEL" con.dos_streq if
        n 2 lt if "USAGE: DEL NAME" con.dos_err retn end
        w1 names.dos_name_from_word 0 neq if "BAD FILE NAME" con.dos_err retn end
        DOS_NAME_A cmds.dos_cmd_delete
        retn
    end
    w0 "RM" con.dos_streq if
        n 2 lt if "USAGE: RM NAME" con.dos_err retn end
        w1 names.dos_name_from_word 0 neq if "BAD FILE NAME" con.dos_err retn end
        DOS_NAME_A cmds.dos_cmd_delete
        retn
    end
    w0 "REN" con.dos_streq if
        n 3 lt if "USAGE: REN OLD NEW" con.dos_err retn end
        w1 names.dos_name_from_word 0 neq if "BAD OLD NAME" con.dos_err retn end
        DOS_NAME_A DOS_NAME_B 11 con.dos_memcpy
        w2 names.dos_name_from_word 0 neq if "BAD NEW NAME" con.dos_err retn end
        DOS_NAME_B DOS_NAME_A cmds.dos_cmd_rename
        retn
    end
    w0 "RENAME" con.dos_streq if
        n 3 lt if "USAGE: RENAME OLD NEW" con.dos_err retn end
        w1 names.dos_name_from_word 0 neq if "BAD OLD NAME" con.dos_err retn end
        DOS_NAME_A DOS_NAME_B 11 con.dos_memcpy
        w2 names.dos_name_from_word 0 neq if "BAD NEW NAME" con.dos_err retn end
        DOS_NAME_B DOS_NAME_A cmds.dos_cmd_rename
        retn
    end
    w0 "COPY" con.dos_streq if
        n 3 lt if "USAGE: COPY SRC DST" con.dos_err retn end
        w1 w2 cmds.dos_cmd_copy
        retn
    end
    w0 "CP" con.dos_streq if
        n 3 lt if "USAGE: CP SRC DST" con.dos_err retn end
        w1 w2 cmds.dos_cmd_copy
        retn
    end
    w0 "EDIT" con.dos_streq if
        n 2 lt if "USAGE: EDIT NAME" con.dos_err retn end
        w1 names.dos_name_from_word 0 neq if "BAD FILE NAME" con.dos_err retn end
        DOS_NAME_A edit.dos_cmd_edit
        retn
    end
    w0 "HEX" con.dos_streq if
        n 2 lt if "USAGE: HEX NAME" con.dos_err retn end
        w1 names.dos_name_from_word 0 neq if "BAD FILE NAME" con.dos_err retn end
        DOS_NAME_A hex.dos_cmd_hex
        retn
    end
    w0 "FORMAT" con.dos_streq if
        dos_drv >num
        n 2 ge if
            w1 names.dos_parse_dec >num
            num 0 lt if "BAD DRIVE NUMBER" con.dos_err retn end
        end
        num 4 ge if "BAD DRIVE NUMBER" con.dos_err retn end
        num cmds.dos_cmd_format
        num >dos_drv
        dos_drv disk.dos_mount 0 neq if "FORMATTED BUT MOUNT FAILED" con.dos_err retn end
        retn
    end
    w0 "DRV" con.dos_streq if
        n 2 lt if "USAGE: DRV N" con.dos_err retn end
        w1 names.dos_parse_dec >num
        num 0 lt if "BAD DRIVE NUMBER" con.dos_err retn end
        num 4 ge if "BAD DRIVE NUMBER" con.dos_err retn end
        num >dos_drv
        dos_drv disk.dos_mount 0 neq if
            "DRIVE " con.dos_puts
            dos_drv con.dos_putnum
            " IS NOT MOUNTED" con.dos_putsln
        else
            "DRIVE " con.dos_puts
            dos_drv con.dos_putnum
            " READY" con.dos_putsln
        end
        retn
    end
    w0 "FREE" con.dos_streq if cmds.dos_cmd_free retn end
    w0 "MODE" con.dos_streq if
        n 2 lt if "USAGE: MODE 40|80" con.dos_err retn end
        w1 names.dos_parse_dec >num
        num 40 eq if
            pcg.col40
            con.dos_cls
            "40-COLUMN MODE" con.dos_putsln
            retn
        end
        num 80 eq if
            pcg.col80
            con.dos_cls
            "80-COLUMN MODE" con.dos_putsln
            retn
        end
        "USAGE: MODE 40|80" con.dos_err
        retn
    end
    w0 "STAT" con.dos_streq if cmds.dos_cmd_free retn end
    w0 "HELP" con.dos_streq if cmds.dos_cmd_help retn end
    w0 "CLS" con.dos_streq if con.dos_cls retn end
    w0 "EXIT" con.dos_streq if cmds.dos_cmd_exit retn end
    w0 "QUIT" con.dos_streq if cmds.dos_cmd_exit retn end
    w0 "BYE" con.dos_streq if cmds.dos_cmd_exit retn end
    "UNKNOWN COMMAND - TYPE HELP" con.dos_err
;

fn dos_banner () :
    "CPT32 DOS v1.0 - DSFS / FDD SHELL" con.dos_putsln
    "DIR TYPE RUN NEW REN DEL COPY EDIT HEX FORMAT DRV MODE FREE CLS HELP EXIT" con.dos_putsln
    con.dos_newline
;

fn dos_repl () :
    local n 0
    local w 0
    con.dos_cls
    dos_banner
    dos_drv disk.dos_mount 0 neq if
        "WARNING: DRIVE " con.dos_puts
        dos_drv con.dos_putnum
        " IS NOT MOUNTED - USE FORMAT" con.dos_putsln
    end
    while
        dos_drv con.dos_putnum
        "> " con.dos_puts
        DOS_CMD_BUF DOS_CMD_CAP kbd.dos_read_line >n
        DOS_CMD_BUF n names.dos_words >w
        DOS_CMD_BUF w dos_dispatch
        0
    end
;
!end

fn main () :
    1 DOS_VDP_ENABLE stb
    pcg.initPCG
    pcg.col80
    8 irqc.IRQC_PENDING sth
    8 irqc.IRQC_ENABLE sth
    shell.dos_repl
;

main
