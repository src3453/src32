# Foundation IPC ベンチマーク

Issue: [#173](https://github.com/src3453/src32/issues/173)

リポジトリのルートから実行します。Clang、Python（LLVM SRC32 driver の依存を含む）、`src32_testbench` が必要です。

```sh
cargo build --release --bin src32_testbench
python foundation/tests/perf/bench_ipc.py target/ipc-bench --mm vm slot
python foundation/tests/perf/bench_ipc.py target/ipc-bench-equal --priority equal
python foundation/tests/perf/bench_ipc.py target/ipc-bench-pipeline --cpu pipeline --repeats 1
```

`--clang` / `CLANG`、`--testbench` / `SRC32_TESTBENCH` で実行ファイルを指定できます。`--rounds` はバッチ内の反復数（既定64、最大512）、`--repeats` は独立した起動回数（既定3）です。出力ディレクトリは条件ごとに分けてください。毎回ビルドし、イメージ、ビルドログ、実行ログ、`results.json` を保存します。`FND_MM` に関係なく `--mm` がバックエンドを選び、ゲストでも実際の選択を検証します。

## 測定対象と単位

- 本番のカーネルを `-O1` でリンクした専用イメージで、独立した U モードのクライアントとサーバーを動かします。IPC や scheduler に計測コードを挿入しません。初期ハンドルの用意だけはカーネル側で行います。
- `time_get(TICK_HZ)` の syscall 対照と、`call → recv → reply → call復帰` の往復を測ります。サーバーの応答後の受信は `reply` + `recv` と `reply_recv` を比較します。要求・応答のデータ量は同じです。
- データは0、32、256、4096 bytes。4096 bytes はページ先頭の整列ケースと、送受信の全バッファを先頭+1 byteにずらしたケースがあります。後者は非整列とページ境界越えの両方を含みます。整列とページ境界それぞれの寄与は、この比較だけでは分離できません。
- 既定ではサーバーが SERVICE、クライアントが APP。`--priority equal` では両方APPです。他のアプリケーション、ディスク、console、WAL は動かしません。コードは builtin 用の共有RX領域、データはそれぞれの私有領域です。
- 各ケースで8回ウォームアップ後、`sys_info(2)` の48 MHzカウンタでバッチを挟みます。`elapsed/kernel/idle` の符号なし32-bit差分を取り、周波数と会計の整合性も検証します。**1 tick = 1/48 µs**で、scheduler の100 Hz tickとは異なります。ホストの実行時間や retired instruction 数ではありません。
- 表示値は**各バッチ平均の中央値**です。`results.json` に各起動の生の差分、平均の最小・最大、往復/秒、kernel/idle を保存します。個々の往復のp95/p99を表す値ではありません。
- 区間には通常のtimer IRQ、ゲストのループ、応答のsequence/長さ確認、終点のsnapshotまでの処理が含まれます。snapshotコストを差し引きません。`time_get` はIPC遅延から引くための補正値ではなく、syscallコストの対照です。128反復でも測り、バッチサイズへの依存を確認できます。
- 各バッチの最終応答のペイロード全体の確認と画面への出力は計測後に行います。要求ごとのsequence、受信長、badge、reply capacityも検証し、故障・不正データ・欠落/重複した結果・ゲスト未完了をエラーにします。性能値そのものに合否閾値は設けません。

CPUモデル、コンパイラ、最適化、反復数、優先度が同じ条件で比較してください。JSONには開始時のGit HEAD、作業ツリーのdirty状態、コンパイラ版、testbenchとWAXのSHA-256を記録します。生成物の同一性はGit HEADだけで判断しないでください。

## 2026-10-25 の測定と改善候補

測定結果と分析は [doc/foundation_ipc_performance.md](../../../doc/foundation_ipc_performance.md)、生データは [baselines/](baselines/) にあります。カーネルの意味を変更せず、まず非整列コピーと短いメッセージの固定費を次の改善対象として切り分けます。
