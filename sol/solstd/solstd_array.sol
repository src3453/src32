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