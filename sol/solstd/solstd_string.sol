# solstd_string.sol - sol Standard Library (solstd): String operations


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

fn prnln (ptr) :
    ptr prn # print with no ln
    0x0d putc # put CR
    0x0a putc # put LF
;