# Rotating cube sent as object-space triangles to the VPU T&L pipeline.
!include "sol/cpt32/irqc.sol"
!const FP 1024
!const FIFO 0x8003002C
!const VDP_ENABLE 0x80000000
!const VDP_MODE 0x80000001
!const VDP_BORDER 0x80000003

fn emit (word) :
    word FIFO st
;

# Convert signed Q10 fixed-point values in [-1,1] to IEEE binary32 bits.
fn q10_to_f32 (value) :
    local sign 0
    local magnitude
    local scale -10
    value >magnitude
    value 0 lt if
        0x80000000 >sign
        magnitude neg >magnitude
    end
    magnitude 0 eq if
        0 ret
    end
    while
        magnitude 1024 ge if
            magnitude 1 shr >magnitude
            scale 1 add >scale
        end
        magnitude 1024 ge
    end
    while
        magnitude 512 lt if
            magnitude 1 shl >magnitude
            scale 1 sub >scale
        end
        magnitude 512 lt
    end
    magnitude 512 sub 14 shl
    scale 136 add 23 shl or sign or ret
;

fn clear_frame () :
    0x04000003 emit
    3 emit
    0 emit
    0 emit
;

fn set_view_matrix () :
    0x02000011 emit
    1 emit # VIEW
    1.0f emit 0.0f emit 0.0f emit 0.0f emit
    0.0f emit 1.0f emit 0.0f emit 0.0f emit
    0.0f emit 0.0f emit 1.0f emit 240.0f emit
    0.0f emit 0.0f emit 0.0f emit 1.0f emit
;

# Infinite-far perspective projection, near plane at camera-space z=1.
fn set_projection_matrix () :
    0x02000011 emit
    2 emit # PROJECTION
    1.5f emit 0.0f emit 0.0f emit 0.0f emit # x scale = 1.5
    0.0f emit 2.0f emit 0.0f emit 0.0f emit # y scale = 2.0
    0.0f emit 0.0f emit 1.0f emit -1.0f emit
    0.0f emit 0.0f emit 1.0f emit 0.0f emit
;

# Model = Rx(pitch) * Ry(yaw), represented as row-major binary32.
fn set_model_matrix (yaw pitch) :
    local sy
    local cy
    local sx
    local cx
    local m10
    local m12
    local m20
    local m22
    yaw sin_deg >sy
    yaw 90 add sin_deg >cy
    pitch sin_deg >sx
    pitch 90 add sin_deg >cx
    sx sy mul FP div >m10
    sx cy mul neg FP div >m12
    cx sy mul neg FP div >m20
    cx cy mul FP div >m22

    0x02000011 emit
    0 emit # MODEL
    cy q10_to_f32 emit 0.0f emit sy q10_to_f32 emit 0.0f emit
    m10 q10_to_f32 emit cx q10_to_f32 emit m12 q10_to_f32 emit 0.0f emit
    m20 q10_to_f32 emit sx q10_to_f32 emit m22 q10_to_f32 emit 0.0f emit
    0.0f emit 0.0f emit 0.0f emit 1.0f emit
;

fn sin_deg (angle) :
    local a
    local sign 1
    local t
    angle 360 mod >a
    a 0 lt if
        a 360 add >a
    end
    a 180 ge if
        a 180 sub >a
        -1 >sign
    end
    a 180 a sub mul >t
    t 4 mul FP mul 40500 t sub div sign mul ret
;

# Vertex index bits select the signs of object-space x, y, and z.
fn emit_vertex (index nx ny nz rgba) :
    local x
    local y
    local z
    index 1 and 0 eq if
        48.0f >x
    else
        -48.0f >x
    end
    index 1 shr 1 and 0 eq if
        48.0f >y
    else
        -48.0f >y
    end
    index 2 shr 1 and 0 eq if
        48.0f >z
    else
        -48.0f >z
    end
    x emit
    y emit
    z emit
    nx emit
    ny emit
    nz emit
    rgba emit
;

fn triangle (a b c rgba nx ny nz) :
    0x16000015 emit
    a nx ny nz rgba emit_vertex
    b nx ny nz rgba emit_vertex
    c nx ny nz rgba emit_vertex
;

fn face (a b c d rgba nx ny nz) :
    a b c rgba nx ny nz triangle
    a c d rgba nx ny nz triangle
;

fn draw_cube () :
    1 3 7 5 0xE05030FF -1.0f 0.0f 0.0f face # -X face
    0 2 6 4 0x3060E0FF 1.0f 0.0f 0.0f face # +X face
    2 3 7 6 0xE0C030FF 0.0f -1.0f 0.0f face # -Y face
    0 1 5 4 0x30C070FF 0.0f 1.0f 0.0f face # +Y face
    4 5 7 6 0xC040C0FF 0.0f 0.0f -1.0f face # -Z face
    0 1 3 2 0x20C0D0FF 0.0f 0.0f 1.0f face # +Z face
;

fn setup_light () :
    0x15000011 emit # SET_LIGHT, light 0 enabled
    0 emit 1 emit
    0.1f emit 0.1f emit 0.1f emit # ambient
    0.9f emit 0.9f emit 0.9f emit # diffuse
    0.3f emit 0.3f emit 0.3f emit # specular
    0.0f emit 0.0f emit 0.0f emit # emission
    0.0f emit 100.0f emit -80.0f emit # view-space position
    0x01000002 emit # SET_STATE lighting on
    1 emit 1 emit
;

fn irq0 () :
    1 IRQC_PENDING sth
    retn
;

fn main () :
    1 VDP_ENABLE stb
    0 VDP_MODE stb
    0 VDP_BORDER stb
    set_view_matrix
    set_projection_matrix
    setup_light
    0x01000002 emit # SET_STATE, flat shading
    0 emit
    0 emit
    1 IRQC_PENDING sth
    1 IRQC_ENABLE sth
    local yaw 25
    local pitch 20
    while
        clear_frame
        yaw pitch set_model_matrix
        draw_cube
        yaw 2 add 360 mod >yaw
        pitch 3 add 360 mod >pitch
        halt
        0
    end
;

main
