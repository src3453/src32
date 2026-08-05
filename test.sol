!var char 0 # define var

fn write ( c ) :
    c 0x80040000 stb # write to uart
;

@loop # loop label
    char write
    char 1 add >char # increment counter and store to var
    char 256 sub
    jnz @loop # infinite loop