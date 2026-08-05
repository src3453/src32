!var char 0 # define var
@loop # loop label
    char 0x80040000 stb # write to uart
    char 1 add >char # increment counter and store to var
    char 256 sub
    jnz @loop # infinite loop