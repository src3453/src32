# Foundation の sol API

`fnd.sol` はカーネルの syscall、`libfnd.sol` は C libfnd の高水準操作を提供します。
メインで `!abi_compat "Foundation" 1` を宣言し、`libfnd.sol` を include します。
最初に `fnd_init` を一度呼び、起動ブロックの argv と役割ハンドルを読み込みます。
sol VM 上では実行できません。

```sh
# C libfnd を静的リンクした PROTECTED WAX v2
python tools/solc/build_foundation.py main.sol -o app.wax
# 既存の fnd WAL を使う WAX v3（Foundation boot build が生成するもの）
python tools/solc/build_foundation.py main.sol -o app.wax --import-wal target/boot/root/lib/fnd.1.wal
```

`--clang PATH` と `--org 0x800000`（slot 5、既定）／`--org 0xc00000`（slot 6）を指定できます。
ビルダーは C dispatcher の実際のアドレスを `FND_LIB_ENTRY` として定義します。
通常の solc 単独ではこのリンク処理を行いません。生成する `.sol.a`、`.libfnd.a`、
`.libfnd.ll` はデバッグ用です。ゲストにインストールするのは `.wax` だけです。

sol のコード・文字列・変数を slot の先頭から1 MiB以内に置き、C bridge をその後に配置します。
スタックは slot 末尾（既定256 KiB）です。重複と slot 超過は拒否します。
WAL 版では既存の WAX v3 loader が fnd の記述子と R30 を設定します。
ディレクトリAPIを加えた WAL は `fnd` **1.1**（パスは major を使う `/lib/fnd.1.wal`）です。
1.0 の利用者とは互換ですが、新しいラッパーを使うゲストは1.1以上を必要とします。
静的版も同じ C 実装を使い、ABI は同じです。R26/R27、R30 を橋渡しで保存し、
C 引数を8-byte境界のスタックへ左から順にコピーし、R1 を sol の戻り値にします。

## 値と構造体

- 引数は C の宣言と同順。値は32-bitのビット列で、文字列はNUL終端UTF-8です。
- `fnd_status` は **0成功／負エラー**。sol の比較は **0真／1偽**。
  `status 0 lt if ... end` でエラーを判定します。`FND_END=1` は成功0とは別です。
- ハンドル、ポインタ、argc は status ではありません。0ポインタは無し／alloc失敗。
  ハンドルを返す syscall と、out 引数でハンドルを返す `fnd_file_open` を区別します。
- 出力引数は4-byte境界の書き込み可能なアドレスを渡します。たとえば
  `4 fnd_mem_alloc` の戻り値を `out` とし、`path flags mode out fnd_file_open` の成功後に
  `out ld` でハンドルを取り出します。必要なバッファは `fnd_mem_free` で返します。
- 失敗時は C の契約をそのまま引き継ぎます。通常は出力値を変更せず、例外は
  `fnd_write` の部分書き込み byte 数と、`fnd_cwd_get` の容量不足時の必要長です。
- `fnd_stat_t` は64 bytes。`FND_STAT_*` が各オフセットです。64-bit値は
  **上位u32、下位u32** の順、時刻は符号付き64-bit UNIX秒と別のu32ミリ秒（0〜999）。
  秒やサイズを単一の sol 値に切り詰めません。`seek`／`truncate` と64-bit引数の
  高水準ラッパーは C libfnd 側もまだ提供していません。
- `fnd_exec_t` は28 bytes。argc、argv（ポインタ配列）、cwd、stdin、stdout、stderr、flags の順。
  **ex 自体は有効なポインタが必要**。cwd=0 は現在のcwd、各stream=0 は自分のものを渡します。
  flags は0。`FND_EXEC_*` を使い、予約を含め全フィールドを初期化してください。
- `fnd_key_t` は12 bytes。code、Unicode scalar、mods の順で、それぞれu32です。
- `fnd_dirent_t` は1100 bytes。stat が0、name_bytes が64、name_chars が68、
  NUL終端UTF-8名が72。長さはともにu32で、wire record のu16とは異なります。
  `FND_DIRENT_SIZE`／`FND_DIRENT_NAME` 等を使います。名前長はNULを含みません。
- `fnd_read`／`fnd_write` は C 実装が4096 bytes単位に分割します。read の got=0 が EOF。
- open で得たハンドルは `fnd_close`。`fnd_proc_wait` は成功時に PROCESS を閉じます。
  stdin/stdout 等の借用した役割ハンドルを通常の処理終了時に閉じる必要はありません。

## 対応範囲

起動、argv、役割、stream open/read/write/stat/sync/close、path stat/delete、cwd、console、
exec/wait/kill、alloc/free、strerror は現在の C libfnd を呼びます。
`fnd_proc_kill` は同じ契約の既存 syscall ラッパーを共用します。
仕様だけに存在する C API（名前照合など）を成功する関数としては公開しません。

`fnd_dir_make`／`fnd_dir_remove` と `fnd_dir_open`／`fnd_dir_next`／`fnd_dir_rewind` も
C libfnd を使います。`next` は4096-byte応答をキャッシュし、1件ずつ返します。
`FND_END` またはエラー時には出力を変えません。`rewind` はサービスの位置とキャッシュを戻し、
`fnd_close` はキャッシュも解放します。列挙ハンドルを閉じるときは生の `fnd_handle_close` を使わないでください。

低水準の
`fnd_dir_read(h, buf, cap, got)` は [API §10.2](../../doc/spec_Foundation_API.md) の
wire records を返します。cap は最大4096、1096以上なら最大エントリも収まります。
終了は got=0。名前はNUL終端ではなく、各 record の rec_len と name_bytes で読みます。
同じハンドルでキャッシュ付き `next` と混用するときは、切り替える前に `rewind` してください。

[実行可能なサンプル](../../examples/foundation-sol/README.md) はディスク上のFS2へ5001 bytesを
書いて再読込し、複数応答に分かれる65件の列挙、途中のrewind、END、close時のキャッシュ再利用を確認します。

`fnd_proc_fork`（SYSCALL 0x14）は、親で子の PROCESS ハンドル、子で 0 を返します。VM と生存スレッド 1 本が必要で、MMIO 付与や THREAD ハンドルは複製できません。RW ページは COW で分離し、ハンドルが指すオブジェクトは共有します（[カーネル仕様 §9.6](../../doc/spec_Foundation.md)）。ハンドルは sol の真偽値とは異なるので、負のエラー・0 の子・正の親を分けて判定してください。
