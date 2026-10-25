# pkcl 仕様（POSIX Kernel Compatibility Layer）v0.1

pkcl（発音は paxel）は、POSIX の関数呼び出しを Foundation の IPC・libfnd 呼び出しへ**コンパイル時にリンクして**変換する層です。Fuchsia の fdio に相当します。POSIX のコード（最終目標は BusyBox）を、カーネルや fs を POSIX 化せずに Foundation 上で動かすために置きます。

状態：v0.1 は設計と最小実装（M0、§10）です。この文書の「実装済み」と書いた部分だけが動きます。

## 1. 原則

1. **カーネルが本質的に持つ機能だけを写す**。Foundation のカーネルは `fork`、シグナル、パイプ、UID、シンボリックリンクを持ちません。それらは pkcl が偽装した「動くふり」にせず、§6 の表のとおり、**エミュレーションできるもの**と**エラー（`ENOSYS` など）にするもの**に分けます。
2. **少しの変更で実装できるものはエミュレーションする**。ただし新しい仕組みを横に足さず、Foundation 側の契約（`spec_Foundation.md`、`spec_Foundation_API.md`）に沿うこと。契約を変える必要があるものは §7 のギャップとして Issue にし、pkcl はその間 `ENOSYS` を返します。
3. **コンパイル時の層**。pkcl は静的ライブラリ（ソース）とヘッダで、アプリケーションに直接リンクします。サービスやプロセスを追加しません。カーネルの ABI は変えません。
4. **libfnd の上に積む**。pkcl は Foundation を `<foundation/fnd.h>`（WAL `fnd`）経由でだけ使い、syscall を直接呼ぶのはハンドルの複製（`handle_dup`）など libfnd に露出していない 1 対 1 の操作だけです。
5. **GPL のコードは含めない**。BusyBox（GPL-2.0）をこのリポジトリに置きません（§8.1）。

## 2. 全体像

```
BusyBox / POSIX アプリ        （外部ソース）
        │  POSIX 呼び出し
libc（stdio、string、qsort…）  （Newlib を予定、§9）
        │  POSIX I/O・プロセス・時刻など（open/read/write/stat/posix_spawn…）
pkcl                          foundation/lib/pkcl（静的ライブラリ）
        │  fnd_file_open / fnd_read / fnd_proc_exec …
libfnd（WAL fnd）             foundation/lib/fnd
        │  IPC（ipc_call）と syscall
Foundation カーネル・fs・console サービス
```

fdio と同じく、pkcl の仕事は「ファイル記述子の表」と「POSIX の意味をサービスのプロトコルに合わせること」です。ストリームの位置・権限・パス解決はサービス（fs）が持ち、pkcl は持ちません（API §6.2：パスは書き換えずに送る）。

## 3. 構成

### 3.1 ファイル

| 場所 | 内容 |
|---|---|
| `foundation/include/pkcl/` | 内部境界 `pkcl/pkcl.h` |
| `foundation/include/pkcl-nolibc/` | libc なしで使う POSIX ヘッダ（`errno.h`、`fcntl.h`、`unistd.h`、`sys/types.h`、`sys/stat.h`…） |
| `foundation/include/pkcl-posix/` | Newlib のヘッダに足す POSIX ヘッダ（`dirent.h`、`termios`、`poll.h`、`sys/socket.h`、`netdb.h`…）と、Newlib が隠す宣言（`lstat`、`nanosleep`、`getline`…）の補い |
| `foundation/lib/pkcl/` | 実装（`errno.c`、`fd.c`、`fs.c`、`crt.c`） |
| `foundation/lib/pkcl/newlib/` | Newlib の OS 層（`_open`、`_sbrk`、`_gettimeofday`…）を pkcl に結ぶアダプタ |
| `foundation/lib/pkcl/posix/` | libc ありで足す POSIX（`DIR`、端末・`ioctl`・`poll`、ソケットの失敗スタブ、プロセス・時刻・シグナルの縮退、ユーザー DB、`fnmatch`） |
| `tools/pkcl/` | BusyBox のビルド（`busybox.py`、`busybox.config`、`busybox_stubs.c`） |
| `foundation/tests/pkcl/` | 実際の fs・console に対するゲストテストと BusyBox のテスト |

### 3.2 ビルド

pkcl は**ソースを一緒にコンパイルして静的にリンク**します（WAL にはしません）。ファイル記述子の表という可変な大域データを持ち、WAL の静的データ制約（アドレスを含むデータは不可）に触れずに済ませるためです。標準の指定は次のとおりです。

```
python tools/llvm-src32/driver.py app.c foundation/lib/pkcl/*.c \
    -I foundation/include/pkcl -I foundation/include -I sdk/cpt32/include \
    --entry pkcl_start --wax3 --import-wal root/lib/fnd.1.wal ...
```

- libc なしなら `-I foundation/include/pkcl-nolibc` を先頭に置きます。Newlib を使うときは `--newlib-profile posix`（§9）と `-I foundation/include/pkcl-posix`、`-DPKCL_LIBC` を指定します。
- WAL `fnd` を import する WAX v3 が基本です。静的 libfnd（WAX v2）でも動きます。

### 3.3 起動

`--entry pkcl_start` を指定すると、`crt.c` の `pkcl_start` が `fnd_init`、`pkcl_init`（fd 0〜2 の準備）、argv の組み立てを行い、`int main(int argc, char **argv)` を呼んで、その戻り値で `fnd_exit` します。`environ` は現状空です（§7 G2）。`pkcl_init` は POSIX 呼び出しの先頭でも遅延して呼ばれるので、`main(void)` と `fnd_init` の既存形式のアプリでも pkcl の関数を使えます。

## 4. 型と ABI の方針

| 型 | 定義 | 理由 |
|---|---|---|
| `off_t` | `int32_t` | `llvm-src32` の `i64` は使えるが、BusyBox は `CONFIG_LFS=n` で組める。FS2 は 64 bit のサイズを持つので、2 GiB 以上は `EOVERFLOW`。64 bit 化は §10 M5 |
| `time_t` | `int64_t` | fnd_stat の時刻は i64 秒 |
| `mode_t`、`uid_t`、`gid_t`、`pid_t` | 32 bit | |
| `errno` | 単一のグローバル `int errno` | スレッドごとにはしない（スレッドは `thread_create` で作れるが pkcl のスレッド対応は M5） |
| 数値定数 | Newlib の `sys/errno.h`、`_default_fcntl.h` と同じ値 | libc を Newlib にしたとき食い違わないため |

## 5. エラーの変換

Foundation の `fnd_status`（負値）を `pkcl_errno` で errno に直します。

| Foundation | errno |
|---|---|
| `BAD_CALL`、`UNSUPPORTED` | `ENOSYS` |
| `BAD_ARG`、`BAD_PATH` | `EINVAL` |
| `BAD_HANDLE` | `EBADF` |
| `ACCESS` | `EACCES` |
| `NO_MEMORY`、`ADDRESS_CONFLICT`、`REGION_LIMIT` | `ENOMEM` |
| `LIMIT` | `EMFILE`（`getcwd` の容量不足は `ERANGE`） |
| `TIMEOUT` | `ETIMEDOUT` |
| `WOULD_BLOCK` | `EAGAIN` |
| `NOT_FOUND`、`DEPENDENCY` | `ENOENT` |
| `EXISTS` | `EEXIST` |
| `FORMAT` | `ENOEXEC` |
| `NO_MEDIA` | `ENODEV` |
| `READ_ONLY` | `EROFS` |
| `BUSY` | `EBUSY` |
| `KILLED`、`INTERRUPTED` | `EINTR` |
| `NOT_DIR` / `IS_DIR` / `NOT_EMPTY` | `ENOTDIR` / `EISDIR` / `ENOTEMPTY` |
| `NAME_TOO_LONG` | `ENAMETOOLONG` |
| `NO_SPACE` | `ENOSPC` |
| `CROSS_DEVICE` | `EXDEV` |
| 上記以外の負値 | `EIO` |

`FND_E_INTERRUPTED`（Ctrl+C による入力待ちの中断、API §8.4）が `EINTR` になるので、BusyBox のシグナル処理（§6.5）と接続できます。

## 6. POSIX 面の対応表

「状態」：**実装済み**（v0.1）／**予定**（Issue あり、Foundation の変更なし）／**要変更**（§7 のギャップが先）／**不可**（恒久的にエラー）。

### 6.1 ファイル記述子と I/O

| 関数 | 方式 | 状態 |
|---|---|---|
| `open`、`creat`、`close`、`read`、`write` | `fnd_file_open`／`fnd_read`／`fnd_write`／`handle_close`。1 記述子 = 1 CHANNEL | 実装済み |
| `lseek` | `ST_SEEK`。コンソール・シーク不可は `ESPIPE`。32 bit を超える位置は `EOVERFLOW` | 実装済み |
| `dup`、`dup2`、`fcntl(F_DUPFD/GETFD/SETFD/GETFL/SETFL)` | `handle_dup`。位置はサービスが持つので dup 間で共有される（POSIX どおり） | 実装済み（`dup3`、`F_DUPFD_CLOEXEC` は予定） |
| `ftruncate`、`truncate`、`fsync` | `ST_TRUNCATE`、`ST_SYNC` | 実装済み |
| `isatty` | `ST_STAT` の type = `CONSOLE` | 実装済み |
| `fstat` | `ST_STAT` | 実装済み |
| `pread`、`pwrite`、`readv`、`writev` | `lseek` ＋ read/write で代用できるが原子的ではない | 予定 |
| `pipe`、`pipe2`、`mkfifo` | ユーザー空間の `pipe` サービス（IPC ＋ハンドル転送。カーネル変更なし）。§7 G3 | 予定 |
| `select`、`poll` | コンソールは `CON_READ_KEY(NOWAIT)`、ファイルは常に ready、pipe はサービスの op | 予定 |
| `mmap`、`munmap` | 無い。`MAP_PRIVATE` ファイル読み込みだけは read で模倣できるが、BusyBox は不要 | 不可 |
| `fcntl(F_SETLK)`、`flock` | ロックなし | 不可（`ENOSYS`） |

`O_NONBLOCK` は記録するだけで、コンソールの `read` は常にブロックします（M4 の `poll` と一緒に扱う）。`O_APPEND` は `FND_OPEN_APPEND`（サービス側）で実現し、`F_SETFL` で後から変えても効きません。

### 6.2 パスとディレクトリ

| 関数 | 方式 | 状態 |
|---|---|---|
| `stat`、`lstat`（= `stat`）、`access` | `fnd_path_stat`。§6.3 の対応 | 実装済み |
| `mkdir`、`rmdir`、`unlink`、`rename` | `fnd_dir_make` 等。`unlink(ディレクトリ)` は `EISDIR`。`rename` は置換先が通常ファイルのときだけ「削除してから rename」で上書きを模倣（途中失敗で両方を失う窓がある。ディレクトリへの上書きは `EEXIST` のまま） | 実装済み |
| `chdir`、`getcwd` | `fnd_cwd_set`／`fnd_cwd_get`（cwd は fs のセッションが持つ） | 実装済み |
| `chmod`、`umask` | `FS_CHMOD`。`umask` は pkcl が保持して `open`／`mkdir` の mode に適用（FS2 に umask はない） | 実装済み |
| `opendir`、`readdir`、`rewinddir`、`closedir`、`dirfd` | `fnd_dir_open`／`fnd_dir_next`／`fnd_dir_rewind`。`d_ino` は 0、`d_type` は stat の type から | 予定（M1） |
| `open(ディレクトリ, O_RDONLY)` | 現状は `EISDIR`（`FS_OPEN` の仕様）。BusyBox は `opendir` を使う | 予定（`O_DIRECTORY`、M1） |
| `chown`、`fchown` | UID は 0 固定なので、所有者を変えない呼び出しだけ成功 | 予定 |
| `utime`、`utimes`、`futimens` | `FS_UTIME` がない。§7 G5。それまで `touch` は新規作成だけ可能（pkcl は `ENOSYS`） | 要変更 |
| `symlink`、`link`、`readlink` | FS2 にない | 不可（`ENOSYS`） |
| `realpath` | libc の実装（`getcwd` ＋ `lstat`）に任せる | 予定 |
| `statvfs`、`statfs` | `FS_VOLUME_INFO` | 予定（`df` 用） |

### 6.3 `struct stat` の対応

| POSIX | Foundation |
|---|---|
| `st_mode` の型 | `FILE`→`S_IFREG`、`DIR`→`S_IFDIR`、`DEVICE`/`CONSOLE`→`S_IFCHR` |
| `st_mode` の権限 | FS2 の owner 3 bit と other 3 bit（FS2 §8）→ `user` と `other`。`group` は other と同じに見せる。逆方向（`chmod`、`open(O_CREAT)`、`mkdir`）は group を捨てる |
| 読み取り専用の mount | `FND_STAT_READ_ONLY` のとき W bit を落とす |
| `st_size` | `size_lo`（`size_hi` ≠ 0 や 2 GiB 以上は `EOVERFLOW`） |
| `st_mtime`、`st_ctime` | `modified_sec`、`changed_sec`（`atime` は mtime と同じ） |
| `st_ino`、`st_dev` | 0、1。**FS2 に inode を返す欄がない**ので、BusyBox の「同じファイルか」の判定（`cp`、`mv`）は偽陽性になりうる。§7 G4 |
| `st_nlink`、`st_gid`、`st_uid` | 1、0、`uid` |

### 6.4 プロセス

Foundation のプロセスは `fnd_proc_exec`（= `proc_spawn`）で作る新しいイメージで、`fork` に相当する操作はありません（メモリのコピーも COW の仕組みもプロセスの複製にはない）。

| 関数 | 方式 | 状態 |
|---|---|---|
| `_exit`、`exit` | `fnd_exit`（`exit` と `atexit` は libc） | `_exit` 実装済み |
| `posix_spawn`、`posix_spawnp` | `fnd_proc_exec`。file actions（`dup2`/`close`/`open`）は子の `std_in/out/err` に渡すハンドルへ翻訳（0〜2 だけ）。`posix_spawnp` は `PATH` を `fnd_path_stat` で探す | 予定（M2） |
| `waitpid`、`wait` | pkcl の子プロセス表（pid → PROCESS ハンドル）と `fnd_proc_wait`。`WNOHANG` は `wait(h, 0, …)`（`WOULD_BLOCK`→0） | 予定（M2） |
| `execve`、`execv`、`execvp` | **代理 exec**：子を spawn して待ち、その終了コードで `_exit`。fd の継承は 0〜2 のみ。自分のハンドル（fd 3 以上）は渡らない | 予定（M2） |
| `system`、`popen` | `posix_spawn("/bin/sh")` ＋ `waitpid`（`popen` は pipe 後） | 予定 |
| `fork`、`vfork` | **不可**（`ENOSYS`）。BusyBox は NOMMU 構成と `posix_spawn` への置き換えで回避する（§8.3） | 不可 |
| `getpid`、`getppid` | カーネルに「自分の pid」を返す手段がない。§7 G1 | 要変更 |
| `kill`、`signal`、`sigaction`、`raise` | 非同期シグナルはない。§6.5 | 予定（M4） |
| `sleep`、`nanosleep`、`usleep` | `sleep`（tick は 10 ms 単位に切り上げ） | 予定（M4） |
| `getuid` 系 | 常に 0 | 実装済み |
| `setuid` 系、`chroot` | | 不可（`EPERM`／`ENOSYS`） |
| `getenv`、`setenv` | libc。環境の継承は §7 G2 | 要変更 |

### 6.5 シグナル

カーネルに非同期シグナルはありません。pkcl は次のことだけをエミュレーションします。

- `signal`/`sigaction` はハンドラを pkcl の表に記録する。`raise` と `kill(getpid(), sig)` はそのハンドラを同期して呼ぶ。
- `SIGINT`：`CON_BREAK_WAIT`（API §8.4）を待つ専用スレッドを pkcl が 1 本作り、Ctrl+C でハンドラを呼ぶ（呼び出しはそのスレッドの上なので、ハンドラは `volatile` のフラグを立てるだけにする。BusyBox の ash／hush はそう書かれている）。ブロック中の `read` は `FND_E_INTERRUPTED` → `EINTR` で戻る。
- `kill(pid, SIGKILL/SIGTERM)`：pkcl が spawn した子なら `fnd_proc_kill`。
- `SIGCHLD`、`SIGPIPE`、`alarm`、タイマーは作らない。

### 6.6 端末（termios）

`tcgetattr`/`tcsetattr` は、コンソールの 2 つの入力モードへ翻訳します。

| termios | コンソール |
|---|---|
| `ICANON` オン | `LINE`（行編集あり、エコーあり） |
| `ICANON` オフ | `RAW`（キーはそのまま、エコーなし。エコーするかはアプリ） |
| `ECHO`、`ISIG`、`VMIN/VTIME` | `ECHO` だけ off にできない（`LINE` では常にエコー）。`ISIG` は §6.5。`VMIN/VTIME` は `poll` との組み合わせ（M4） |

`ioctl(TIOCGWINSZ)` は `CON_GET_SIZE`。キー入力は `FND_KEY_*` から VT100 風のバイト列（矢印は `ESC [ A`…）に変換して `read` に返す必要がある（`vi` や `ash` の行編集用）。これは M4 の項目です。

### 6.7 時刻

| 関数 | 方式 |
|---|---|
| `time`、`gettimeofday`、`clock_gettime(CLOCK_REALTIME)` | `time_get(3, 4)`（RTC 秒）。ミリ秒は tick から補完 |
| `clock_gettime(CLOCK_MONOTONIC)`、`times`、`clock` | `time_get(0, 1)`（tick。`tick_hz` で換算し 1 tick = 1 ms と仮定しない） |
| `localtime`、`strftime` | libc。タイムゾーンは UTC 固定 |

## 7. Foundation 側のギャップ（pkcl が待つもの）

pkcl の方針は「少しの変更なら入れる」です。次は、それぞれ独立した小さな変更の候補です。**この PR では実装しません**（Issue にします）。

| ID | 欠けているもの | 提案する最小の変更 | 影響する POSIX |
|---|---|---|---|
| G1 | 自分の pid を知る手段 | `sys_info` に sel を 1 つ足す（自プロセスの pid と parent） | `getpid`、`getppid`、`kill(getpid())` |
| G2 | 環境変数の受け渡し | **カーネルは起動ブロックを解釈しない**（Foundation §9.3）ので、libfnd の規約として起動ブロック version 2 に `env_off`/`env_count` を足す。`fnd_exec_t` に `envp` を足す | `environ`、`getenv`、`execve` の環境 |
| G3 | パイプ | ユーザー空間の `pipe` サービス（`svc_register`、`FS_OPEN` 風の op で 2 本のハンドルを `XFER_HANDLE` で返す）。カーネル変更なし | `pipe`、`|`、`popen`、`mkfifo` |
| G4 | inode 番号 | `fnd_stat` の予約欄（offset 60）に FS2 の inode を入れる（FS2 の inode 番号。実装時に FS2 仕様で確認する） | `st_ino`、`cp`/`mv` の同一ファイル判定 |
| G5 | 時刻の更新 | `FS_UTIME` op（path、mtime、atime）。FS2 は mtime を持つ | `touch`、`cp -p`、`utimes` |
| G6 | シンボリックリンク／ハードリンク | FS2 の拡張が必要（影響大）。当面入れない | `ln`、`readlink` |
| G7 | `setjmp`/`longjmp` | `tools/llvm-src32` の README に言及がなく、対応は未確認。ash が必須。未対応なら runtime のアセンブリ関数で実装する | ash、`libc` 全般 |
| G8 | 64 bit のファイルオフセット | `off_t` を 64 bit にする（i64 は対応済み）。API の `fnd_seek` を libfnd に足す（現在は pkcl が `ST_SEEK` を直接呼ぶ） | 2 GiB 超のファイル |
| G9 | `ST_READ` のノンブロッキング | コンソールはあるが、pipe と通常のストリームにない | `O_NONBLOCK`、`poll` |

## 8. BusyBox への道

### 8.1 取り込み方針（GPL）

BusyBox は GPL-2.0 なので、**リポジトリには含めません**。Newlib（`--newlib-source`）と同じく、利用者が公式ソースを別途用意し、パスを指定してビルドします。リポジトリに置くのは、pkcl・ビルド用のスクリプト・`.config` の差分（設定値の一覧）・ドキュメントだけです。BusyBox のソースへの patch は GPL の対象になるため、同梱せず、必要なら別リポジトリに置きます。patch を要しない構成（設定、`-D` オプション、pkcl 側の互換関数）を優先します。

### 8.2 必要な POSIX 面

最初の対象 applet を `cat`、`echo`、`ls`、`cp`、`mv`、`rm`、`mkdir`、`rmdir`、`head`、`tail`、`wc`、`sort`、`uniq`、`grep`、`sed`、`sleep`、`true`、`false`、`pwd`、`env`、`basename`、`dirname`、`tr`、`cut`、`od`/`hexdump`、`test`、`printf`、`ash`（または `hush`）とします。必要な面と pkcl の状態：

| 必要なもの | 状態 |
|---|---|
| ファイル I/O、`stat`、`mkdir` 等 | 実装済み（§6.1〜6.3） |
| `DIR`（`ls`、`cp -r`、`rm -r`） | 実装済み（`posix/dirent.c`） |
| libc：`stdio`（`printf` 系、`fopen`、バッファ）、`strtol`、`qsort`、`getopt`、`regcomp`（`grep`、`sed`）、`locale` の最小 | 実装済み（Newlib の posix プロファイル、§9） |
| `setjmp`/`longjmp`（ash が必須） | 未対応。`posix/setjmp.c` は `setjmp` が 0 を返し `longjmp` は終了する仮実装（G7） |
| `posix_spawn`、`waitpid`（外部コマンド、`xargs`、`find -exec`） | `execve` 系は「子を起動して終了を待ち、終了コードで自分も終わる」代理実装。`vfork`／`waitpid` は失敗（M2 の残り） |
| `pipe`（シェルのパイプ、`popen`） | `ENOSYS`（G3、M4） |
| `termios`、`ioctl`、`signal`（対話シェル、`vi`、`top`） | termios／`TIOCGWINSZ` は console の LINE／RAW に対応。`sigaction` は登録を受け付けるだけで配送されない。`poll` は要求をそのまま準備済みとして返す（G9） |
| `getpwnam` 等 | BusyBox の `USE_BB_PWD_GRP` で自前実装を使い、`/etc/passwd` を読む |
| 浮動小数点 | `llvm-src32` が SoftFloat で対応済み。BusyBox は `awk`、`dc` 以外で不要 |

### 8.3 `fork` を使わない構成

BusyBox の NOMMU 構成（`CONFIG_NOMMU`）と NOFORK／NOEXEC applet（`FEATURE_PREFER_APPLETS`、`FEATURE_SH_NOFORK`、`FEATURE_SH_STANDALONE`）を使い、`fork` を呼ぶ経路を無くします。外部コマンドは `posix_spawn("/bin/busybox", argv…)` の再実行（`BB_FORCE_APPLETS`風）で起動します。**パイプラインは複数プロセスを並行して動かし、pipe サービス越しにつなぐ**ことで実現します。ash の NOMMU サポートは「`vfork` ＋ exec」を前提としているため、`vfork` が使えない場合に ash を使えるかは M5 の調査項目です（hush は NOMMU でもより素直）。

### 8.4 ビルド

`tools/pkcl/busybox.py` が、`--busybox-source PATH` の外部ソース、`--newlib-source PATH` の Newlib、pkcl を `tools/llvm-src32/driver.py` に渡して `busybox.wax` を作ります。

```
python3 tools/pkcl/busybox.py --busybox-source /path/to/busybox-1.36.1 \
    --newlib-source /path/to/newlib-4.4.0.20231231 --fnd-wal root/lib/fnd.1.wal --out build/busybox
```

1. `allnoconfig` に `tools/pkcl/busybox.config` の差分を重ね、ホストの gcc で `make V=1` して生成ヘッダとコンパイルコマンド（`-I`／`-D`／`-include`）を集める。
2. 各翻訳単位を `driver.py -c` で LLVM IR にする（`-Os`、`gnu11`）。`-D__clang_major__=0` で Clang 向けのインラインアセンブリの分岐を避け、`-DBB_GLOBAL_CONST=` で `ptr_to_globals` を rodata に置かせない（U モードで書込みフォルトになる）。バリアの空 asm は IR から消す。
3. 全部を一度にリンクして `busybox.wax`（WAX v3、`fnd` WAL を import）を作る。

`/bin/busybox.wax` を置いて `busybox echo hi` のように呼びます（シンボリックリンクがないので applet 名のファイルは作らない。必要なら小さな WAX か shell 側のディスパッチ）。WAX は 3 MiB の `FND_EXEC_IMAGE_MAX` に収まる必要があり、依存 WAL を含む staging の合計上限も 3 MiB です。シェルのヒープにも丸ごと読み込まれるので、**サイズが applet の数を決める**。現在の構成は 1.04 MB です。サイズの内訳と抑えている手段は §9 を参照。

**設定が選ぶもの**：`CONFIG_NOMMU=y`、`CONFIG_LFS=n`（`off_t` は 32 bit）、`CONFIG_STATIC=y`、シェル（ash／hush）なし。applet は `cat echo true false pwd basename dirname head wc mkdir rmdir rm cp mv ls sleep test printf`。applet を足すにはサイズを確認すること（`tools/pkcl/busybox.py` のあとで `busybox.wax` が WAL とともに staging に収まること。テストでは従来の 1 MiB 未満もサイズ回帰の基準として維持する）。

## 9. libc の方針

pkcl は libc を持たず、Newlib（`--newlib-source` の外部ソース。同梱しない）を使います。`tools/llvm-src32` の Newlib 統合には 2 つのプロファイルがあります。

- `portable`（既定）：文字列・整数・`qsort`・nano allocator だけを、必要なものだけコンパイルする。
- `posix`（`--newlib-profile posix`）：`argz ctype errno locale misc reent search signal stdio stdlib string time` と `libm` のほぼ全部を一度 IR にして `--newlib-cache` に置き、**未解決シンボルの索引**から必要な翻訳単位だけをリンクする。OS 層（`posix/`、`sys/`、`machine/`）は pkcl（`foundation/lib/pkcl/newlib/syscalls.c`）が `_open` や `_sbrk` などを pkcl の POSIX 関数と `fnd_sys_mem_grow` に結ぶ。

posix プロファイルの判断（サイズが起動時のメモリ使用量に直結するため）：

| 判断 | 理由 |
|---|---|
| ライブラリは `-Os` | `-O0` では BusyBox が 1.8 MB |
| nano の scanf を使い、printf は full | nano の printf は `%lld` に対応しない（`ll` を解釈せず "ld" と出力する）。BusyBox の `printf` が必要とする |
| full の `_svfprintf_r`（文字列用）と整数専用の `_vfiprintf_r`／`_svfiprintf_r` を、`_vfprintf_r` への転送関数にする（`runtime/newlib/stdio_i.c`） | 同じ整形器が 3 つ入って約 130 KB 増える。文字列の FILE は fd を持たないので full の整形器がそのまま書ける |
| IR のエイリアスを転送関数に、`weak`／`extern_weak` を強いシンボルに書き換える | バックエンドにエイリアスと弱いシンボルがない。nano scanf の `_scanf_float` を常にリンクするため、BusyBox は `tools/pkcl/busybox_stubs.c` の失敗スタブで置き換えて strtod 一式（約 60 KB）を外す |
| `setjmp`／`longjmp` は仮実装 | G7 |

バックエンドに足した命令（最適化したコードが出す）：`llvm.fshl`／`llvm.fshr`（i32 は変数のシフト量、i64 は定数のシフト量）、`llvm.umul.with.overflow.i64`、`llvm.uadd.sat`（32 bit 以下）、`llvm.get.rounding`／`llvm.set.rounding`、グローバル初期値の `undef`／`poison`。Clang は `-std=gnu11` で、`asm` キーワードが使えます（インラインアセンブリそのものは未対応）。

pkcl のヘッダは libc なしの `pkcl-nolibc`（`errno`、`fcntl`、`unistd`、`sys/stat`、`sys/types`）と、Newlib のヘッダの足りない分を補う `pkcl-posix` に分かれます。`pkcl-posix` のラッパーは `#include_next` で Newlib のヘッダを呼び、隠されている宣言を足します。

## 10. マイルストーンと Issue

| マイルストーン | 内容 | 状態 |
|---|---|---|
| M0 | 設計、ヘッダ、fd I/O、パス操作、errno、起動コード、ゲストテスト | 完了 |
| M1 | ディレクトリ（`DIR`）、`O_DIRECTORY`、`statvfs`、`chown`、`dup3`、`pread`/`pwrite` | `DIR`・`chown` 縮退は完了。残りは予定 |
| M2 | libc 統合（Newlib の posix プロファイル）、代理 `execve` | 完了。`posix_spawn`・`waitpid` は予定 |
| M3 | 環境変数（G2）、`getpid`（G1）、時刻、`sleep` | 時刻・`sleep` は完了。G1・G2 は予定（`getpid` は 1 を返す仮実装） |
| M4 | pipe サービス（G3）、`poll`、シグナル、termios | termios は完了。他は予定 |
| M5 | BusyBox のビルド統合（`--busybox-source`）、最初の applet、ash／hush の調査、`setjmp`（G7） | ビルド統合と最初の applet は完了。ash／hush と `setjmp` は予定 |
| M6 | 64 bit オフセット（G8）、`utime`（G5）、inode（G4） | 予定 |

各 Issue は GitHub Issues にあります。GitHub のマイルストーンは分野の分類（Foundation OS）で、上の M は Issue タイトルの接頭辞に書きます。

## 11. テスト

`foundation/tests/pkcl/` は実際の Foundation の fs・console サービスに対して、pkcl の POSIX 呼び出しを検証します。

```
python3 -m unittest discover -s foundation/tests/pkcl -v
```

BusyBox のテスト（`test_busybox.py`）は GPL のソースを置かないので、環境変数を渡したときだけ走ります。

```
SRC32_BUSYBOX_SOURCE=/path/to/busybox-1.36.1 SRC32_NEWLIB_SOURCE=/path/to/newlib-4.4.0.20231231 \
    python3 -m unittest discover -s foundation/tests/pkcl -k busybox -v
```

BusyBox のビルドが数分、起動が 1 MiB のイメージの読み込みで遅いので、全体で数分かかります。

- Clang と `llvmlite` の LLVM のメジャー版を合わせてください（`CLANG` で指定）。`src32_testbench` は release ビルドを使います（`SRC32_TESTBENCH`）。
- ゲストは `--entry pkcl_start` で `main(argc, argv)` を持ち、失敗は失敗した行番号で終了します。テストは `pkcl guest passed` の出力と `status=0` を確認します。

## 12. 未決事項

| 論点 | 現在の案 |
|---|---|
| `fork` を要求するソフトウェア | 動かさない。BusyBox は NOMMU 構成で回避（§8.3） |
| pkcl の WAL 化 | 当面は静的リンク。fd 表を WAL の外に置く設計が固まってから検討 |
| スレッドごとの `errno` | 当面グローバル。`thread_create` を使う BusyBox の機能はない |
| group の扱い | FS2 は owner と other だけ。`st_mode` の group は other のコピー |
| `unlink` 中のオープンファイル | FS2 が BUSY を返すかを確認する（POSIX は削除後も開いたままを許す） |
