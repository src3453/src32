# KAN-36 / KAN-37: Foundation 起動とシェルテストの計測

以下のKAN-36/KAN-37計測は完全マウント検査を行う時点の記録。
後続のKAN-38による暫定的な全走査省略は末尾に記す。

2026-10-22、Windows、AMD Clang 20、Python 3.15、llvmlite 0.50、release
`src32_testbench.exe` で計測。変更前は `f2a6b96`（KAN-15 と sol/libfnd を統合済み）。
ゲストは `foundation/boot/build.py -O1 --stop-ticks 6000` の生成物、RAM 16 MiB、
VRAM 4 MiB、8 MiB FS2 HDD を IDC slot 0 に接続した。BIOS は設定せず
`foundation.wax` を直接ロードした。BIOS POST を含む起動時間ではない。

## 変更と効果

- 同梱 string ランタイムは従来、ゲストの最適化レベルにかかわらず `-O0` だった。
  これを `-O2` にし、`memcpy` / `memset` に整列した 32-bit アクセスを加えた。
  先頭と末尾の端数、異なる整列のコピーは byte 単位。`may_alias` 型で任意の C オブジェクトを扱う。
  ユーザー実装を優先するリンク方針と外部 Newlib のビルド方針は維持する。
- シェルの機能テストを Cached に変更。再起動後の `ps` と CPU 統計の snapshot は
  Pipeline に残す。`FOUNDATION_TEST_CPU=pipeline` で全件を Pipeline に戻せる。
  600 frame / stop tick 6000 の上限、全画面比較、書き込み後のディスク検査、
  ケースごとの新しいディスクコピーは維持する。
- FS2 §10 の検査は省略も遅延もしない。

| 計測 | 変更前 | 変更後 |
|---|---:|---:|
| 同一 Pipeline で最初のプロンプトが揃う frame 数 | 101 | 99 |
| 8 KiB の memset と memcpy を 4 回、Pipeline のゲストサイクル | 6,096,030 | 976,474 |

コピー計測は main を `-O0` に固定し、PeC timer ch1 を 48 MHz の free-running
カウンタとして処理の前後で読み取った。計 64 KiB 分の書き込み／コピーを行い、
コピー後の全 byte が 0xA5 であることも確認した。約 6.2 倍の改善はこの処理の値で、
OS 起動全体の改善率ではない。testbench の最後の `cycles=` は指定 frame 分まで
進むため、処理コストの指標には使っていない。

シェル全 16 件の実測は、ビルド込みで 196.973 秒（変更前、全件 Pipeline）から
157.568 秒（変更後、Cached と代表 2 ケースの Pipeline）になった。約 20% の短縮。
各 1 回の実行であり、変更前の終盤にはランタイムの検証や生成物のビルドも並走したため、
厳密なホスト性能ベンチマークではない。生成物を再利用した変更後の全件検証も
Cached 118.728 秒、Pipeline 174.759 秒で成功したが、こちらはビルド時間を含まず、
一部ほかの検証と並走しているためビルド込みの数値と直接比較しない。

変更後の検証結果：シェルは最終構成・全件 Cached・全件 Pipeline の各 16 件が成功。
同じ生成物を使った BIOS 経由のディスク起動全 7 件も成功。
toolchain は string 境界、複数 C 翻訳単位、Foundation WAX ローダ、
fastcc/byval/64-bit 戻り値、IR/ASM 再アセンブルの focused 5 件が成功した。
string の境界・端数・ゼロ長・戻り値・memmove 重なりは O0/O1/O2 × Scalar/Pipeline で検証した。

初回プロンプトの frame 数は、入力なし・read-only HDD・Pipeline で実行し、
画面にバナーと `/>` が揃う最小の `--frames` を二分探索した。
全チェーン検査とディスク転送が残るため、ランタイム変更だけでは起動は大きく短縮しない。

## KAN-37 の状態表示

`shell/shell.c::run_program` は `fnd_proc_exec` が戻った後で `fnd_sys_wait` を呼ぶ。
`lib/fnd/proc.c::fnd__exec` は spawn 後も、依存 WAL と WAX のイメージおよび
依存リストを解放している。この間に親の time slice が切れると、子の `ps` は
親が wait でブロックする前に走れる。
`kernel/sched.c::sched_proc_runnable` は READY と RUNNING のどちらも RUN として
`sys_info` に反映するので、親の RUN は正当な snapshot である。

KAN-15 で既に入った RUN/BLOCK の許容を維持し、その根拠をテストに追記した。
テストは古い shell PID 5 の不在、新 shell PID 6 の親 PID 4、子 ps PID 7 の
親 PID 6、二度目のバナーとプロンプト復帰、一覧の件数を引き続き比較する。
スケジュールを変えて BLOCK に固定する修正は不要。

## 再検証

```sh
python -m unittest discover -s foundation/tests/shell -v
FOUNDATION_TEST_CPU=pipeline python -m unittest discover -s foundation/tests/shell -v
python -m unittest discover -s tools/llvm-src32 -k string_runtime_boundaries -v
```

Windows は TEMP/TMP を workspace 内の書き込み可能なディレクトリに設定する。
PowerShell の CPU 指定は `$env:FOUNDATION_TEST_CPU='pipeline'`。
比較時は同じ testbench バイナリとゲスト最適化レベルを使い、ビルド込みと
ビルド済み生成物の実行時間を区別する。ほかの計測プロセスとの競合も記録する。

## KAN-38: マウント全走査の一時省略

2026-10-22、`28f8d08` の完全検査イメージと、同じ `-O1 --stop-ticks 6000` の
暫定対策後のイメージを比較した。同じrelease testbench、Pipeline、RAM 16 MiB、
VRAM 4 MiB、read-onlyの8 MiB HDD、BIOSなしの直接WAX起動で、入力せず
バナーと `/>` が揃う最小frame数を二分探索した。
初回プロンプトは **99 → 61 frame**（約38%短縮、60 Hzで約1.65 → 1.02秒）。
これはゲストのframe数であり、ホストの実行秒数やBIOS経由の起動時間ではない。

`foundation/boot/build.py` はfsサービスに `FS2_FULL_MOUNT_CHECK=0` を渡す。
署名・レイアウト・DIRTY拒否とbitmap/inodeの基本検査、割当管理の初期化は残す。
全チェーン・ディレクトリ・参照整合性検査を省き、走査用の一時bitmapも割り当てない。
clean媒体の破損を見逃す暫定措置であり、事前にホストツールの `check` を行う。
定数を1に戻して再ビルドすると完全検査に戻る。詳細は `spec_FS2.md` §10.1。

検証はホストfs全27件（完全検査と暫定モード）、シェル全16件（Cachedと代表Pipeline）、
BIOS/ディスク起動全7件が成功した。シェル・ディスク検証は再ビルドした生成物を再利用し、
各ケースのディスクは独立コピー。シェル116.945秒、ディスク56.502秒は他の検証と
並走した各1回の値で、ビルド時間を含まず、上記の過去の時間との性能比較には使わない。

## KAN-41: PR #13 以降の API 性能（fs とカーネル）

2026-10-22、Linux、clang 18、release `src32_testbench`、Pipeline、`-O1`、直接 WAX 起動、
RAM 16 MiB、read-only でない 8 MiB HDD。コマンド側から `fnd_*` を繰り返し呼び、
`sys_info` の CPU 統計（カーネル／各プロセス）と tick（1 ms）で測った。

PR #13 前後（`c58cebf` と現行カーネル）で、syscall 単体（yield、time_get、sys_info、
debug_write の各 4 万回）と 2 プロセス間の文脈切り替えを同条件で測ると差は 0.3% 以内だった。
syscall 経路そのものは劣化しておらず、PR #13 以降に重くなった原因は
IPC と fs サービスを経由する API の層にある。測定値（変更前）：

| 操作 | 変更前 |
|---|---:|
| libfnd（WAL）呼び出し | 約 80 cycle |
| console への IPC 往復（stream_stat） | 約 3 万 cycle（うちカーネル約 2.2 万） |
| `fnd_path_stat("/bin/echo.wax")`（キャッシュが温まった後） | 約 24 万 cycle（fs サービス約 18 万、カーネル約 4 万） |

fs サービスの PC サンプリング（1 kHz）で、path_stat の時間の過半が次にあった。

- `fnd_fold_next`（約 53%）: ASCII の文字でも大文字小文字畳み込みの表を二分探索していた。
- ディレクトリ走査: エントリごとに inode の復号と 64 bit 除算（ソフトウェア実装）をやり直し、
  head と名前を別の `fs2_read` で読んでいた。1040 byte のエントリを線形に比べる。

変更:

- `fnd_casefold` / `fnd_fold_next` に ASCII の速い経路（A–Z だけが畳まれる）。
- `fs2_read` の本体を `read_with`（復号済み inode を受け取る）に分け、32 bit で割れるときは 32 bit 除算。
  ディレクトリのエントリは head と名前の先頭 32 byte を一度に読む。
- ディレクトリごとの索引（畳み込み鍵のハッシュとスロット）。検索は一致候補の 1 エントリだけを読む。
  構築は read-ahead バッファだけを使い、24 line のデータキャッシュを押し出さない。
  変更系（`begin_change`）で全索引を捨てる。索引は 4 つ（`/`、`/bin`、`/lib` と 1 つ）で、
  2 つだと `/bin` と `/lib` を交互に引くたびに作り直して遅くなった。
- `proc_activate` は 8 領域を先に全部無効化せず、新しい領域を書いてから余分な分だけ無効化する
  （領域のチェックは U モードだけ。spec_CPU_priv.md）。文脈切り替えの MTSR 書き込みが減る。

結果（同一条件、変更前 → 後）:

| 操作 | 変更前 | 変更後 |
|---|---:|---:|
| `fnd_path_stat` の fs サービス CPU（100 回） | 18.1 Mcycle | 6.8 Mcycle |
| 同、100 回の経過時間（コールド開始） | 492 ms | 332 ms |
| console 往復あたりのカーネル | 約 2.2 万 cycle | 約 2.0 万 cycle |
| 20 回の `open`+`close`（/bin/shell.wax） | 295–412 ms | 121 ms |

シェルでコマンドを 4 frame ごとに 20 個打つテストでは、変更前後でほぼ同じ速さ（200 frame で line9、
250 frame で line12 と line13）だった。コマンド 1 回は約 13 Mcycle で、その大半は fs の検索ではなく
プロセスの起動（読み込み、WAL、spawn）にある。こちらは未調査で、次の候補。

### 起動イメージの余裕

`foundation.wax` は FNDBOOT をカーネルイメージにリンクしていた。モジュール合計が約 970 KB で、
§5.1 のカーネルイメージ範囲（`0x10000–0xEFFFF`、約 900 KB）とスタック（`0xF0000–0xFFFFF`）を
すでに越えており、fs.wax が数 KB 増えると init の起動が `bad format` や `limit exceeded` で
失敗した。FNDBOOT をディスク起動と同じ `0x00200000` の block として足す形に変えた。
fs サービスのスロット（512 KiB）も余裕が少ない（コード約 330 KB、スタック 32 KB、
ヒープは /tmp の 64 KiB と 2 つのボリュームでほぼ使い切る）。索引を 6 つにすると起動に失敗した。
