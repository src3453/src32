# Rotating cube sent as object-space triangles to the VPU T&L pipeline.
!include "sol/cpt32/irqc.sol"
!include "sol/cpt32/pec_rng.sol"
!const FP 1024
!const FIFO 0x8003002C
!const VDP_ENABLE 0x80000000
!const VDP_MODE 0x80000001
!const VDP_BORDER 0x80000003
!const VDP_BITMAP_COLOR_MODE 0x80000004

!const VPU_OUTPUT_GP 0x80030014
!const SGC_CONTROL 0x80010004
!const SGC_OUTPUT_GP 0x80010018
!const SGC_FIFO 0x80010020
!const PEC_EVENT_DATA 0x80040030
!const PEC_EVENT_STATUS 0x80040034
!const GP0_BITMAP 0x10000000
!const GP0_CLUT 0x10012C00
!const GP0_BITMAP_PIXELS 76800
!const PEC_EVENT_POP 0x80040038

!var camera_distance 240
!var rotation_speed 2
!var light_red 12
!var light_green 12
!var light_blue 12
!var overlay_dirty 1
!var light_dirty 0
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
    # RGBA zero clears VPU alpha, exposing the lower GP0 bitmap.
    0x04000003 emit
    3 emit
    0 emit
    1.0f emit # Reset depth to the far plane so the cube passes LESS testing.
;

fn set_view_matrix (distance) :
    0x02000011 emit
    1 emit # VIEW
    1.0f emit 0.0f emit 0.0f emit 0.0f emit
    0.0f emit 1.0f emit 0.0f emit 0.0f emit
    0.0f emit 0.0f emit 1.0f emit distance 1024 mul q10_to_f32 emit
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

# Keep CLUT entries 0..63 intact for the system palette and SGC text colors.
# Use the remaining entries for GP0 grayscale noise.
fn setup_gp_noise () :
    local i 64
    local clut
    local gray
    while
        i 64 sub 255 mul 191 div >gray
        i 3 mul GP0_CLUT add >clut
        gray clut stb
        gray clut 1 add stb
        gray clut 2 add stb
        i 1 add >i
        i 256 lt
    end
    0 >i
    while
        rand 255 and 192 mod 64 add GP0_BITMAP i add stb
        i 1 add >i
        i GP0_BITMAP_PIXELS lt
    end
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

fn setup_light (red green blue) :
    0x15000011 emit # SET_LIGHT, light 0 enabled
    0 emit 1 emit
    0.2f emit 0.2f emit 0.2f emit # ambient
    red 68 mul q10_to_f32 emit green 68 mul q10_to_f32 emit blue 68 mul q10_to_f32 emit
    0.2f emit 0.2f emit 0.2f emit # specular
    0.0f emit 0.0f emit 0.0f emit # emission
    0.0f emit 0.0f emit -100.0f emit # view-space point light on the camera side
    0x01000002 emit # enable fixed-function lighting
    1 emit 1 emit
;

# Clamp an integer adjustment to its inclusive range.
fn adjust_clamped (value delta minimum maximum) :
    local next
    value delta add >next
    next minimum lt if
        minimum >next
    end
    next maximum gt if
        maximum >next
    end
    next ret
;

fn draw_glyph (code x y) :
    0x13000000 SGC_FIFO st
    code SGC_FIFO st
    x SGC_FIFO st
    y SGC_FIFO st
;

fn draw_text (ptr x y) :
    local i 0
    local code 0
    while
        ptr i add ldb >code
        code 0 neq if
            code x i 8 mul add y draw_glyph
        end
        i 1 add >i
        code 0 neq
    end
;

fn draw_number2 (value x y) :
    value 10 div 48 add x y draw_glyph
    value 10 mod 48 add x 8 add y draw_glyph
;

fn draw_number3 (value x y) :
    value 100 div 48 add x y draw_glyph
    value 10 div 10 mod 48 add x 8 add y draw_glyph
    value 10 mod 48 add x 16 add y draw_glyph
;

# Rebuild the persistent SGC plane only after an input change.
fn draw_overlay () :
    2 SGC_CONTROL st # soft reset clears the old text plane
    7 SGC_OUTPUT_GP st
    1 SGC_CONTROL st
    0x0100003F SGC_FIFO st # white glyphs
    "W/S DIST A/D SPD" 8 8 draw_text
    "J/K R U/I G N/M B" 8 24 draw_text
    "D:" 8 48 draw_text
    camera_distance 24 48 draw_number3
    "S:" 64 48 draw_text
    rotation_speed 80 48 draw_number2
    "R:" 112 48 draw_text
    light_red 128 48 draw_number2
    "G:" 160 48 draw_text
    light_green 176 48 draw_number2
    "B:" 208 48 draw_text
    light_blue 224 48 draw_number2
;

fn handle_key (key) :
    local next
    key 26 eq if # W: move closer
        camera_distance -8 128 512 adjust_clamped >next
        next camera_distance neq if
            next >camera_distance
            1 >overlay_dirty
        end
    else
        key 22 eq if # S: move farther
            camera_distance 8 128 512 adjust_clamped >next
            next camera_distance neq if
                next >camera_distance
                1 >overlay_dirty
            end
        else
            key 4 eq if # A: slower
                rotation_speed -1 0 12 adjust_clamped >next
                next rotation_speed neq if
                    next >rotation_speed
                    1 >overlay_dirty
                end
            else
                key 7 eq if # D: faster
                    rotation_speed 1 0 12 adjust_clamped >next
                    next rotation_speed neq if
                        next >rotation_speed
                        1 >overlay_dirty
                    end
                else
                    key 13 eq if # J: red down
                        light_red -1 0 15 adjust_clamped >next
                        next light_red neq if
                            next >light_red
                            1 >light_dirty 1 >overlay_dirty
                        end
                    else
                        key 14 eq if # K: red up
                            light_red 1 0 15 adjust_clamped >next
                            next light_red neq if
                                next >light_red
                                1 >light_dirty 1 >overlay_dirty
                            end
                        else
                            key 24 eq if # U: green down
                                light_green -1 0 15 adjust_clamped >next
                                next light_green neq if
                                    next >light_green
                                    1 >light_dirty 1 >overlay_dirty
                                end
                            else
                                key 12 eq if # I: green up
                                    light_green 1 0 15 adjust_clamped >next
                                    next light_green neq if
                                        next >light_green
                                        1 >light_dirty 1 >overlay_dirty
                                    end
                                else
                                    key 17 eq if # N: blue down
                                        light_blue -1 0 15 adjust_clamped >next
                                        next light_blue neq if
                                            next >light_blue
                                            1 >light_dirty 1 >overlay_dirty
                                        end
                                    else
                                        key 16 eq if # M: blue up
                                            light_blue 1 0 15 adjust_clamped >next
                                            next light_blue neq if
                                                next >light_blue
                                                1 >light_dirty 1 >overlay_dirty
                                            end
                                        end
                                    end
                                end
                            end
                        end
                    end
                end
            end
        end
    end
;

# Consume PeC events and react only to keyboard-down events.
fn poll_keyboard () :
    local event 0
    local usage
    while
        PEC_EVENT_STATUS ld 1 and 0 gt if
            PEC_EVENT_DATA ld >event
            event 28 shr 15 and 0 eq if
                event 24 shr 15 and 0 eq if
                    event 16 shr 255 and >usage
                    event 1 and 0 neq if
                        usage handle_key
                    end
                end
            end
            1 PEC_EVENT_POP st
        end
        PEC_EVENT_STATUS ld 1 and 0 gt
    end
;

fn irq0 () :
    1 IRQC_PENDING sth
    retn
;

fn main () :
    1 VDP_ENABLE stb
    0 VDP_MODE stb
    0 VDP_BORDER stb
    6 VPU_OUTPUT_GP st # leave GP7 above the cube for the SGC text overlay
    2 SGC_CONTROL st # reset the command plane
    7 SGC_OUTPUT_GP st
    0 VDP_BITMAP_COLOR_MODE stb
    1 SGC_CONTROL st
    initPRNG
    setup_gp_noise
    setup_noise_texture
    camera_distance set_view_matrix
    set_projection_matrix
    light_red light_green light_blue setup_light
    draw_overlay
    1 IRQC_PENDING sth
    1 IRQC_ENABLE sth
    local yaw 25
    local pitch 20
    while
        poll_keyboard
        light_dirty 0 neq if
            light_red light_green light_blue setup_light
            0 >light_dirty
        end
        overlay_dirty 0 neq if
            draw_overlay
            0 >overlay_dirty
        end
        clear_frame
        camera_distance set_view_matrix
        yaw pitch set_model_matrix
        draw_cube
        yaw rotation_speed add 360 mod >yaw
        pitch rotation_speed add 360 mod >pitch
        halt
        0
    end
;

main
