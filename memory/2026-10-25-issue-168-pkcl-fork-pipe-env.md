# pkcl: fork・waitpid・pipe・環境変数・setjmp（Issue #168、#169 の一部）

`proc_fork`（VM のみ）と COW がマージされたあとに、BusyBox の MMU 構成（ash／hush）に要る POSIX 面を足した作業の記録。

## 足したもの（契約）

- `sys_info` sel 4 = 呼んだプロセスの pid と parent（8 bytes）。sel 5 = PROCESS ハンドルの pid（ハンドル番号を `buf` に渡す）。`fork` の戻りは PROCESS ハンドルなので、pid は sel 5 で引く。
- pkcl `posix/process.c`: `fork`/`vfork`/`waitpid`/`wait`/`kill`/`getpid`/`getppid`。子プロセス表は 32 件で、`fork` の子では空にして始める（子は孫しか待てない）。`waitpid(WNOHANG)` は `wait(h, 0)` の `FND_E_WOULD_BLOCK` を 0 に直す。
- `setjmp`/`longjmp` は `tools/llvm-src32/backend.py` がインライン展開する（このバックエンドは inline asm も weak も alias もないので、C やアセンブリ関数では書けない）。`sigsetjmp` は `pkcl-posix/setjmp.h` で `setjmp` に倒す。
- pipe サービス（`foundation/services/pipe`、名前 `pipe`、API §9.5）。端は badge 付き CHANNEL で、`fork`／`dup` で共有できる。EOF と EPIPE は `CLIENT_GONE`（端の最後のハンドルが閉じた）で決まる。どのバックエンドでも起動する（`fork` が VM 専用なので、端の共有は VM だけ）。
- 環境変数: 起動ブロック version 2（`env_count`、`env_off`）、`fnd_envc()`／`fnd_env(i)`（WAL fnd 1.3）、`fnd_exec_t.envp`。`pkcl_start` が `environ` を作り、`execve` が `envp` を渡す。
- プロセス上限を 7/8 から 10/16（`FND_MAX_PROCS`／`FND_MAX_THREADS`）に増やした。pipe サービスが 1 つ使い、`fork` の子に余地が要るため。 12 にすると `tests/wax` の PIE 配置の期待（42/43）が崩れるので、10 までにしてある。

## 落とし穴

- `ST_READ` を 4096 bytes より大きく要求すると libfnd の `fnd_read` は満杯の塊の次を待つ。pipe の `read` は pkcl が 4096 bytes に切る（`PKCL_FD_PIPE`）。切らないとシェルのパイプが固まる。
- `FND_E_PEER_GONE` は `EPIPE` に対応づけた（それまでは `EIO`）。SIGPIPE は配送されない。
- 起動ブロックは 4096 bytes まで。環境と引数の合計がこれを超えると `execve` は `E2BIG` 相当（`FND_E_LIMIT`）で失敗する。
- `execve` は代理 exec（子を作って待ち、終了コードで自分も終わる）のまま。fork 後の exec は動くが、イメージを毎回読み直す。
- `-k posix` のように名前で絞ると `test_posix_io`（nolibc 側）も一緒に走る。nolibc の `sys/stat.h` に `S_IFIFO` がないと両方が壊れる。

- console は、読み手が `kill` された直後（`CLIENT_GONE` が届く前）に渡したキーを失っていた。`ipc_reply` が失敗したらキーをキューの先頭へ戻す（`unget_key`）。`test_ctrl_c_interrupts_a_blocked_read` が起動ブロックの大きさ（8 の倍数かどうか）で落ちる形で見つかった。
- 起動ブロックの大きさは 8 byte 境界に切り上げる（`lib/fnd/proc.c`）。上の失敗の引き金だったタイミング差を減らすためで、原因そのものは console 側。
- 新しいサービスを足すと `ps`／`top`、FNDBOOT のモジュール表を数えるテスト（`test_shell.py`、`test_disk_boot.py`）の期待値が変わる。pipe は全バックエンドで起動するので行数は同じ。

## 検証

- `SRC32_NEWLIB_SOURCE=... python3 -m unittest discover -s foundation/tests/pkcl -k posix`（`guest_posix.c`: fork の COW、waitpid、execve と環境、6 KB の pipe、EOF、EPIPE、setjmp）。
- `python3 -m unittest discover -s foundation/tests/shell`（VM と `FND_MM=nommu` の両方）、`foundation/tests/wax`、`tools/llvm-src32` の `test_toolchain -k foundation`。

## 残り（#169 ほか）

`poll`／`O_NONBLOCK`（pipe と通常ストリーム）、シグナルの配送（SIGINT、SIGCHLD）、`posix_spawn`、`proc_exec`（in-place exec）、BusyBox の MMU 構成（ash／hush）での不足シンボルの洗い出しと 1 MiB 制限。

## BusyBox の MMU 構成と ash（同じ作業の続き）

- `busybox.py --config busybox.config --config busybox_ash.config`（複数指定は後勝ち）。ash 入りは 1.39 MB なので、`FND_EXEC_IMAGE_MAX` と `FND_SPAWN_IMAGE_MAX` を 1.5 MiB にした（kernel の staging は 3 MiB で、WAL と合わせて収まる）。
- ash が要求した不足は `times`、`WCOREDUMP`、`fcntl(F_DUPFD_CLOEXEC)`（スクリプトを開くと `sh: 3: m` のような壊れたメッセージで落ちる）だけだった。
- applet の自己 exec は `BUSYBOX_EXEC_PATH`。既定の `/proc/self/exe` は Foundation にないので `/bin/busybox` にし、WAX を拡張子なしでも置く。`fnd_proc_exec` は argv[0] をパスで上書きするため、applet 名が消えて `a.sh: applet not found` になる。`FND_EXEC_KEEP_ARGV0` で防ぐ。
- 設定ファイルを更新しても `--stage link` は `.config` を作り直さない。出力ディレクトリを変えるか消す。
- 確認済み: `busybox sh /a.sh`（`|`、`()`、`export`、`<` `>`、`||`）、対話の `busybox sh`。未確認: ジョブ制御、`read`、`trap`（シグナルがない）、`wait`、`kill`。
