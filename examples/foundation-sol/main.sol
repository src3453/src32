!abi_compat "Foundation" 1
!include "../../sol/foundation/libfnd.sol"

!var work
!var file
!var child
!var status
!var i
!var seen_lo
!var seen_hi
!var seen_unicode
!var entries

# 失敗を成功として続けず、実際の status の説明を表示して終了する。
fn check (s) :
    s 0 lt if
        s fnd_strerror fnd_con_write drop
        "\n" fnd_con_write drop
        work fnd_mem_free
        s fnd_exit
    end
    retn
;

fn same_text (a b) :
    while
        a ldb b ldb neq if 1 ret end
        a ldb 0 eq if 0 ret end
        a 1 add >a
        b 1 add >b
        0
    end
;

# work: out handle +0, byte count +4, child exit code +8, buffer +64。
fnd_init check
16384 fnd_mem_alloc >work
work 0 eq if FND_E_NO_MEMORY fnd_exit end
"sol libfnd sample\n" fnd_con_write check
work work 4 add fnd_con_size check
work ld 80 neq if -108 fnd_exit end
work 4 add ld 30 neq if -109 fnd_exit end

# cwd と out 引数。文字列は NUL 終端、byte count は別の u32。
"/bin" fnd_cwd_set check
work 64 add 128 work 4 add fnd_cwd_get check
"cwd: " fnd_con_write check
work 64 add fnd_con_write check
"\n" fnd_con_write check

# 存在しないパス: out handle は失敗時に変更されない。
1234 work st
"/missing" FND_OPEN_READ FND_MODE_DEFAULT work fnd_file_open >status
status FND_E_NOT_FOUND neq if -100 fnd_exit end
work ld 1234 neq if -101 fnd_exit end
"missing: " fnd_con_write check
status fnd_strerror fnd_con_write check
"\n" fnd_con_write check

# 実際の WAX を read して magic を確認し、必ず close する。
"/bin/echo.wax" FND_OPEN_READ FND_MODE_DEFAULT work fnd_file_open check
work ld >file
file work 128 add fnd_stream_stat >status
status 0 lt if file fnd_close drop status check end
work 128 add FND_STAT_SIZE_LO add ld 0 le if file fnd_close drop -111 fnd_exit end
file work 64 add 4 work 4 add fnd_read >status
file fnd_close check
status check
work 4 add ld 4 neq if -102 fnd_exit end
work 64 add ld 0x57415800 neq if -103 fnd_exit end
"read/close: WAX magic OK\n" fnd_con_write check

# FS2 上へ5001 bytesを書き、4096境界をまたいで分割・再読込する。
0 >i
while
    i 251 mod work 2048 add i add stb
    i 1 add >i
    i 5001 lt
end
"/sol-result.bin" FND_OPEN_READ FND_OPEN_WRITE or FND_OPEN_CREATE or FND_OPEN_EXCL or
FND_MODE_DEFAULT work fnd_file_open check
work ld >file
file work 2048 add 5001 work 4 add fnd_write >status
status 0 lt if file fnd_close drop status check end
work 4 add ld 5001 neq if file fnd_close drop -114 fnd_exit end
file fnd_sync >status
file fnd_close check
status check
"/sol-result.bin" FND_OPEN_READ FND_MODE_DEFAULT work fnd_file_open check
work ld >file
1234 work 4 add st
file "x" 1 work 4 add fnd_write FND_E_ACCESS neq if file fnd_close drop -132 fnd_exit end
work 4 add ld 0 neq if file fnd_close drop -133 fnd_exit end
file work 8192 add 5001 work 4 add fnd_read >status
file fnd_close check
status check
work 4 add ld 5001 neq if -115 fnd_exit end
0 >i
while
    work 8192 add i add ldb 255 and i 251 mod neq if -116 fnd_exit end
    i 1 add >i
    i 5001 lt
end
"write/read/close: 5001 bytes OK\n" fnd_con_write check
"read-only stream: access error OK\n" fnd_con_write check

# 複数応答に分かれる65件を1件ずつ返す。順序は仮定せず重複・欠落も検査する。
"/data/sol-fixtures" work fnd_dir_open check
work ld >file
# 応答の途中で rewind しても、古いキャッシュの残りを返さない。
file work 512 add fnd_dir_next FND_OK neq if -134 fnd_exit end
file fnd_dir_rewind check
0 >seen_lo
0 >seen_hi
0 >seen_unicode
0 >entries
0 >status
while
    file work 512 add fnd_dir_next >status
    status FND_OK eq if
        work 512 add FND_DIRENT_NAME_BYTES add ld 13 eq if
            work 512 add FND_DIRENT_NAME_CHARS add ld 7 neq if -117 fnd_exit end
            work 512 add FND_DIRENT_NAME add "日本語.txt" same_text if else -135 fnd_exit end
            seen_unicode 0 neq if -118 fnd_exit end
            1 >seen_unicode
        else
            work 512 add FND_DIRENT_NAME_BYTES add ld 11 neq if -119 fnd_exit end
            work 512 add FND_DIRENT_NAME add 5 add ldb 48 sub 10 mul
            work 512 add FND_DIRENT_NAME add 6 add ldb 48 sub add >i
            i 0 lt if -120 fnd_exit end
            i 64 ge if -121 fnd_exit end
            i 32 lt if
                seen_lo 1 i shl and 0 neq if -122 fnd_exit end
                seen_lo 1 i shl or >seen_lo
            else
                seen_hi 1 i 32 sub shl and 0 neq if -123 fnd_exit end
                seen_hi 1 i 32 sub shl or >seen_hi
            end
        end
        entries 1 add >entries
    end
    status FND_OK eq
end
status FND_END neq if file fnd_close drop status check -124 fnd_exit end
entries 65 neq if file fnd_close drop -125 fnd_exit end
seen_lo 0xFFFFFFFF neq if -126 fnd_exit end
seen_hi 0xFFFFFFFF neq if -127 fnd_exit end
seen_unicode 1 neq if -128 fnd_exit end
# END は出力を変えない。rewind はキャッシュとサービスの両方を戻す。
1234 work 512 add FND_DIRENT_NAME_BYTES add st
file work 512 add fnd_dir_next FND_END neq if -129 fnd_exit end
work 512 add FND_DIRENT_NAME_BYTES add ld 1234 neq if -130 fnd_exit end
file fnd_dir_rewind check
file work 512 add fnd_dir_next FND_OK neq if -136 fnd_exit end
file fnd_close check
# 途中まで読んで閉じても、キャッシュのヒープ領域を再利用できる。
0 fnd_mem_grow work 12 add st
0 >i
while
    "/data/sol-small" work fnd_dir_open check
    work ld >file
    file work 512 add fnd_dir_next FND_OK neq if -137 fnd_exit end
    file fnd_close check
    i 1 add >i
    i 4 lt
end
0 fnd_mem_grow work 12 add ld neq if -138 fnd_exit end
# 空ディレクトリ、close後のハンドル再利用、エラー時の出力保持も検査。
"/sol-empty" FND_MODE_DEFAULT fnd_dir_make check
"/sol-empty" work fnd_dir_open check
work ld >file
file work 512 add fnd_dir_next FND_END neq if -131 fnd_exit end
file fnd_close check
"/sol-empty" fnd_dir_remove check
"/sol-result.bin" fnd_file_delete check
"directory: 65 entries, UTF-8, rewind, END OK\n" fnd_con_write check
"cleanup: file and directory removed\n" fnd_con_write check

# C の fnd_exec_t は実体が必要。cwd/stream の各欄の 0 は継承を意味する。
0 work 16 add st
0 work 20 add st
0 work 24 add st
0 work 28 add st
0 work 32 add st
0 work 36 add st
0 work 40 add st
2 work 16 add FND_EXEC_ARGC add st
work 48 add work 16 add FND_EXEC_ARGV add st
"/bin/sol-child.wax" work 48 add st
"from-parent" work 52 add st
"/bin/sol-child.wax" work 16 add work fnd_proc_exec check
work ld >child
child work 8 add fnd_proc_wait check
work 8 add ld 17 neq if -106 fnd_exit end
"child: exit 17 (wait closed handle)\n" fnd_con_write check

# コンソールも同じ stream write。status=0 と put の byte 数を別々に扱う。
fnd_std_out "stream write OK\n" 16 work 4 add fnd_write check
work 4 add ld 16 neq if -107 fnd_exit end
work fnd_mem_free
"sample: OK\n" fnd_con_write drop
0 fnd_exit
