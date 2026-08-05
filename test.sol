!const UART_ADDR 0x80040000

fn putc (char) :
    char UART_ADDR stb
;

fn print (ptr) :
    @loop
    local i 0
    ptr i add ldb
    i 1 add >i
    dup putc
    jnz @loop
;

"Hello, world!" print