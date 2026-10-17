!include "cpt32/pcg.sol"

# CPT32 BIOS: POST (hardware survey) + IPL (jump to 0x0).
#
# Build as a RAM program (default origin, runs from 0x0):
#   python3 tools/solc/solc.py compile sol/bios.sol -o bios.asm
#   python3 tools/asm/asm.py -o bios.bin bios.asm
# Build as a BIOS ROM image (runs from 0xFF000000, 64KB max):
#   python3 tools/solc/solc.py compile --org 0xFF000000 \
#       --read-only-data-base 0xFF008000 sol/bios.sol -o bios_rom.asm
#   python3 tools/asm/asm.py --strip -o assets/bios.rom bios_rom.asm
# Notes: rodata must live inside the ROM window (RAM addresses are not
# preloaded at ROM boot); --strip omits the leading zero padding so the
# file starts at the ROM base instead of address 0.
#
# Survey policy: only touch registers that are always mapped in every
# standard configuration (emulator main binary and src32_testbench).
# Unmapped access raises a bus error which sol cannot catch, so
# UART (testbench has none), PeC timer/RTC (not wired in main) and
# WASM expansion slots (empty slots are unmapped) are NOT probed.

!var fails 0
!var tmp 0
!var frames 0

!const IRQC_PENDING 0xFFFF0040
!const IRQC_ENABLE 0xFFFF0044
!const POST_HOLD_FRAMES 120

# VBlank counter: acknowledge IRQ0 so the next host frame latches a new
# edge, then count the frame. Fires on every VSync while IRQ0 is enabled.
fn irq0 () :
    1 IRQC_PENDING sth
    frames 1 add >frames
    retn
;

fn prnhexch (n) :
    n 10 lt if
        n 0x30 add pcg.putcscr retn
    end
    n 0x37 add pcg.putcscr
;

fn prnhex8 (v) :
    local n 8
    while
        n 1 sub >n
        v n 4 mul shr 0xF and prnhexch
        n 0 gt
    end
;

fn prnhex2 (v) :
    v 4 shr 0xF and prnhexch
    v 0xF and prnhexch
;

fn ok () :
    0x2A pcg.colorfg "OK" pcg.prnscr 0x3f pcg.colorfg
;

fn ng () :
    0x15 pcg.colorfg "NG" pcg.prnscr 0x3f pcg.colorfg
    fails 1 add >fails
;

# Print value as 8 hex digits, then OK/NG against expect.
fn report (value expect) :
    value prnhex8
    " " pcg.prnscr
    value expect eq if
        ok retn
    end
    ng
;

# 32-bit RAM R/W test with save/restore. Returns sol-true (0) on match.
fn ramtest32 (addr) :
    addr ld >tmp
    0x5A5A5A5A addr st
    addr ld 0x5A5A5A5A eq if
        tmp addr st
        0 ret
    end
    tmp addr st
    1 ret
;

# SGU wavetable byte R/W test with save/restore (proves mapping).
fn sgutest (base) :
    base ldb >tmp
    0xA5 base stb
    base ldb 0xA5 eq if
        tmp base stb
        0 ret
    end
    tmp base stb
    1 ret
;

pcg.initPCG pcg.col40
0xc pcg.colorfg "CPT32 BIOS v0.3 POST+IPL" pcg.prnlnscr
0x3f pcg.colorfg "CPU: SRC32 @48MHz" pcg.prnlnscr
"MEM: RAM16M VRAM4M PCM1M" pcg.prnlnscr

"VDP status=" pcg.prnscr
0x80000002 ldb >tmp
tmp prnhex2
" " pcg.prnscr
tmp 0 eq if ok else ng end
pcg._pcg_ln

"SGC id=" pcg.prnscr
0x80010000 ld >tmp
tmp 0x53474331 report
pcg._pcg_ln
"SGC status=" pcg.prnscr
0x80010008 ld >tmp
tmp prnhex8
" " pcg.prnscr
tmp 0x1E and 0 eq if ok else ng end
pcg._pcg_ln

"VPU id=" pcg.prnscr
0x80030000 ld >tmp
tmp 0x56505531 report
pcg._pcg_ln
"VPU status=" pcg.prnscr
0x80030004 ld >tmp
tmp prnhex8
" " pcg.prnscr
tmp 0x2C and 0 eq if ok else ng end
pcg._pcg_ln

"SGU0 wave=" pcg.prnscr
0x80020000 sgutest if ok else ng end
" SGU1 wave=" pcg.prnscr
0x80021000 sgutest if ok else ng end
pcg._pcg_ln

"PeC seed=" pcg.prnscr
0x80040014 ld >tmp
tmp 0x55555555 report
pcg._pcg_ln
"PeC trng=" pcg.prnscr
0x80040018 ld prnhex8
" ev=" pcg.prnscr
0x80040034 ld prnhex8
pcg._pcg_ln

"DMAC id=" pcg.prnscr
0x80050000 ld >tmp
tmp 0x444D4101 report
pcg._pcg_ln
"BMC id=" pcg.prnscr
0x80060000 ld >tmp
tmp 0x424D4301 report
pcg._pcg_ln

"IDC id=" pcg.prnscr
0x80041000 ld >tmp
tmp 0x49444301 report
pcg._pcg_ln
"IDC cap=" pcg.prnscr
0x80041030 ld prnhex8
" sz=" pcg.prnscr
0x8004102C ld prnhex8
pcg._pcg_ln

# IRQC pending/enable is event-driven (VBlank etc. fire during
# POST), so a nonzero value is normal. Display only, never fail.
"IRQC=" pcg.prnscr
0xFFFF0040 ld prnhex8
pcg._pcg_ln

"RAM=" pcg.prnscr
0x00F00000 ramtest32 if ok else ng end
" VRAM=" pcg.prnscr
0x10300000 ramtest32 if ok else ng end
pcg._pcg_ln
"UART/TIMER/EXP: skipped" pcg.prnlnscr

fails 0 eq if
    0xc pcg.colorfg "POST OK" pcg.prnlnscr
else
    0x15 pcg.colorfg "POST FAIL" pcg.prnlnscr
end
0x3f pcg.colorfg

!var i 0
while
    i pcg.colorbg
    " " pcg.prnscr
    i 1 add >i
    i 16 mod if
        pcg._pcg_ln
    end
    i 16 neq
end
0 pcg.colorbg
0x3f pcg.colorfg

"IPL: hold POST 120 frames..." pcg.prnlnscr
# One host frame runs the whole POST+IPL, so without waiting the survey
# above is never rendered. HALT sleeps until the next enabled IRQ, hence
# each loop iteration spans one VBlank (~1/60s) on the emulator and on
# src32_testbench, which both pulse IRQ0 once per frame.
1 IRQC_PENDING sth
0 >frames
1 IRQC_ENABLE sth
while
    halt
    frames POST_HOLD_FRAMES lt
end
0 IRQC_ENABLE sth
1 IRQC_PENDING sth

"IPL: boot from 0x00000000..." pcg.prnlnscr
# Far jump via register: JMP encodes a 16-bit PC-relative offset and
# cannot reach 0x0 from the ROM window, so load the target and use JR.
# This form works at any load origin and never returns (R1 may clobber
# the sol stack cache, which no longer matters here).
!asm
LDIH R1, 0x0000
LDIL R1, 0x0000
JR R1
!end
