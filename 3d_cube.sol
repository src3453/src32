!const W 320
!const H 240
!const real_w 320
!const real_h 240
!const VRAM_BASE 0x10000000

fn pix (x y c) :
    local addr
    y real_w mul x add VRAM_BASE add >addr
    c addr stb
;

!const BLACK 0x00
!const WHITE 0x3F
!const FP 1024
!const HALF_SIZE 48
!const CAMERA_Z 240
!const FOCAL_LENGTH 240
!const FRAME_DELAY 3000

# 投影した8頂点の座標を置くRAM領域（8頂点 x (x,y) x 4バイト）。
# 既定のグローバル変数領域・スタックとは重ならない。
!const VERTICES 0x00110000

# 角度は度。sinを1024倍した値を整数近似で返す。
# 毎回角度から求めるので、回転を続けても丸め誤差が蓄積しない。
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

# 全方向に対応するBresenhamの線分描画（両端を含む）。
fn line (start_x start_y x1 y1 c) :
    local x0
    local y0
    local dx
    local dy
    local sx 1
    local sy 1
    local err
    local e2
    start_x >x0
    start_y >y0
    x1 x0 sub >dx
    dx 0 lt if
        dx neg >dx
        -1 >sx
    end
    y1 y0 sub >dy
    dy 0 lt if
        dy neg >dy
        -1 >sy
    end
    dy neg >dy
    dx dy add >err
    while
        # pix自体は範囲を検査しないため、ここで画面外書き込みを防ぐ。
        x0 0 ge x0 W lt or y0 0 ge or y0 H lt or if
            x0 y0 c pix
        end
        # solは真=0。条件の論理ANDにはorを使う。
        x0 x1 eq y0 y1 eq or if
            retn
        end
        err 2 mul >e2
        e2 dy ge if
            err dy add >err
            x0 sx add >x0
        end
        e2 dx le if
            err dx add >err
            y0 sy add >y0
        end
        0
    end
;

fn clear_screen () :
    local x
    local y 0
    while
        0 >x
        while
            x y BLACK pix
            x 1 add >x
            x W lt
        end
        y 1 add >y
        y H lt
    end
;

# Y軸回転 → X軸回転 → 透視投影。
# 頂点番号のbit 0/1/2を、それぞれ元のx/y/zの符号とする。
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

        # CAMERA_Zはキューブの外接球半径より大きく、depthは常に正。
        VERTICES i 8 mul add >addr
        rx FOCAL_LENGTH mul depth div W 2 div add addr st
        H 2 div ry FOCAL_LENGTH mul depth div sub addr 4 add st
        i 1 add >i
        i 8 lt
    end
;

fn edge (a b c) :
    local pa
    local pb
    VERTICES a 8 mul add >pa
    VERTICES b 8 mul add >pb
    pa ld pa 4 add ld pb ld pb 4 add ld c line
;

# 1ビットだけ異なる頂点を結ぶ。各辺を一度ずつ、計12本描画。
# 隠線処理はせず、裏側の辺も表示するワイヤーフレーム。
fn draw_cube (c) :
    local i 0
    local bit
    while
        1 >bit
        while
            i bit and 0 eq if
                i i bit or c edge
            end
            bit 1 shl >bit
            bit 4 le
        end
        i 1 add >i
        i 8 lt
    end
;

fn frame_wait () :
    local remaining
    # VBlank待機レジスタがないため、ソフトウェアループで速度調整。
    FRAME_DELAY >remaining
    while
        remaining 1 sub >remaining
        remaining 0 gt
    end
;

fn main () :
    local yaw 25
    local pitch 20
    0 0x80030001 stb  # グラフィックスモード
    1 0x80030000 stb  # 表示有効
    BLACK 0x80030003 stb  # 画面の枠も黒
    clear_screen
    yaw pitch project_vertices
    WHITE draw_cube
    while
        frame_wait
        BLACK draw_cube
        yaw 2 add 360 mod >yaw
        pitch 3 add 360 mod >pitch
        yaw pitch project_vertices
        WHITE draw_cube
        0
    end
;

main