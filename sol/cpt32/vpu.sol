# Helpers for issuing VPU FIFO commands.
#
# Pointer arguments below refer to word-aligned 32-bit CPU memory. The pointed
# to words use the same order and bit representation as the VPU command stream.

!const VPU_FIFO_DATA 0x8003002C
!const VPU_STATUS 0x80030004

!const VPU_MATRIX_MODEL 0
!const VPU_MATRIX_VIEW 1
!const VPU_MATRIX_PROJECTION 2

!const VPU_SHADE_NONE 0
!const VPU_SHADE_FLAT 1
!const VPU_SHADE_GOURAUD 2

!const VPU_MODEL_UPLOAD 0x1A
!const VPU_DRAW_MODEL 0x1B

fn vpu_emit (word) :
    while
        VPU_STATUS ld 2 and 0 neq
    end
    word VPU_FIFO_DATA st
;

fn vpu_wait_idle () :
    while
        VPU_STATUS ld 1 and 0 neq
    end
;

# Build a command header from an 8-bit opcode and a 24-bit payload length.
fn vpu_command_word (opcode payload_words) :
    local shifts 24
    while
        opcode 1 shl >opcode
        shifts 1 sub >shifts
        shifts 0 gt
    end
    opcode payload_words or ret
;

fn vpu_clear (flags rgba depth) :
    4 3 vpu_command_word vpu_emit
    flags vpu_emit
    rgba vpu_emit
    depth vpu_emit
;

fn vpu_set_state (state_id value) :
    1 2 vpu_command_word vpu_emit
    state_id vpu_emit
    value vpu_emit
;

fn vpu_set_lighting (enabled) :
    1 enabled vpu_set_state
;

# ptr contains 16 row-major binary32 matrix words.
fn vpu_set_matrix (matrix_id ptr) :
    2 17 vpu_command_word vpu_emit
    matrix_id vpu_emit
    local i 0
    while
        ptr ld vpu_emit
        ptr 4 add >ptr
        i 1 add >i
        i 16 lt
    end
;

fn vpu_set_identity_matrix (matrix_id) :
    2 17 vpu_command_word vpu_emit
    matrix_id vpu_emit
    0x3F800000 vpu_emit 0 vpu_emit 0 vpu_emit 0 vpu_emit
    0 vpu_emit 0x3F800000 vpu_emit 0 vpu_emit 0 vpu_emit
    0 vpu_emit 0 vpu_emit 0x3F800000 vpu_emit 0 vpu_emit
    0 vpu_emit 0 vpu_emit 0 vpu_emit 0x3F800000 vpu_emit
;

# ptr contains ambient RGB, diffuse RGB, specular RGB, emission RGB, position XYZ
# as 15 consecutive binary32 words. Position is in VIEW space.
fn vpu_set_light (index enabled ptr) :
    21 17 vpu_command_word vpu_emit
    index vpu_emit
    enabled vpu_emit
    local i 0
    while
        ptr ld vpu_emit
        ptr 4 add >ptr
        i 1 add >i
        i 15 lt
    end
;

# Upload packed RGBA4444 texels from word-aligned CPU memory. Each source word
# contains two pixels, first pixel in bits 31:16. Odd pixel counts ignore the
# final word's low half. Caller must supply dimensions from 1 through 1024.
fn vpu_upload_texture (texture_id width height packed_pixels_ptr) :
    local words
    width height mul 1 add 1 shr >words
    local payload_words
    words 3 add >payload_words
    23 payload_words vpu_command_word vpu_emit
    texture_id vpu_emit
    width vpu_emit
    height vpu_emit
    local i 0
    while
        packed_pixels_ptr ld vpu_emit
        packed_pixels_ptr 4 add >packed_pixels_ptr
        i 1 add >i
        i words lt
    end
;

# ptr contains 18 words: for each vertex, POSITION xyz, UV uv, packed RGBA8.
fn vpu_draw_textured_triangle (texture_id shading_mode vertex_ptr) :
    24 20 vpu_command_word vpu_emit
    texture_id vpu_emit
    shading_mode vpu_emit
    local i 0
    while
        vertex_ptr ld vpu_emit
        vertex_ptr 4 add >vertex_ptr
        i 1 add >i
        i 18 lt
    end
;

# ptr contains 27 words: each vertex has POSITION xyz, NORMAL xyz, UV uv,
# and packed RGBA8 color. Positions and normals are in MODEL space.
fn vpu_draw_textured_lit_triangle (texture_id shading_mode vertex_ptr) :
    25 29 vpu_command_word vpu_emit
    texture_id vpu_emit
    shading_mode vpu_emit
    local i 0
    while
        vertex_ptr ld vpu_emit
        vertex_ptr 4 add >vertex_ptr
        i 1 add >i
        i 27 lt
    end
;

# Upload count records from RAM. Each record is texture ID plus 27 words of
# POSITION/NORMAL/UV/RGBA8 for the three vertices.
fn vpu_upload_model (model_id triangle_count record_ptr) :
    local payload_words
    triangle_count 28 mul 2 add >payload_words
    VPU_MODEL_UPLOAD payload_words vpu_command_word vpu_emit
    model_id vpu_emit
    triangle_count vpu_emit
    local total_words
    triangle_count 28 mul >total_words
    local i 0
    while
        record_ptr ld vpu_emit
        record_ptr 4 add >record_ptr
        i 1 add >i
        i total_words lt
    end
;

# Draw a model cached in VPU memory with the selected NONE/FLAT/GOURAUD mode.
fn vpu_draw_model (model_id shading_mode) :
    VPU_DRAW_MODEL 2 vpu_command_word vpu_emit
    model_id vpu_emit
    shading_mode vpu_emit
;

# ptr contains 12 words: three POSITION xyz + packed RGBA8 vertices.
fn vpu_draw_tl_triangle (vertex_ptr) :
    20 12 vpu_command_word vpu_emit
    local i 0
    while
        vertex_ptr ld vpu_emit
        vertex_ptr 4 add >vertex_ptr
        i 1 add >i
        i 12 lt
    end
;

# ptr contains six target words: color_base, z_base, width, height,
# color_stride, z_stride.
fn vpu_set_target (ptr) :
    3 6 vpu_command_word vpu_emit
    local i 0
    while
        ptr ld vpu_emit
        ptr 4 add >ptr
        i 1 add >i
        i 6 lt
    end
;
