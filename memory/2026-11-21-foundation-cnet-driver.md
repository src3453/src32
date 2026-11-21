# Foundation の CNET ドライバと libcnet

- `/dev/cnet` はカーネルドライバ `foundation/kernel/cnet.c`（チャネル種別 13/14、`KDEV_CNET`）。ユーザー空間のサービスにしなかったのは、拡張カードの MMIO がユーザープロセスに付与できない（spec_Foundation §10.3）ため。この規則はユーザーと相談して変えないことにした。
- カードは最初の `DEV_OPEN` で拡張スロットを調べて見つける（空きスロットの読み出しは 0 で bus error にならない）。S モードは VM でも identity なので、カーネルは `0xA0000000` 以降を直接触れる。
- ストリームの badge に「open の権利（bit0-1）＋カードのハンドル番号+1（bit8-12）」を入れる。`cnet_call` は bind で badge を書き換えるので、`&chan->badge` を渡す（他のカーネルドライバは値渡し）。
- 受信待ちは PEC と同じ `B_PEC_READ` で待たせ、`ipc_pec_poll`（100 Hz）で再試行する。cnet は内部の戻り値 `CNET_PARK` で「待たせる」を表す。`FND_E_WOULD_BLOCK` は NONBLOCK のときにそのまま利用者へ返すので、PEC のように WOULD_BLOCK で待たせる判定に混ぜないこと（ipc.c の PEC 判定は `KDEV_TRNG..KDEV_UART0` に限定した）。
- カーネルのソース一覧は build.py だけでなく `foundation/CMakeLists.txt`、`foundation/tests/{code,crypt,lzma,rle,panic}`、`tools/llvm-src32/test_toolchain.py` にもある。ドライバを足したら全部に足す。`foundation/tests/pec/host.c` は ipc.c を include するので、新しいドライバ関数のスタブも必要。
- libcnet は WAL「cnet」1.0（`foundation/lib/cnet`、libsgc と同じ作り）。`bin/cnet.c` は libfnd の WAL が export していない関数（`fnd_name_match`）を使えない。リンク時に undefined external になる。
- テスト `foundation/tests/cnet` は、testbench の UDP セグメントの向こうに Python の独立した CNET ノードを置いて、ワイヤ形式を相互に確かめる。カードはテスト内で cargo からビルドする（wasm32 ターゲットが必要）。
- 検証（2026-11-21）: `python -m unittest discover -s foundation/tests/cnet -v`（VM と `FND_MM=nommu`）。ほかに shell（31 件）、panic、rle、sgc、pec、`test_toolchain.ToolchainTests.test_foundation_vm_kernel` が通ることを確認した。
- 未対応: ドライバのホスト単体テスト（`CNET_READ8` は差し替えられるようにしてある）、IRQ による受信通知（今は 100 Hz のポーリング）、複数カード、Tang 向け（`dev.cnet` は UNSUPPORTED）。
