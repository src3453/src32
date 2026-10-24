!namespace pcg -
!var pcg_cur_x 0
!var pcg_cur_y 0
!var pcg_width 40
!var pcg_height 30
!var pcg_cell_height 8
!var pcg_fg_color 0x3f
!var pcg_bg_color 0
!var pcg_mode 0
!var pcg_cursor 0
!var pcg_vram_base 0x10000000
!var pcg_scroll_y 0
!var pcg_vram_row 0

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

fn _pcg_set_cur (x y) :
    x 0x8000f005 stb
    y 0x8000f006 stb
;

fn col40 () :
    0 0x8000f003 stb # mode
    40 >pcg_width
    8 >pcg_cell_height
;

fn col80 () :
    1 0x8000f003 stb # mode
    80 >pcg_width
    8 >pcg_cell_height
;

fn mode80x16 () :
    3 0x8000f003 stb # mode
    80 >pcg_width
    30 >pcg_height
    16 >pcg_cell_height
;

fn _pcg_update_cur () :
    # The cursor swaps the cell's FG/BG, so prepare both current colors.
    local ptr 0
    pcg_vram_row pcg_width mul pcg_cur_x add pcg_vram_base add >ptr
    pcg_fg_color ptr 0x1000 add stb
    pcg_bg_color ptr 0x2000 add stb
    # CURSOR_POS registers are tilemap coordinates: the display samples
    # (screen + scroll), so writing the logical row pcg_cur_y would
    # drift by scroll_y/8 rows. pcg_vram_row is the physical ring row
    # backing the logical line, i.e. the tilemap coordinate to display.
    pcg_cur_x pcg_vram_row _pcg_set_cur
;

fn _pcg_update_vram_row () :
    pcg_scroll_y pcg_cell_height div pcg_cur_y add pcg_height mod >pcg_vram_row
;

fn locate (x y) :
    x >pcg_cur_x
    y >pcg_cur_y
    _pcg_update_vram_row
    _pcg_update_cur
;

fn cls (color) :
    # Clear the entire ring, including cells exposed by switching to 80 columns.
    # Equal FG/BG blanks writable glyph zero even if its bitmap is nonempty.
    pcg_vram_base 0x1000 0 _pcg_fill
    pcg_vram_base 0x1000 add 0x1000 color _pcg_fill
    pcg_vram_base 0x2000 add 0x1000 color _pcg_fill
    _pcg_update_cur
;

fn initPCG () :
    1 0x80000001 stb # mode
    0 0x8000f003 stb # mode
    40 >pcg_width
    30 >pcg_height
    8 >pcg_cell_height
    0 0x8000f000 stb # enable
    0 >pcg_scroll_y
    0 0x8000f00c stb # scroll X low
    0 0x8000f00d stb # scroll X high
    0 0x8000f00e stb # scroll Y low
    0 0x8000f00f stb # scroll Y high
    0 >pcg_cur_x
    0 >pcg_cur_y
    _pcg_update_vram_row
    1 0x8000f007 stb # cursor
    10 0x8000f009 stb # cursor
    _pcg_update_cur
;

fn _pcg_ln () :
    0 >pcg_cur_x
    pcg_cur_y 1 add >pcg_cur_y
    pcg_vram_row 1 add pcg_height mod >pcg_vram_row
    pcg_cur_y pcg_height ge if
        # Advance the pixel scroll by one text row, wrapping the source rows.
        pcg_scroll_y pcg_cell_height add pcg_height pcg_cell_height mul mod >pcg_scroll_y
        pcg_scroll_y 0x8000f00e stb
        pcg_height 1 sub >pcg_cur_y

        # Reuse and clear only the newly exposed physical row in the VRAM ring.
        local ptr 0
        pcg_vram_row pcg_width mul pcg_vram_base add >ptr
        ptr pcg_width 0x00 _pcg_fill
        ptr 0x1000 add pcg_width 0x3f _pcg_fill
        ptr 0x2000 add pcg_width 0x00 _pcg_fill
    end
    _pcg_update_cur
;

fn putcscr (char) :
    local ptr
    pcg_vram_row pcg_width mul pcg_cur_x add pcg_vram_base add >ptr
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
    _pcg_update_cur
;

fn colorbg (color) :
    color >pcg_bg_color
    _pcg_update_cur
;
!end
