!const UART_ADDR 0x80040000

fn std_putc (char) :
    char UART_ADDR stb
;

fn std_print (ptr) :
    local i 0
    local char 0
    @loop
    ptr i add ldb >char
    i 1 add >i
    char std_putc
    char jnz @loop
;