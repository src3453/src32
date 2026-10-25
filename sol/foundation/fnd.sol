# Foundation OS ABI v1 の sol ラッパー (doc/spec_Foundation.md §13.3)。
# !abi_compat "Foundation" のプログラムから !include して使う。
# 各ラッパーは「SYSCALL は R1 以外のレジスタを保存する」(§7) に依存し、
# 引数をフレームから R1..R4 に読み込んで SYSCALL を実行し、R1 を返す。
# 成功は 0 以上、エラーは負の FND_E_* 。成功 0 は sol の真 0 と一致するので
# `status if ... end` は成功時に then 節を実行する。ハンドルや byte 数を返す
# 呼び出しは `0 lt`（負なら真 = 0）で判定してから使う。
!abi_compat "Foundation" 1

# §4 エラーコード
!const FND_OK 0
!const FND_E_BAD_CALL -1
!const FND_E_BAD_ARG -2
!const FND_E_BAD_HANDLE -3
!const FND_E_ACCESS -4
!const FND_E_NO_MEMORY -5
!const FND_E_LIMIT -6
!const FND_E_PEER_GONE -7
!const FND_E_TIMEOUT -8
!const FND_E_WOULD_BLOCK -9
!const FND_E_NOT_FOUND -10
!const FND_E_EXISTS -11
!const FND_E_FORMAT -12
!const FND_E_UNSUPPORTED -13
!const FND_E_ADDRESS_CONFLICT -14
!const FND_E_REGION_LIMIT -15
!const FND_E_IO -16
!const FND_E_NO_MEDIA -17
!const FND_E_READ_ONLY -18
!const FND_E_BUSY -19
!const FND_E_KILLED -20
!const FND_E_DEPENDENCY -21

# §7.1 タイムアウト
!const FND_POLL 0
!const FND_FOREVER 0xFFFFFFFF

# §8.1 fnd_msg (32 bytes)
!const FND_MSG_OP 0
!const FND_MSG_FLAGS 4
!const FND_MSG_A0 8
!const FND_MSG_A1 12
!const FND_MSG_A2 16
!const FND_MSG_A3 20
!const FND_MSG_BUF 24
!const FND_MSG_LEN 28
!const FND_MSG_SIZE 32
!const FND_MSG_XFER_HANDLE 1
# §8.1 fnd_recv (64 bytes)
!const FND_RECV_MSG 0
!const FND_RECV_DATA 32
!const FND_RECV_CAP 36
!const FND_RECV_TOKEN 40
!const FND_RECV_BADGE 44
!const FND_RECV_REPLY_CAP 48
!const FND_RECV_TOTAL 52
!const FND_RECV_TIMEOUT 56
!const FND_RECV_SIZE 64
!const FND_IPC_MAX_DATA 4096
# §8.5 通知
!const FND_NOTIFY_IRQ 0xFFFF0001
!const FND_NOTIFY_CLIENT_GONE 0xFFFF0002

# §9.1 ケーパビリティ
!const FND_CAP_PRIO 1
!const FND_CAP_IRQ 2
!const FND_CAP_DISK 4
!const FND_CAP_SVC 8
!const FND_CAP_MMIO 16
# §9.2 fnd_spawn (48 bytes)
!const FND_SPAWN_IMAGE 0
!const FND_SPAWN_IMAGE_LEN 4
!const FND_SPAWN_STARTUP 8
!const FND_SPAWN_STARTUP_LEN 12
!const FND_SPAWN_HANDLES 16
!const FND_SPAWN_HANDLE_COUNT 20
!const FND_SPAWN_PRIORITY 24
!const FND_SPAWN_CAPS 28
!const FND_SPAWN_GRANTS 32
!const FND_SPAWN_GRANT_COUNT 36
!const FND_SPAWN_NAME 40
!const FND_SPAWN_SIZE 48
!const FND_PRIO_DRIVER 0
!const FND_PRIO_SERVICE 1
!const FND_PRIO_APP 2
!const FND_PRIO_IDLE 3
# §9.3 起動ブロック
!const FND_STARTUP_MAGIC_VALUE 0x464E4453
!const FND_STARTUP_MAGIC 0
!const FND_STARTUP_VERSION 4
!const FND_STARTUP_SIZE 8
!const FND_STARTUP_ARGC 12
!const FND_STARTUP_ARGV_OFF 16
!const FND_STARTUP_CWD_OFF 20
!const FND_STARTUP_ROLE_COUNT 24
!const FND_STARTUP_ROLES_OFF 28
!const FND_ROLE_STDIN 1
!const FND_ROLE_STDOUT 2
!const FND_ROLE_STDERR 3
!const FND_ROLE_FS 4
!const FND_ROLE_CONSOLE 5
# §9 sys_info / time_get のセレクタ
!const FND_SYSINFO_SYSTEM 0
!const FND_SYSINFO_PROCESSES 1
!const FND_TIME_TICK_LO 0
!const FND_TIME_TICK_HI 1
!const FND_TIME_TICK_HZ 2
!const FND_TIME_RTC_LO 3
!const FND_TIME_RTC_HI 4
# §10.4 fnd_blk (24 bytes)
!const FND_BLK_DRIVE 0
!const FND_BLK_OP 4
!const FND_BLK_LBA 8
!const FND_BLK_COUNT 12
!const FND_BLK_BUF 16
!const FND_BLK_SIZE 24
!const FND_BLK_READ_OP 1
!const FND_BLK_WRITE_OP 2
!const FND_BLK_IDENTIFY_OP 3

# §9 システムコールのラッパー。フレームは [R26+0] 旧フレームベース、
# [R26+4] 戻り先、[R26+8] ローカル r、[R26+12] 以降が引数（左から）。

# SYSCALL 0x00 sys_info(sel, buf, cap) -> 必要な byte 数（cap まではコピー済み）
fn fnd_sys_info (sel buf cap) :
    local r
!asm
    LD R1, [R26 + 12]    ; sel
    LD R2, [R26 + 16]    ; buf
    LD R3, [R26 + 20]    ; cap
    SYSCALL 0x00
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x01 ipc_call(ch, req, rep) -> 配送結果。サーバーの処理結果は rep の FND_MSG_OP
fn fnd_ipc_call (ch req rep) :
    local r
!asm
    LD R1, [R26 + 12]    ; ch
    LD R2, [R26 + 16]    ; req
    LD R3, [R26 + 20]    ; rep
    SYSCALL 0x01
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x02 ipc_recv(port, rcv) -> 0
fn fnd_ipc_recv (port rcv) :
    local r
!asm
    LD R1, [R26 + 12]    ; port
    LD R2, [R26 + 16]    ; rcv
    SYSCALL 0x02
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x03 ipc_reply(token, msg) -> 0
fn fnd_ipc_reply (token msg) :
    local r
!asm
    LD R1, [R26 + 12]    ; token
    LD R2, [R26 + 16]    ; msg
    SYSCALL 0x03
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x04 ipc_reply_recv(token, msg, port, rcv) -> 0
fn fnd_ipc_reply_recv (token msg port rcv) :
    local r
!asm
    LD R1, [R26 + 12]    ; token
    LD R2, [R26 + 16]    ; msg
    LD R3, [R26 + 20]    ; port
    LD R4, [R26 + 24]    ; rcv
    SYSCALL 0x04
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x05 port_create() -> PORT ハンドル
fn fnd_port_create () :
    local r
!asm
    SYSCALL 0x05
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x06 chan_create(port, badge) -> CHANNEL ハンドル
fn fnd_chan_create (port badge) :
    local r
!asm
    LD R1, [R26 + 12]    ; port
    LD R2, [R26 + 16]    ; badge
    SYSCALL 0x06
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x07 handle_close(h) -> 0
fn fnd_handle_close (h) :
    local r
!asm
    LD R1, [R26 + 12]    ; h
    SYSCALL 0x07
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x08 handle_dup(h) -> ハンドル
fn fnd_handle_dup (h) :
    local r
!asm
    LD R1, [R26 + 12]    ; h
    SYSCALL 0x08
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x09 thread_create(entry, sp, arg) -> THREAD ハンドル
fn fnd_thread_create (entry sp arg) :
    local r
!asm
    LD R1, [R26 + 12]    ; entry
    LD R2, [R26 + 16]    ; sp
    LD R3, [R26 + 20]    ; arg
    SYSCALL 0x09
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x0A thread_exit(code) -> 戻らない
fn fnd_thread_exit (code) :
!asm
    LD R1, [R26 + 8]    ; code
    SYSCALL 0x0A
!end
    retn
;

# SYSCALL 0x0B yield() -> 0
fn fnd_yield () :
    local r
!asm
    SYSCALL 0x0B
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x0C sleep(ticks) -> 0
fn fnd_sleep (ticks) :
    local r
!asm
    LD R1, [R26 + 12]    ; ticks
    SYSCALL 0x0C
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x0D wait(h, timeout, code_ptr) -> 0
fn fnd_wait (h timeout code_ptr) :
    local r
!asm
    LD R1, [R26 + 12]    ; h
    LD R2, [R26 + 16]    ; timeout
    LD R3, [R26 + 20]    ; code_ptr
    SYSCALL 0x0D
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x10 proc_spawn(spawn) -> PROCESS ハンドル
fn fnd_proc_spawn (spawn) :
    local r
!asm
    LD R1, [R26 + 12]    ; spawn
    SYSCALL 0x10
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x14 proc_fork() -> 親: PROCESS ハンドル、子: 0
fn fnd_proc_fork () :
    local r
!asm
    SYSCALL 0x14
    ST R1, [R26 + 8]
!end
    r ret
;

# SYSCALL 0x11 proc_exit(code) -> 戻らない
fn fnd_proc_exit (code) :
!asm
    LD R1, [R26 + 8]    ; code
    SYSCALL 0x11
!end
    retn
;

# SYSCALL 0x12 proc_kill(h) -> 0
fn fnd_proc_kill (h) :
    local r
!asm
    LD R1, [R26 + 12]    ; h
    SYSCALL 0x12
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x13 proc_startup(buf, cap) -> 起動ブロックの byte 数
fn fnd_proc_startup (buf cap) :
    local r
!asm
    LD R1, [R26 + 12]    ; buf
    LD R2, [R26 + 16]    ; cap
    SYSCALL 0x13
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x18 mem_grow(delta) -> 伸長前のヒープ末尾
fn fnd_mem_grow (delta) :
    local r
!asm
    LD R1, [R26 + 12]    ; delta
    SYSCALL 0x18
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x20 irq_bind(irq, port) -> 0
fn fnd_irq_bind (irq port) :
    local r
!asm
    LD R1, [R26 + 12]    ; irq
    LD R2, [R26 + 16]    ; port
    SYSCALL 0x20
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x21 irq_ack(irq) -> 0
fn fnd_irq_ack (irq) :
    local r
!asm
    LD R1, [R26 + 12]    ; irq
    SYSCALL 0x21
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x28 blk_io(blk) -> 0
fn fnd_blk_io (blk) :
    local r
!asm
    LD R1, [R26 + 12]    ; blk
    SYSCALL 0x28
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x30 time_get(sel) -> 値
fn fnd_time_get (sel) :
    local r
!asm
    LD R1, [R26 + 12]    ; sel
    SYSCALL 0x30
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x31 svc_register(name, port) -> 0
fn fnd_svc_register (name port) :
    local r
!asm
    LD R1, [R26 + 12]    ; name
    LD R2, [R26 + 16]    ; port
    SYSCALL 0x31
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x32 svc_lookup(name) -> CHANNEL ハンドル
fn fnd_svc_lookup (name) :
    local r
!asm
    LD R1, [R26 + 12]    ; name
    SYSCALL 0x32
    ST R1, [R26 + 8]          ; r
!end
    r ret
;

# SYSCALL 0x3F debug_write(buf, len) -> len
fn fnd_debug_write (buf len) :
    local r
!asm
    LD R1, [R26 + 12]    ; buf
    LD R2, [R26 + 16]    ; len
    SYSCALL 0x3F
    ST R1, [R26 + 8]          ; r
!end
    r ret
;
