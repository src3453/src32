!var pcg_cur_x 0
!var pcg_cur_y 0
!var pcg_width 40
!var pcg_height 30
!var pcg_fg_color 0x3f
!var pcg_bg_color 0
!var pcg_mode 0
!var pcg_cursor 0
!var pcg_vram_base 0x10000000

fn _pcg_fill (addr len val) :
    local i 0
    local ptr 0
    while
        addr i add >ptr
        val ptr stb
        i 1 add >i
        i len lt
    end
;

fn _pcg_acopy (ptr len ptr2) :
    local i 0
    local addr 0
    local addr2 0
    while
        ptr2 i add >addr2
        ptr i add >addr
        addr ldb addr2 stb
        i 1 add >i
        i len neq
    end
;

fn _pcg_set_cur (x y) :
    x 0x8003f005 stb
    y 0x8003f006 stb
;

fn col40 () :
    0 0x8003f003 stb # mode
    40 >pcg_width
;

fn col80 () :
    1 0x8003f003 stb # mode
    80 >pcg_width
;

fn _pcg_update_cur () :
    pcg_cur_x pcg_cur_y _pcg_set_cur
;

fn locate (x y) :
    x >pcg_cur_x
    y >pcg_cur_y
    _pcg_update_cur
;

fn cls (color) :
    0x10002000 0x1000 color _pcg_fill
;

fn initPCG () :
    1 0x80030001 stb # mode
    0 0x8003f003 stb # mode
    0 0x8003f000 stb # enable
    1 0x8003f007 stb # cursor
    10 0x8003f009 stb # cursor
    0 0 _pcg_set_cur # cursor
;

fn _pcg_ln () :
    0 >pcg_cur_x
    pcg_cur_y 1 add >pcg_cur_y
    pcg_cur_y pcg_height ge if
        local ptr
        local len 
        local ptr2
        local ptr3
        pcg_width pcg_vram_base add >ptr
        pcg_width pcg_height 1 sub mul >len
        pcg_vram_base >ptr2
        len pcg_vram_base add >ptr3
        ptr len ptr2 _pcg_acopy
        ptr3 pcg_width 0x00 _pcg_fill
        ptr 0x1000 add >ptr
        ptr2 0x1000 add >ptr2
        ptr3 0x1000 add >ptr3
        ptr len ptr2 _pcg_acopy
        ptr3 pcg_width 0x3f _pcg_fill
        ptr 0x1000 add >ptr
        ptr2 0x1000 add >ptr2
        ptr3 0x1000 add >ptr3
        ptr len ptr2 _pcg_acopy
        ptr3 pcg_width 0x00 _pcg_fill
        pcg_height 1 sub >pcg_cur_y 
    end
    _pcg_update_cur
;

fn putcscr (char) :
    local ptr
    pcg_cur_y pcg_width mul pcg_cur_x add pcg_vram_base add >ptr
    char ptr stb
    ptr 0x1000 add >ptr
    pcg_fg_color ptr stb
    ptr 0x1000 add >ptr
    pcg_bg_color ptr stb
    pcg_cur_x 1 add >pcg_cur_x
    pcg_cur_x pcg_width ge if
        _pcg_ln
    end
    _pcg_update_cur
;

fn prnscr (ptr) :
    local i 0 # offset
    local char 0 # current char
    while
        ptr i add ldb >char # get current char with offset and store to local var
        char 0 eq # continue until null terminator
        if
            retn
        end
        i 1 add >i # increment ptr
        char putcscr # put char
        0
    end
;

fn prnlnscr (ptr) :
    ptr prnscr
    _pcg_ln
;

fn colorfg (color) :
    color >pcg_fg_color
;

fn colorbg (color) :
    color >pcg_bg_color
;