# コマンド起動の初期調査（Issue #106）

2026-10-24、調査元 `18f5694`。最適化の実装前に、`echo` の起動・終了を段階別に測った。
スロット方式で約 13 Mcycle を再現し、最大の項目は WAX と WAL の読み込みだった。
この計測は調査用プローブを含む。過去の 13 Mcycle と厳密に同じバイナリの比較ではない。

## 再現

PowerShell、リポジトリ直下で実行する。

```powershell
python foundation/tests/perf/profile_spawn.py target/spawn-profile --cpu cached
python foundation/tests/perf/profile_spawn.py target/spawn-profile --reuse --cpu pipeline
python foundation/tests/perf/profile_spawn.py target/spawn-profile --reuse --fast-disk
$env:FND_MM = 'slot'
python foundation/tests/perf/profile_spawn.py target/spawn-profile-slot --cpu cached
python foundation/tests/perf/profile_spawn.py target/spawn-profile-slot --reuse --fast-disk
Remove-Item Env:FND_MM
```

Clang は通常の boot builder と同じ規則で探す。`--clang`、`--testbench` または
`SRC32_TESTBENCH` で指定できる。`--reuse` は同じスクリプトで作った既存の調査用イメージを使う。
ソースを変えた場合は再ビルドする。VM/slot の切り替えも別ビルドが必要。

スクリプトは `kernel/proc.c`、`kernel/blk.c`、WAL の `lib/fnd/proc.c`、`shell/shell.c` を
出力ディレクトリへコピーし、アンカーが一意であることを確認して計測処理を挿入する。
通常のソースは書き換えない。fs、disk、init のコードも変更しない。
ABI 外の診断用 `sys_info` selector 106〜111/120 はこのイメージだけのもの。
通常のカーネル ABI として使用してはならない。

16 MiB RAM、4 MiB VRAM、通常の HDD（10 ms、5 MB/s）、`-O1`、660 frames。
frame 180 に `echo` を 2 回入力し、800 tick で R1=42 に停止する。
表示は子の終了を待った後に行い、2 回の完全なプロファイルと子の正常終了を確認する。
`run-<cpu>/result.txt` に PCG 全文、`samples.json` に数値、`config.yaml` に条件を保存する。
`--fast-disk` は実験用の 0 ms / 1 GB/s 設定で、`run-<cpu>-fast/` に保存する。
DMAC 自体の転送コストは残る。

## 計測範囲

各行の数値は `elapsed kernel idle ticks fs_user` の順。サイクル数は
`sys_info(2)` の u32 差分で、PeC timer ch1 の 48 MHz 時間。
`ticks` はカーネルが処理した 100 Hz tick なので、割り込み禁止中の長い syscall の
経過時間をこれだけで判断しない。fs は通常の起動順で PID 3。

| 行 | 範囲 |
| --- | --- |
| image | `fnd__exec` 入口から WAX の open/stat/alloc/read/close まで |
| deps | deps の確保と `add_deps`（WAL の open/stat/alloc/read/close を含む） |
| spawn | cwd 取得、fs session 作成、startup 構築、`proc_spawn`、補助バッファ解放 |
| free | 親にある WAX/WAL/deps バッファの解放 |
| wait | `fnd__exec` 復帰後から子の終了待ちの復帰まで |

合計はコマンド名検索・字句解析・診断結果表示・PROCESS handle close を含まない。
スケジューラの切り替えによって子の実行・終了の一部が spawn/free 区間に入ることもある。
そのため終了処理のコストは下記のカーネル内プローブでも確認する。

`parts` は、staging へのコピー、`wax_plan`、VM ページ準備、`wax_load[_mapped]`、
終了時 cleanup、`mm_vm`、WAX byte 数、WAL byte 数の順。
終了時 cleanup は `sched_kill_proc` から handle close、`mm_release`、slot zero までで、
後続の `ipc_proc_exited` と record 解放は含まない。これらは区間の内数で、合計へ足さない。
`io` はその起動から終了待ちまでの IDC READ/WRITE 開始回数とセクタ数。
`lbas` は最初の6転送のLBAで、残りは0。転送数が6を超える場合も `io` は総数を数える。

## 結果

以下は同一起動内の 2 回目。単位は Mcycle、丸め前の値は再現スクリプトで取得できる。
release testbench（2026-10-24 のローカルビルド）を使用し、`mm_vm` の実測値で分類した。

| 区間 | cached / slot | cached / VM | pipeline / VM |
| --- | ---: | ---: | ---: |
| image | 2.875 | 3.869 | 4.172 |
| deps | 7.087 | 8.606 | 9.584 |
| spawn | 2.008 | 2.572 | 2.978 |
| free | 0.003 | 0.003 | 0.004 |
| wait | 0.467 | 0.249 | 0.313 |
| 合計 | **12.440** | **15.299** | **17.050** |
| 合計中の idle | 5.468 | 5.466 | 5.435 |

最初の echo は cached / slot が約 12.89 Mcycle、cached / VM が約 15.78 Mcycle。
2 回目でもディスクアクセスは消えない。いずれも毎回 **5 READ、各 128 セクタ、計 320 KiB**。
対象の WAX は 8,556 bytes、WAL は 76,192 bytes（通常ビルド 75,964 bytes にプローブが
228 bytes 増加）。ファイルサイズより大きい転送量には disk サービスの先読みが含まれる。
診断シェルのサイズ増加もファイル配置へ影響し得るため、厳密な無計測時の絶対値とは区別する。

| カーネル内部の内訳 | cached / slot | cached / VM |
| --- | ---: | ---: |
| staging copy | 0.625 | 0.636 |
| wax_plan | 0.531 | 0.531 |
| VM ページ準備 | 約 0 | 0.529 |
| wax_load | 0.719 | 0.731 |
| 終了 cleanup | 0.440 | 0.227 |

同じバイナリでディスク設定だけを 0 ms / 1 GB/s にすると、2 回目の合計は
cached / slot **7.521 Mcycle**、cached / VM **10.393 Mcycle** に下がった。
いずれも I/O は 5 READ / 640 sectors のまま。idle は約 0.64 Mcycle 残る。
これは高速化の成果ではなく、ディスク待ちの寄与を切り分けるための対照実験。

## 解釈と次の調査

1. `lib/fnd/proc.c` の `fnd__exec` は毎回 `read_image` と `add_deps` を実行し、子を起動したら
   読み込んだ WAX/WAL を解放する。WAL の共有・永続キャッシュはこの経路にはない。
   cached / slot では image + deps が全体の約 80%、cached / VM では約 82%。
2. `services/disk/main.c` は 128 セクタの先読み窓を 1 個だけ持つ。
   `src/devices/pec/idc/idc.rs` の deadline はアクセス待ち + 転送時間で、標準 HDD の
   128 セクタ転送は約 1.109 Mcycle、5 回で約 5.546 Mcycle。
   観測した idle と高速ディスク比較は I/O 待ちの大きさを裏付ける。
   どの LBA・メタデータ読み取りが先読み窓を追い出すかは未計測。
3. staging copy、loader、VM 準備も無視できないが、`fnd_mem_free` や終了 zero だけでは
   約 13 Mcycle を説明しない。VM では終了時に frame を返し、次の割り当て時にゼロ化する。
4. 次は各 READ の LBA と呼び出し元、`read_image` 内の open/stat/read/close、IPC の転送回数を
   分ける。WAL 再読込の回避やキャッシュの改善は、その結果と更新時の整合性契約を確認してから判断する。

今回は性能の挙動変更をしていない。fs の 512 KiB スロットを拡張する計測コードも加えていない。
cached の VM/slot、pipeline の VM と高速ディスク対照の計 5 条件で、両 echo と停止を確認した。

## 修正：disk の先読み窓を4つ保持する

READ の開始LBAを追加計測すると、変更前は `4166, 4556, 4166, 5671, 5799` だった。
`4166` は inode 領域を含むメタデータで、コマンド本体を読むと追い出され、WAL の open 時に
同じ領域を読み直していた。さらに WAL 本体が2つの先読み窓に分かれる。

`services/disk/cache.c` に固定4窓の LRU を実装し、各128セクタを保持する。
キャッシュ本体は合計256 KiB、従来からの増加は192 KiB。disk サービスの
512 KiB スロット内に収まり、fs の容量は増やさない。
WAX/WAL は毎回通常の open/stat/read を通るので、パス・権限・ファイル更新の確認を省かない。
WAL のプロセス間共有や libfnd 内の永続キャッシュは追加していない。

書込前に重なる全窓を無効化し、IDENTIFY で該当 drive の全窓を無効化する。
read/write エラー時は全窓を捨てる。先読み失敗後の要求範囲だけの再読込を維持し、
失敗時にバッファへ書かれた部分データをヒットとして返さない。
LRU は有効な窓の順位で管理し、長期稼働で age カウンタが wrap する問題を避ける。

同じ計測プローブ、`-O1`、標準 HDD、同じ testbench での2回目の結果：

| 条件 | 変更前 Mcycle | 変更後 Mcycle | 短縮率 | READ回数 |
| --- | ---: | ---: | ---: | ---: |
| cached / VM | 15.290 | 7.269 | 52.5% | 5 → 0 |
| pipeline / VM | 17.040 | 8.629 | 49.4% | 5 → 0 |
| cached / slot | 12.440 | 6.863 | 44.8% | 5 → 0 |

VM の変更前は `f27ac63` の disk 実装へ LBA プローブを追加した `target/spawn-before`、
変更後は `target/spawn-after`。slot の変更前は初期調査の値（LBA プローブ追加前）、
変更後は `target/spawn-after-slot`。いずれもゲスト時間でありホスト時間の測定ではない。
変更後 VM の image/deps はそれぞれ約0.643/3.801 Mcycle、idle は0。
残りは IPC 転送・fs 処理・カーネルのコピー/ローダ等で、起動コスト全体がなくなるわけではない。
多種類のファイルを巡回して4窓を超える場合や、起動直後のコールドアクセスまで0 READになるとは限らない。

disk.wax は124,966→326,818 bytes（このビルド）。ロード範囲は `0x500000` の59,488 bytes と
`0x50F000` の267,234 bytes、stack は `0x57C000..0x580000` で、両者は重ならない。
VM では静的バッファ増加分の物理ページも消費する。

検証には `foundation/tests/disk_cache` のホストテストを追加した。
実際のキャッシュに対して、観測したワーキングセット、LRU、窓の重なり、別 drive、媒体差し替え、
先読み失敗後の exact retry、部分読込エラー、容量末尾・不正件数を確認する。
シェルには既存の実行ファイルを上書きして次回起動で新しい内容が動くテストも加えた。

最終検証（2026-10-24）:

- `python -m unittest discover -s foundation/tests/disk_cache -v`: 1 test / 7シナリオ通過。
- `python -m unittest discover -s foundation/tests/shell -v`: VMで24件通過。
- 同じシェルテストを `FND_MM=nommu` で実行: 24件通過。
- `python -m unittest discover -s foundation/tests/disk -v`: 7件通過（BIOS→stage1→kernel→shellを含む）。
- 計測用イメージの cached/VM、pipeline/VM、cached/slot で両echoの正常終了とR1=42停止を確認。
- `git diff --check` と変更したPythonの `py_compile` 通過。

ホスト検証の `CC` は `C:/Program Files/AMD/ROCm/6.4/bin/clang.exe`、
`TEMP` / `TMP` はワークスペース内の書込可能なディレクトリを指定した。
