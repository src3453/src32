!var UART_ADDR 0x80040000

fn putc (char) :
    char UART_ADDR stb # put char to UART
;

fn prn (ptr) :
    local i 0 # offset
    local char 0 # current char
    @loop
    ptr i add ldb >char # get current char with offset and store to local var
    i 1 add >i # increment ptr
    char putc # put char
    char jnz @loop # if not null then loop
;

fn arevcp (ptr len ptr2) :
    local i 0
    local addr 0
    local addr_rev 0
    @loop
    ptr2 i add >addr
    ptr len add i sub 1 sub >addr_rev
    addr_rev ldb addr stb
    i 1 add >i
    i len eq jnz @loop
;

fn tostr (num ptr ptr2) :
    local i 0
    local curval 0
    local isneg 0
    local rem 0
    num >curval
    num jz @handle_zero
    num sgn >isneg
    num sgn jnz @handle_negative
    @ret_handle_negative
    curval 10 mod >rem
    0x30 rem add ptr i add stb
    i 1 add >i
    curval 10 div >curval
    curval jnz @ret_handle_negative
    isneg jz @skip_append_negative
    0x2d ptr i add stb
    i 1 add >i
    @skip_append_negative
    ptr i ptr2 arevcp 
    retn

    @handle_negative
    curval neg >curval jmp @ret_handle_negative

    @handle_zero
    0x30 ptr stb retn

;

!var i 1
!var is_mod3or5 0
@loop
0 >is_mod3or5
101 i eq jz @end
i 3 mod jz @mod3
@ret_mod3
i 5 mod jz @mod5
@ret_mod5
is_mod3or5 jnz @skip
i 0x30000 0x30100 tostr
0x30100 prn
@skip
i 1 add >i
0x0d putc
0x0a putc
jmp @loop

@mod3
"Fizz" prn
1 >is_mod3or5
jmp @ret_mod3

@mod5
"Buzz" prn
1 >is_mod3or5
jmp @ret_mod5

@end
halt