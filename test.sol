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
        ptr2 i add >addr # get writing addr
        ptr len add i sub 1 sub >addr_rev # get reading addr
        addr_rev ldb addr stb # xfer bytes addr_rev to addr
        i 1 add >i # increment ptr
        i len neq # continue until reach end
    end
;

fn tostr (num ptr ptr2) :
    local i 0
    local curval 0
    local isneg 0
    local rem 0
    num >curval # store input
    num 0 eq if # if zero
        0x30 ptr stb retn # print "0" and return
    else
        num sgn >isneg # store isneg if negative
        num sgn if
        else
            curval neg >curval # if negative negate and store
        end
        while
            curval 10 mod >rem # mod 10 and get remainder
            0x30 rem add ptr i add stb # add to 0x30 ("0") and calculate digit ascii code (ex: "9" -> 0x39)
            i 1 add >i # increment ptr
            curval 10 div >curval # division by 10
            curval 0 neq # if not zero then continue
        end
        isneg if # if is negative
        else
            0x2d ptr i add stb # append "-"
            i 1 add >i # increment ptr
        end
        ptr i ptr2 arevcp # reverse bytearray
        retn # and return
    end
;

!var i 1
!var is_mod3or5 0
while
    0 >is_mod3or5 # reset flag
    i 3 mod if # if mod 3
        "Fizz" prn # print fizz
        1 >is_mod3or5 # store flag
    end 
    i 5 mod if # if mod 5
        "Buzz" prn # print buzz
        1 >is_mod3or5 # store flag
    end
    is_mod3or5 if # else
        i 0x30000 0x30100 tostr # conv num to str
        0x30100 prn # print ptr
    end
    i 1 add >i # increment i
    0x0d putc # put cr
    0x0a putc # put lf
    i 101 neq # if not 100 then continue
end
halt