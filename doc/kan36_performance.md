# KAN-36 / KAN-37: Foundation 起動とシェルテストの計測

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
