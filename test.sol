!const UART_ADDR 0x80040000

fn putc (char) :
    char UART_ADDR stb
;

!var i 0
@loop
i putc
i 1 add >i
jmp @loop