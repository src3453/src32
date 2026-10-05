# Flat-shaded, untextured cube using the VPU inline triangle command (0x12).
!const W 320
!const H 240
!const FP 1024
!const HALF_SIZE 48
!const CAMERA_Z 240
!const FOCAL_LENGTH 240
!const VERTICES 0x00110000
!const FIFO 0x8003002C
!const VDP_ENABLE 0x80000000
!const VDP_MODE 0x80000001
!const VDP_BORDER 0x80000003

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

# FIFO accepts 32-bit big-endian command words.
fn emit (word) :
    word FIFO st
;

fn clear_frame () :
    0x04000003 emit
    3 emit
    0 emit
    0 emit
;

# Vertex index bits select the signs of object-space x, y, and z.
fn project_vertices (yaw pitch) :
    local sn_y
    local cs_y
    local sn_x
    local cs_x
    local i 0
    local x
    local y
    local z
    local rx
    local ry
    local rz
    local depth
    local addr
    yaw sin_deg >sn_y
    yaw 90 add sin_deg >cs_y
    pitch sin_deg >sn_x
    pitch 90 add sin_deg >cs_x
    while
        i 1 and 2 mul 1 sub HALF_SIZE mul >x
        i 1 shr 1 and 2 mul 1 sub HALF_SIZE mul >y
        i 2 shr 1 and 2 mul 1 sub HALF_SIZE mul >z
        x cs_y mul z sn_y mul add FP div >rx
        z cs_y mul x sn_y mul sub FP div >rz
        y cs_x mul rz sn_x mul sub FP div >ry
        y sn_x mul rz cs_x mul add FP div CAMERA_Z add >depth
        VERTICES i 12 mul add >addr
        rx FOCAL_LENGTH mul depth div W 2 div add addr st
        H 2 div ry FOCAL_LENGTH mul depth div sub addr 4 add st
        depth addr 8 add st
        i 1 add >i
        i 8 lt
    end
;

# VPU 0x12 payload: xyz for three vertices, RGB, diffuse shade (0..255).
fn triangle (a b c rgb shade) :
    local pa
    local pb
    local pc
    VERTICES a 12 mul add >pa
    VERTICES b 12 mul add >pb
    VERTICES c 12 mul add >pc
    0x1200000B emit
    pa ld emit
    pa 4 add ld emit
    pa 8 add ld emit
    pb ld emit
    pb 4 add ld emit
    pb 8 add ld emit
    pc ld emit
    pc 4 add ld emit
    pc 8 add ld emit
    rgb emit
    shade emit
;

# Each face is two filled triangles. The VPU shades the whole face uniformly.
fn face (a b c d rgb shade) :
    a b c rgb shade triangle
    a c d rgb shade triangle
;

fn draw_cube () :
    0 2 6 4 0x00E05030 190 face
    1 5 7 3 0x003060E0 220 face
    0 1 3 2 0x00E0C030 145 face
    4 6 7 5 0x0030C070 205 face
    0 4 5 1 0x00C040C0 115 face
    2 3 7 6 0x0020C0D0 235 face
;

fn main () :
    1 VDP_ENABLE stb
    0 VDP_MODE stb
    0 VDP_BORDER stb
    local yaw 25
    local pitch 20
    while
        clear_frame
        yaw pitch project_vertices
        draw_cube
        yaw 12 add 360 mod >yaw
        pitch 7 add 360 mod >pitch
        0
    end
;

main
