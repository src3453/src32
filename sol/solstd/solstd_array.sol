# solstd_array.sol - sol Standard Library (solstd): Array operations

fn aget (ptr n) :
    local off
    local addr
    n 2 shl >off # multiply by 4
    ptr off add >addr
    addr ld
    ret
;

fn aset (ptr n val) :
    local off
    local addr
    n 2 shl >off # multiply by 4
    ptr off add >addr
    val addr st
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

fn acopy (ptr len ptr2) :
    local i 0
    local addr 0
    local addr2 0
    @loop
    ptr2 i add >addr2
    ptr i add >addr
    addr ldb addr2 stb
    i 1 add >i
    i len eq jnz @loop
;
