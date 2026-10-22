!abi_compat "Foundation" 1
!include "../../sol/foundation/libfnd.sol"

!var work
!var file
!var child
!var status

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

# work: out handle +0, byte count +4, child exit code +8, buffer +64。
fnd_init check
512 fnd_mem_alloc >work
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

# bootfs は読み取り専用。成功したようには表示しない。
"/bin/echo.wax" FND_OPEN_WRITE FND_MODE_DEFAULT work fnd_file_open >status
status 0 eq if work ld fnd_close check -104 fnd_exit end
status FND_E_READ_ONLY neq if status check -112 fnd_exit end
"write-open: " fnd_con_write check
status fnd_strerror fnd_con_write check
"\n" fnd_con_write check

# bootfs は列挙未実装。フル fs ができたら dir_read/rewind/close も使える。
"/bin" work fnd_dir_open >status
status 0 eq if work ld fnd_close check -105 fnd_exit end
status FND_E_UNSUPPORTED neq if status check -113 fnd_exit end
"directory: " fnd_con_write check
status fnd_strerror fnd_con_write check
"\n" fnd_con_write check

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
