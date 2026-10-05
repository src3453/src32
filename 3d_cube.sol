# Rotating cube sent as object-space triangles to the VPU T&L pipeline.
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
    0x3F800000 emit 0 emit 0 emit 0 emit
    0 emit 0x3F800000 emit 0 emit 0 emit
    0 emit 0 emit 0x3F800000 emit 0x43700000 emit
    0 emit 0 emit 0 emit 0x3F800000 emit
;

# Infinite-far perspective projection, near plane at camera-space z=1.
fn set_projection_matrix () :
    0x02000011 emit
    2 emit # PROJECTION
    0x3FC00000 emit 0 emit 0 emit 0 emit # x scale = 1.5
    0 emit 0x40000000 emit 0 emit 0 emit # y scale = 2.0
    0 emit 0 emit 0x3F800000 emit 0xBF800000 emit
    0 emit 0 emit 0x3F800000 emit 0 emit
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
    cy q10_to_f32 emit 0 emit sy q10_to_f32 emit 0 emit
    m10 q10_to_f32 emit cx q10_to_f32 emit m12 q10_to_f32 emit 0 emit
    m20 q10_to_f32 emit sx q10_to_f32 emit m22 q10_to_f32 emit 0 emit
    0 emit 0 emit 0 emit 0x3F800000 emit
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
fn emit_vertex (index rgba) :
    local x
    local y
    local z
    index 1 and 0 eq if
        0x42400000 >x # +48.0f
    else
        0xC2400000 >x # -48.0f
    end
    index 1 shr 1 and 0 eq if
        0x42400000 >y
    else
        0xC2400000 >y
    end
    index 2 shr 1 and 0 eq if
        0x42400000 >z
    else
        0xC2400000 >z
    end
    x emit
    y emit
    z emit
    rgba emit
;

fn triangle (a b c rgba) :
    0x1400000C emit
    a rgba emit_vertex
    b rgba emit_vertex
    c rgba emit_vertex
;

fn face (a b c d rgba) :
    a b c rgba triangle
    a c d rgba triangle
;

fn draw_cube () :
    0 2 6 4 0xE05030FF face # -X face
    1 5 7 3 0x3060E0FF face # +X face
    0 1 3 2 0xE0C030FF face # -Y face
    4 6 7 5 0x30C070FF face # +Y face
    0 4 5 1 0xC040C0FF face # -Z face
    2 3 7 6 0x20C0D0FF face # +Z face
;

fn main () :
    1 VDP_ENABLE stb
    0 VDP_MODE stb
    0 VDP_BORDER stb
    set_view_matrix
    set_projection_matrix
    0x01000002 emit # SET_STATE, flat shading
    0 emit
    0 emit
    local yaw 25
    local pitch 20
    while
        clear_frame
        yaw pitch set_model_matrix
        draw_cube
        yaw 2 add 360 mod >yaw
        pitch 3 add 360 mod >pitch
        0
    end
;

main
