# nommu のスロット方式をアリーナ方式と再配置可能 WAX に置き換えた（#175）

V のない CPU（`FND_MM=nommu` / `-D FND_MM_NOMMU_ONLY`）の `mm` バックエンドは、固定スロットをやめて nommu Linux 風になった。仕様は `doc/spec_Foundation.md` §5.2、画像形式は `doc/spec_WAX_v3.md` §3.1。V のある CPU のページ表バックエンドは変えていない。

## 契約

- ページプールは `[0x400000, RAM 末尾)`、4 KiB、最大 256 MiB のビットマップ。空きページは常にゼロ（プロセス終了時にゼロ化して返す）。
- プロセスは連続したアリーナ 1 つ。長さは「使用分 + 予備（`heap_hint`、無ければ 128 KiB）」。
- 固定アドレスの画像は `[min(base, stack 下端), max(終端, stack 上端))` を丸めてちょうどそこを要求する。取れなければ `FND_E_ADDRESS_CONFLICT`。
- 再配置可能な画像は、試しの基底 `0x01000000` で一度だけ計画してから確保し、`wax_rebase` で本番の基底に動かす（計画を 2 回すると spawn が遅く、nommu のシェルテストが時間切れになった）。
- `mem_grow` はアリーナを動かせない。ヒープがアリーナの末尾にあり、上のページが空いているときだけ伸びる。大きなヒープが要るプログラムは `--heap-hint` で最初に求める。
- 再配置可能な WAX のスタックは最下位ブロックの直下（`stack_top == base_addr`）。初期 R30 は配置後の origin。
- 全 Foundation プログラム（init・各サービス・シェル・`/bin`）は `--wax3 --pie`。`FND_MAX_PROCS`（7）とスレッド 8 はスロットに縛られなくなったが、まだ増やしていない。

## ツールチェーン（`--pie`）

- `driver.py` は PIE_BASE（`0x100000`）と、`0x01000000` ずらした 2 回目のリンクを作り、ブロックの差分から relocation を作る（`v3.relocations`）。データ語は差がちょうど probe、コードは `LDIH` の直後に同じ rd の `LDIL`。説明のつかない差はエラー（アドレスを足す・切り捨てるコードは `--pie` にできない）。
- relocation entry は上位 30 bit がリンク時アドレス、下位 2 bit が種類（0 データ語、1 LDIH/LDIL 組）。

## 落とし穴

- llvm-src32 の生成コードは遅い（約 25 命令/操作）。relocation と spawn のループは軽く保つ。`@llvm.fshl` は未対応なので、ビット操作に回転を使わない。
- ホストテスト（`foundation/tests/wax/host.c`）の PIE_BASE が、ホストの RAM バッファより下にあるとメモリが壊れる（`0x110000` にした）。
- `tests/wax`・`tests/wal` は物理メモリを直接見るので `FND_MM_NOMMU_ONLY` でビルドする。`tests/shell` などは両モードで通る。free の値は両モードとも N に正規化して比べる。
- 変更後に確認したもの: `python3 -m unittest discover -s foundation/tests/wax`、`foundation/tests/shell`（V と `FND_MM=nommu` の両方）、disk・fb・sgc・libsgc・tinybasic・wav・sgub・bmpview・chip8・console・pec・panic・fs・lzma（nommu）、`tools/llvm-src32/test_toolchain.py` の phase1／vm／wal。
