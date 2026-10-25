# Foundation COW と proc_fork（#176）

- `proc_fork` は SYSCALL `0x14`。親には子の PROCESS ハンドル、子には 0 を返す。子は保存済み EPC（SYSCALL の次）から再開する。C は `fnd_sys_proc_fork()` / libfnd 1.2 の `fnd_proc_fork()`、sol は `fnd_proc_fork`。
- VM の所有フレームは byte 参照数で管理する（プロセス上限 7）。PTE の SW bit 8 は OWNED、bit 9 は COW。RX は共有のみ、RW は W を落として COW。ページ表は共有しない。子の表をすべて作ってから親の W を落とすため、途中の確保失敗は子の参照を戻すだけでよい。
- COW の store protection fault `0x8006000A` は VFAULT のページを私有化し、TLB を消去して同じ EPC から再実行する。最後の所有者ならコピー不要。RX の書き込み、未マップ、COW の確保不足は従来どおり CAUSE を終了コードにする。
- S モードの出力も必ず COW を解決する。`proc_user_range_ok(..., W)` は全範囲の権限確認後に私有化する。失敗は従来のポインタ検査エラー（ACCESS）、出力データは変更しない。ページの一部を私有化してから不足した場合、その私有化は巻き戻さない。
- IPC/IRQ/PeC/wait/blk_io の遅延出力は検査済み物理ページを前提にする。生存スレッド 1 本に fork を制限する理由は、別スレッドがブロック中の出力先を fork で再共有してしまうのを防ぐため。複数スレッド対応を加えるなら pin または全遅延出力の COW/OOM 再検証が必要。
- `mm_vm_map` が既存 COW ページに W を足す際も私有化が必要。`mem_grow(8)` のようなページ内の伸長で、親子に同じ書き込み可能フレームを渡さないこと。
- ハンドルは穴を含め同じ番号で複製し、オブジェクト参照数を増やす。ファイル位置・fs セッションなどは共有。新しい子の PROCESS ハンドルは親にだけ追加する。THREAD ハンドルと MMIO 付与は ACCESS、複数生存スレッドは BUSY、slot は UNSUPPORTED。
- `sys_info` のプロセスメモリ量は共有分を各プロセスに含める。free_bytes は物理フレームの空きなので、各プロセスの使用量を足しても物理使用量にはならない。
- `python -m unittest discover -s tools/llvm-src32 -p test_toolchain.py -k test_foundation_fork -v`: VM/slot × O0/O1/O2 × scalar/cached/cached-relaxed/pipeline の 24 実行。少量 RAM の VM で共有・親子孫の store・TLB・kernel/IPC 出力・失敗途中の参照回収・OOM・mem_grow・kill・RX 保護・終了時の全フレーム回収を検証する。
- Windows では TEMP/TMP を workspace 内に設定し、SRC32_TESTBENCH に release 版の絶対パスを設定すると実行時間を抑えられる。pytest の標準入力 capture で WinError 6/50 が出る環境では `-s` で実行できた。

- 検証結果: fork 回帰は 24 実行すべて成功。既存 VM 回帰は O0/O1/O2 × CPU 4 種の 12 実行成功。sol Foundation は `python -m pytest tools/solc/test_sol_foundation.py -q -s` で 27 件成功。
- slot シェル全体は 29 件中 28 件成功。`test_ctrl_c_kills_running_command` は `^C` が top の行末に残って独立行の検査が失敗する。変更前 HEAD `458294f` の Foundation ソースを `target/fork-shell-baseline/` に複製してビルドした場合も、同じ画面・同じ失敗を再現した。#141 を bug/on-hold で再オープン済み。COW/fork の変更ではシェルの表示を修正しない。
- VM シェル全体は `python -m unittest discover -s foundation/tests/shell -v` で 29 件すべて成功（release testbench）。
