# Guest smoke test for sol/cpt32/idc.sol + sol/cpt32/dsfs.sol.
#
# Expects slot 0 to hold a DSFS image made by tools/dsfs/dsfs.py:
#   mkfs IMAGE --sectors 2880 --time 1700000000
#   put IMAGE GUEST.TXT content "Hello DSFS"
# The program mounts the volume, checks the preexisting file, writes
# NEWFILE.DAT (600 bytes of (i*7+3)&0xFF), reads it back, then deletes
# GUEST.TXT. R1 = 0 on success; any other value is a failing step
# (see the ret constants below).
#
# Build and run (from the repository root):
#   python3 tools/solc/solc.py compile tools/dsfs/dsfs_smoke.sol -o dsfs_guest.a
#   python3 tools/asm/asm.py dsfs_guest.a -o dsfs_guest.bin
#   src32_testbench dsfs_guest.bin        (run in a dir with config.yaml)
!include "../../sol/cpt32/idc.sol"
!include "../../sol/cpt32/dsfs.sol"

!const DIR_BUF 0x00300000
!const BM_BUF 0x00302000
!const FILE_BUF 0x00310000

fn main () :
    local i 0
    # Slot 0 must hold a drive with at least the 19-sector DSFS minimum.
    0 idc.idc_identify 19 lt if 1 ret end
    0 DIR_BUF dsfs.dsfs_mount 0 neq if 2 ret end
    dsfs.dsfs_total 2880 neq if 3 ret end
    DIR_BUF dsfs.dsfs_dir_load 0 neq if 4 ret end
    DIR_BUF dsfs.dsfs_count 1 neq if 5 ret end
    DIR_BUF "GUEST   TXT" dsfs.dsfs_find 0 neq if 6 ret end
    DIR_BUF "GUEST   TXT" FILE_BUF dsfs.dsfs_read_file 0 neq if 7 ret end
    FILE_BUF ldb 72 neq if 8 ret end
    # Write NEWFILE.DAT: 600 bytes of the (i*7+3)&0xFF pattern.
    1700000123 dsfs.dsfs_set_time
    0 >i
    while
        i 600 lt
        if
            i 7 mul 3 add 255 and FILE_BUF i add stb
            i 1 add >i
            0
        else
            1
        end
    end
    DIR_BUF BM_BUF "NEWFILE DAT" FILE_BUF 600 dsfs.dsfs_write_file 0 neq if 9 ret end
    FILE_BUF 600 add ldb 0 neq if 10 ret end
    # Read it back and spot-check the pattern endpoints.
    DIR_BUF "NEWFILE DAT" FILE_BUF dsfs.dsfs_read_file 0 neq if 11 ret end
    FILE_BUF ldb 3 neq if 12 ret end
    FILE_BUF 1 add ldb 10 neq if 13 ret end
    FILE_BUF 599 add ldb 100 neq if 14 ret end
    # Delete the original file and confirm the directory state.
    DIR_BUF BM_BUF "GUEST   TXT" dsfs.dsfs_delete_file 0 neq if 15 ret end
    DIR_BUF dsfs.dsfs_dir_load 0 neq if 16 ret end
    DIR_BUF "GUEST   TXT" dsfs.dsfs_find 0 ge if 17 ret end
    DIR_BUF dsfs.dsfs_count 1 neq if 18 ret end
    0 ret
;

main
