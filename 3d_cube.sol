# Rotating cube sent as object-space triangles to the VPU T&L pipeline.
!include "sol/cpt32/irqc.sol"
!include "sol/cpt32/pec_rng.sol"
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

# Generate one opaque grayscale RGBA4444 noise texel from the hardware RNG.
fn noise_texel () :
    local n
    rand 15 and >n
    n 12 shl n 8 shl or n 4 shl or 15 or ret
;

# Upload 32x32 RGBA4444 noise. Two texels are packed in each FIFO payload word.
fn setup_noise_texture () :
    local i 0
    local pair
    local opcode 23
    local shift 24
    while
        opcode 1 shl >opcode
        shift 1 sub >shift
        shift 0 gt
    end
    opcode 515 or emit # TEXTURE_UPLOAD, 3 descriptor words + 512 packed texel words
    1 emit # texture ID
    32 emit
    32 emit
    while
        noise_texel 16 shl noise_texel or >pair
        pair emit
        i 1 add >i
        i 512 lt
    end
;

# Vertex index bits select the signs of object-space x, y, and z.
fn emit_cube_vertex (index u v rgba) :
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
    u emit
    v emit
    rgba emit
;

fn triangle (a b c u0 v0 u1 v1 u2 v2 rgba) :
    0x18000014 emit # texture 1, Flat, then three position/UV/color vertices
    1 emit # texture ID
    1 emit # Flat: vertex[0] color modulates the texture across the face
    a u0 v0 rgba emit_cube_vertex
    b u1 v1 rgba emit_cube_vertex
    c u2 v2 rgba emit_cube_vertex
;

fn face (a b c d rgba) :
    a b c 0.0f 0.0f 1.0f 0.0f 1.0f 1.0f rgba triangle
    a c d 0.0f 0.0f 1.0f 1.0f 0.0f 1.0f rgba triangle
;

fn draw_cube () :
    1 5 7 3 0xE05030FF face # -X, warm red tint
    0 2 6 4 0x3060E0FF face # +X, blue tint
    2 3 7 6 0xE0C030FF face # -Y, yellow tint
    0 4 5 1 0x30C070FF face # +Y, green tint
    4 6 7 5 0xC040C0FF face # -Z, purple tint
    0 1 3 2 0x20C0D0FF face # +Z, cyan tint
;

fn setup_light () :
    0x15000011 emit # SET_LIGHT, light 0 enabled
    0 emit 1 emit
    0.2f emit 0.2f emit 0.2f emit # ambient
    0.8f emit 0.8f emit 0.8f emit # diffuse
    0.2f emit 0.2f emit 0.2f emit # specular
    0.0f emit 0.0f emit 0.0f emit # emission
    0.0f emit 0.0f emit -100.0f emit # view-space point position outside the cube on the camera side (-Z)
    0x01000002 emit # enable fixed-function lighting
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
    initPRNG
    setup_noise_texture
    set_view_matrix
    set_projection_matrix
    setup_light
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
