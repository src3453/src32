# CNET の 2 インスタンス・チャットデモ

- `examples/cnet-chat/run_demo.py` が Foundation・CNET カード・`chat.c` をビルドし、`target/cnet-demo/{A,B}` にディスクのコピーと config を書いて、エミュレータを 2 つ起動する。2 台は localhost の空きポートを `udp.peers` で互いに指定する（マルチキャストにしないのは、環境によって届かないことがあるため）。
- `--headless` は testbench を 2 つ同時に走らせ、`--type` で会話を入力して両方の PCG 画面を照合する。`foundation/tests/cnet/test_demo.py` がこれを実行し、CI の shell ジョブ（`discover -s foundation/tests/cnet`）にも含まれる。testbench は `--allow-running` で動かし、Foundation は `--stop-ticks` なしでビルドする。
- 2 台の仮想時間は同期していない。受信は 100 Hz で再確認しているので、会話の入力は数十フレーム間隔を空ける（SCRIPT の frame）。
- WAX の `--image-id` に `-` は使えない（`[a-z][a-z0-9_]*` を `.` で区切った形）。
- `chat` は RAW モードで `fnd_con_pollkey` と `cnet_recvfrom(NONBLOCK)` を交互に調べ、何もなければ 1 tick 眠る。終了時は LINE モードに戻す。
- GUI 版（cpt32 を 2 つ起動する経路）は、実際にウィンドウを開いての確認はしていない。config は `foundation/run.py` と同じ cpu・pcg の設定にしてある。
