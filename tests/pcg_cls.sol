!include "../sol/cpt32/pcg.sol"
!var failed 0

fn check_plane (addr value) :
    local i 0
    local expected 0
    while
        value >expected
        i pcg.pcg_vram_row pcg.pcg_width mul pcg.pcg_cur_x add eq if
            addr pcg.pcg_vram_base 0x1000 add eq if pcg.pcg_fg_color >expected end
            addr pcg.pcg_vram_base 0x2000 add eq if pcg.pcg_bg_color >expected end
        end
        addr i add ldb expected neq if 1 >failed end
        i 1 add >i
        i 0x1000 lt
    end
;

fn check_cls () :
    pcg.pcg_vram_base 0x3000 0x58 pcg._pcg_fill
    0xa5 0x10003000 stb
    0xff 0x10020000 stb
    0xff 0x10021000 stb
    pcg.pcg_cell_height 29 mul >pcg.pcg_scroll_y
    pcg.pcg_width 1 sub 29 pcg.locate
    0x12 pcg.colorfg
    0x23 pcg.colorbg
    0x15 pcg.cls
    pcg.pcg_vram_base 0 check_plane
    pcg.pcg_vram_base 0x1000 add 0x15 check_plane
    pcg.pcg_vram_base 0x2000 add 0x15 check_plane
    pcg.pcg_cur_x pcg.pcg_width 1 sub neq if 1 >failed end
    pcg.pcg_cur_y 29 neq if 1 >failed end
    pcg.pcg_vram_row 28 neq if 1 >failed end
    pcg.pcg_scroll_y pcg.pcg_cell_height 29 mul neq if 1 >failed end
    pcg.pcg_fg_color 0x12 neq if 1 >failed end
    pcg.pcg_bg_color 0x23 neq if 1 >failed end
    0x10003000 ldb 0xa5 neq if 1 >failed end
    0x10020000 ldb 0xff neq if 1 >failed end
    0x10021000 ldb 0xff neq if 1 >failed end
;

pcg.initPCG
pcg.col40 check_cls
pcg.col80 check_cls
pcg.mode80x16 check_cls
failed 0 eq if 42 else 1 end
