# Procedural VPU terrain fly-through.
# The sky is a camera-aligned far-plane gradient; the terrain is a Gouraud
# shaded grid whose vertex heights come from a deterministic integer hash.
!include "sol/cpt32/irqc.sol"
!const FIFO 0x8003002C
!const VDP_ENABLE 0x80000000
!const VDP_MODE 0x80000001
!const VDP_BORDER 0x80000003

fn emit (word) :
    word FIFO st
;

# Signed Q10 integer to IEEE binary32 bits.
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

fn set_identity_matrix (id) :
    0x02000011 emit
    id emit
    0x3F800000 emit 0 emit 0 emit 0 emit
    0 emit 0x3F800000 emit 0 emit 0 emit
    0 emit 0 emit 0x3F800000 emit 0 emit
    0 emit 0 emit 0 emit 0x3F800000 emit
;

fn set_sky_state () :
    0 set_identity_matrix
    1 set_identity_matrix
    2 set_identity_matrix
;

# Flat command payload: xyz and RGBA for each vertex.
fn colored_triangle (x0 y0 z0 c0 x1 y1 z1 c1 x2 y2 z2 c2) :
    0x1400000C emit
    x0 emit y0 emit z0 emit c0 emit
    x1 emit y1 emit z1 emit c1 emit
    x2 emit y2 emit z2 emit c2 emit
;

# A full-screen strip split into two Gouraud triangles. NDC z=.999 places
# this camera-facing sky just in front of the cleared far plane.
fn sky_band (top bottom top_color bottom_color) :
    0xBF800000 top 0x3F7FBE77 top_color
    0x3F800000 top 0x3F7FBE77 top_color
    0x3F800000 bottom 0x3F7FBE77 bottom_color
    colored_triangle
    0xBF800000 top 0x3F7FBE77 top_color
    0x3F800000 bottom 0x3F7FBE77 bottom_color
    0xBF800000 bottom 0x3F7FBE77 bottom_color
    colored_triangle
;

fn draw_sky () :
    set_sky_state
    # Zenith, high haze, horizon, and the faint warm band near sunset.
    0x3F800000 0x3F000000 0x18345EFF 0x18345EFF sky_band
    0x3F000000 0xBE800000 0x18345EFF 0x4E91C8FF sky_band
    0xBE800000 0xBF400000 0x4E91C8FF 0xFFC58AFF sky_band
    0xBF400000 0xBF800000 0xFFC58AFF 0xB76D55FF sky_band
;

# Two-octave deterministic lattice hash. Inputs are grid coordinates; the
# coarse octave adds broad hills while the fine octave adds ridges.
fn terrain_height (ix iz phase) :
    local fine
    local broad
    ix 73856093 mul iz phase add 19349663 mul xor
    0x7FFFFFFF and 31 mod 15 sub >fine
    ix 2 div 83492791 mul iz 3 div phase add 297121507 mul xor
    0x7FFFFFFF and 23 mod 11 sub >broad
    fine broad 2 mul add ret
;

# Elevation palette with a small deterministic light variation. The packed
# bytes are RGBA; Gouraud interpolation shades the mesh between vertex colors.
fn terrain_color (height seed) :
    local jitter
    local rgb
    seed 17 mul 0x45D9F3B xor 0x7FFFFFFF and 7 mod >jitter
    height -18 lt if
        0x214A35 >rgb
    else
        height 8 gt if
            0xAEB8B0 >rgb
        else
            height 0 gt if
                0x675A3D >rgb
            else
                0x39784A >rgb
            end
        end
    end
    # Vary green/rock brightness gently by a per-vertex hash.
    rgb 16 shr 255 and jitter add 16 shl
    rgb 8 shr 255 and jitter add 8 shl or
    rgb 255 and jitter add or
    8 shl 0xFF or ret
;

fn terrain_vertex (ix iz phase) :
    local x
    local z
    local h
    ix 18 mul 72 sub 1024 mul >x
    iz 14 mul phase sub 1024 mul >z
    ix iz phase 14 div add 0 terrain_height >h
    x q10_to_f32 emit
    h 2 mul 1024 mul q10_to_f32 emit
    z q10_to_f32 emit
    h ix iz xor terrain_color emit
;

fn terrain_triangle (x0 z0 x1 z1 x2 z2 phase) :
    0x1400000C emit
    x0 z0 phase terrain_vertex
    x1 z1 phase terrain_vertex
    x2 z2 phase terrain_vertex
;

fn draw_terrain (phase) :
    local x 0
    local z 0
    # 8 x 12 cells: 192 triangles, close enough for the terrain palette to
    # read clearly while keeping the software VPU demo responsive.
    while
        0 >z
        while
            x z x 1 add z x 1 add z 1 add phase terrain_triangle
            x z x 1 add z 1 add x z 1 add phase terrain_triangle
            z 1 add >z
            z 12 lt
        end
        x 1 add >x
        x 8 lt
    end
;

fn set_view_matrix () :
    # Camera looks gently down over the ground; world z advances through the
    # generated terrain while the camera banks from side to side in translation.
    0x02000011 emit
    1 emit
    0x3F800000 emit 0 emit 0 emit 0xC0000000 emit
    0 emit 0x3F708FB2 emit 0xBEAF1D44 emit 0xC2340000 emit
    0 emit 0x3EAF1D44 emit 0x3F708FB2 emit 0x41400000 emit
    0 emit 0 emit 0 emit 0x3F800000 emit
;

fn set_projection_matrix () :
    0x02000011 emit
    2 emit
    1.5f emit 0.0f emit 0.0f emit 0.0f emit
    0.0f emit 2.0f emit 0.0f emit 0.0f emit
    0.0f emit 0.0f emit 1.0f emit -1.0f emit
    0.0f emit 0.0f emit 1.0f emit 0.0f emit
;

# Acknowledge the VDP frame interrupt so HALT resumes for the next frame.
fn irq0 () :
    1 IRQC_PENDING sth
    retn
;

fn main () :
    1 VDP_ENABLE stb
    0 VDP_MODE stb
    0 VDP_BORDER stb
    0x01000002 emit
    0 emit 1 emit # Gouraud vertex-color interpolation
    1 IRQC_PENDING sth
    1 IRQC_ENABLE sth
    local phase 0
    while
        clear_frame
        draw_sky
        set_view_matrix
        set_projection_matrix
        phase draw_terrain
        phase 1 add 84 mod >phase
        halt
        0
    end
;

main
