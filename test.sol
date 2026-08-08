!var UART_ADDR 0x80040000

fn putc (char) :
    char UART_ADDR stb # put char to UART
;

fn prn (ptr) :
    local i 0 # offset
    local char 0 # current char
    while
        ptr i add ldb >char # get current char with offset and store to local var
        i 1 add >i # increment ptr
        char putc # put char
        char 0 neq # continue until null terminator
    end
;

fn arevcp (ptr len ptr2) :
    local i 0
    local addr 0
    local addr_rev 0
    while
        ptr2 i add >addr
        ptr len add i sub 1 sub >addr_rev
        addr_rev ldb addr stb
        i 1 add >i
        i len neq
    end
;

fn tostr (num ptr ptr2) :
    local i 0
    local curval 0
    local isneg 0
    local rem 0
    num >curval
    num 0 eq if
        0x30 ptr stb retn
    else
        num sgn >isneg
        num sgn if
        else
            curval neg >curval
        end
        while
            curval 10 mod >rem
            0x30 rem add ptr i add stb
            i 1 add >i
            curval 10 div >curval
            curval 0 neq
        end
        isneg if
        else
            0x2d ptr i add stb
            i 1 add >i
        end
        ptr i ptr2 arevcp
        retn
    end
;

!var i 1
!var is_mod3or5 0
while
    0 >is_mod3or5
    i 3 mod if
        "Fizz" prn
        1 >is_mod3or5
    end
    i 5 mod if
        "Buzz" prn
        1 >is_mod3or5
    end
    is_mod3or5 if
        i 0x30000 0x30100 tostr
        0x30100 prn
    end
    i 1 add >i
    0x0d putc
    0x0a putc
    i 101 neq
end
halt