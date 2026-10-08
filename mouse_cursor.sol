# PeC mouse cursor rendered by the SGC once per VSync (IRQ0).
!include "sol/cpt32/irqc.sol"
!const VDP_ENABLE 0x80000000
!const VDP_MODE 0x80000001
!const VDP_BORDER 0x80000003
!const SGC_CONTROL 0x80010004
!const SGC_OUTPUT_GP 0x80010018
!const SGC_FIFO 0x80010020
!const PEC_MOUSE_X 0x80040040
!const PEC_MOUSE_Y 0x80040044

# Append one signed-coordinate LINE command to the SGC FIFO.
fn line (x0 y0 x1 y1) :
    0x10000000 SGC_FIFO st
    x0 SGC_FIFO st
    y0 SGC_FIFO st
    x1 SGC_FIFO st
    y1 SGC_FIFO st
;


# Clear the SGC command plane to black before drawing the current pointer.
fn clear_frame () :
    0x01000000 SGC_FIFO st
    0x14000000 SGC_FIFO st
;

# A small arrow pointer: black outline with a white interior.
fn draw_cursor (x y) :
    0x01000000 SGC_FIFO st
    x y x y 14 add line
    x y 14 add x 4 add y 10 add line
    x 4 add y 10 add x 7 add y 16 add line
    x 7 add y 16 add x 10 add y 15 add line
    x 10 add y 15 add x 7 add y 9 add line
    x 7 add y 9 add x 12 add y 9 add line
    x 12 add y 9 add x y line

    0x0100003F SGC_FIFO st
    x 1 add y 2 add x 9 add y 8 add line
    x 1 add y 3 add x 1 add y 11 add line
    x 2 add y 11 add x 4 add y 9 add line
    x 5 add y 10 add x 7 add y 14 add line
;

# IRQ0 is the VDP VSync; acknowledge it so HALT waits for the next frame.
fn irq0 () :
    1 IRQC_PENDING sth
    retn
;

fn main () :
    1 VDP_ENABLE stb
    0 VDP_MODE stb
    0 VDP_BORDER stb
    2 SGC_CONTROL st
    1 SGC_OUTPUT_GP st
    1 SGC_CONTROL st
    1 IRQC_PENDING sth
    1 IRQC_ENABLE sth

    local x
    local y
    while
        clear_frame
        PEC_MOUSE_X ld 2 div >x
        PEC_MOUSE_Y ld 2 div >y
        x y draw_cursor
        halt
        0
    end
;

main
