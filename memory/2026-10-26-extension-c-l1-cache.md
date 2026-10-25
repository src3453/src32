# Extension C: 分離 L1 キャッシュ（エミュレータ + FPGA）

- 仕様は `doc/spec_CPU_cache.md`。CPUID bit10（`0x400`）、パイプライン CPU のみ `0x7BF`。scalar/cached/JIT は `0x33F` のまま（C を実装しない）。ISA 変更なし、ソフトの保守操作不要（ハード同期）。
- 構成: I 8 KiB + D 8 KiB、ダイレクトマップ、32 B ライン、物理アドレスでインデックスとタグ、ライトスルー・ノーライトアロケート。ストアは一致する I ラインも無効化する。キャッシュ対象は Main RAM（FPGA では SDRAM 窓と RAM ディスク窓）のみ。MMIO と ROM はバイパス。
- レジスタ `0xFFFF0300..033F`（CACHE_INFO / CTRL / I,D の hit,miss カウンタ）。イネーブル変更で該当キャッシュを無効化する。ユーザーモードからは見えない（`0xFFFF0000` 以上は不可）。
- エミュレータ（`src/cpu_cache.rs`）はタグのみのモデルで、データは常にバスから読むので古いデータは返せない。既定ではミスペナルティ 0（既存のサイクル期待値を壊さない）。`set_cache_miss_penalty(n)` で停止サイクルを足せる。DMA 書き込みの snoop はしない（カウンタがずれるだけ）。パイプラインは HALT の先まで投機フェッチするのでカウンタに入る。
- RTL（`src32-sv/rtl/src32_l1.sv`）: コアのバスとSoCバスの間に挿入。BSRAM 推論（`syn_ramstyle="block_ram"`）で SSRAM は 0（リビジョン B にない）、データは D をバイトレーン 4 本に分けてバイト書き込みに対応。ヒットはリクエストの次サイクルに ack。ミスは `sdram_ctrl` の `burst` 入力で 8 ワードを 1 回の ACT + 16 READ で取得。フィル後 1 サイクル待ってから再ルックアップ（BSRAM の読み書き同時衝突を避ける）。
- 落とし穴: レジスタ窓のデコードは `c_addr[31:6]==26'h3FFFC0C`。最初 `3FFFFC` と書き間違え、TB も同じ誤りで通っていた（実機/SoC シミュレーションでバスエラーになって発覚）。アドレス定数は TB と DUT で独立に確認する。TB の uncached 操作は cacheable なアドレスと別窓にする（同じ物理アドレスをキャッシュ可/不可で混ぜると当然非コヒーレント）。
- `tb_soc` は CPU が高速になり TX FIFO を残したまま HALT するので、HALT 後の待ちを伸ばした。
- 検証: `scripts/run_l1_sim.py`（ランダム、変異で snoop/ストア更新を壊すと失敗することを確認）、`run_soc_sim.py --phase 337.5`、`run_timer_sim.py`、cargo `cpu_cache` 単体 5 件と tests/cpu_pipeline 3 件追加。実機: 1M 反復ループ 1.258 s → 0.510 s（12.1 → 4.9 clk/命令）、Foundation 起動 28.7 s → 13.2 s、UART シェルテスト PASS。
- 配置配線は `--place-option 3` で 48.116 MHz（既定では 48 MHz 未達になることがある）。15,038 LUT+ALU、BSRAM 10。
- 以降の律速はコアのアクセスエンジン（NEXT→CHK→BUS 要求→BUS ack で 1 アクセス約 4 クロック）。メモリではなくコアの構造なので、これ以上はコア側の改造が必要。
