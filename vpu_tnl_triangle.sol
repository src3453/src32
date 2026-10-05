# VPU transform, clip, and Gouraud interpolation example.
!const FIFO 0x8003002C
!const VDP_ENABLE 0x80000000
!const VDP_MODE 0x80000001

fn emit (word) :
    word FIFO st
;

fn clear_frame () :
    0x04000003 emit
    3 emit
    0 emit
    0 emit
;

fn set_identity_matrix (id) :
    0x02000011 emit
    id emit
    0x3F800000 emit 0 emit 0 emit 0 emit
    0 emit 0x3F800000 emit 0 emit 0 emit
    0 emit 0 emit 0x3F800000 emit 0 emit
    0 emit 0 emit 0 emit 0x3F800000 emit
;

fn main () :
    1 VDP_ENABLE stb
    0 VDP_MODE stb
    0 set_identity_matrix # MODEL
    1 set_identity_matrix # VIEW
    2 set_identity_matrix # PROJECTION
    0x01000002 emit       # SET_STATE, 2-word payload
    0 emit                # shading mode
    1 emit                # Gouraud
    clear_frame
    # The green vertex is outside the right clip plane. The VPU clips the
    # triangle and interpolates vertex colors along the generated edge.
    0x1400000C emit
    0xBF4CCCCD emit 0xBF333333 emit 0x3F000000 emit 0xFF2020FF emit
    0x3FB33333 emit 0xBF333333 emit 0x3F000000 emit 0x20FF20FF emit
    0x00000000 emit 0x3F666666 emit 0x3F000000 emit 0x2020FFFF emit
    while
        0
    end
;

main
