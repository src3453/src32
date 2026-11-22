# llvm-src32: SSA 値のレジスタ割り当て（R13〜R27）

以前の `backend.py` は全 SSA 値をフレームのスロットに置き、使うたびに `LD`、定義するたびに `ST` していた（C の待ちループが毎周スタックへ書き戻す原因。`memory/2026-11-22-issue-206-spin-same-value-store.md`）。バックエンドは R13〜R27 を一切使っていなかったので、そこを値の置き場にした。

## 規約

- R13〜R20 は caller-saved（呼出しをまたがない値用、保存不要）、R21〜R27 は callee-saved（呼出しをまたぐ値用。使う関数だけがプロローグでフレームに保存し、エピローグで戻す。`_Function.saved_registers`）。この区別はこのツールチェーンが作る関数どうしの約束で、公開 call ABI 1（`doc/spec_WAX_v3.md` §6）は R1〜R27 を caller-saved と定める。なので import 関数の呼出し、`setjmp`/`longjmp`（longjmp は途中の関数のエピローグを通らない）、別の文脈へ移る `cpt32_run_user`/`cpt32_return_to_kernel`/`cpt32_ctx_resume`/`cpt32_jump` をまたぐ値はどちらの組にも置かない（`leaves_frame`）。IRQ 入口・SYSCALL・`kernel_trap` のスタブは R1〜R31 を全部保存・復元する。
- 「呼出しをまたぐ」は実際に `JALR` を出す命令で判定する（`calls_out`）: 通常・間接・import の呼出し、ソフトウェア浮動小数点のランタイムを呼ぶ float 演算と fcmp、float を含む `llvm.*`。インライン展開される intrinsic（memcpy ループ、オーバーフロー演算、setjmp 本体など）は含まない。
- **落とし穴**: `fcmp` と float の `llvm.*` は 1 命令の中でランタイムを 2 回呼び、途中結果を自分の値（スロット／レジスタ）に置いてから 2 回目の後で読み戻す。結果が自分の命令の中の呼出しをまたぐので、callee-saved の組に入れる（`inner_calls`）。これを忘れると `test_float_instructions_against_ieee_vectors` の -O2 が R1=20（cmp_d）で落ちる。
- 最初は R13〜R27 を全部 callee-saved にしたが、この CPU ではメモリアクセスとレジスタ間コピーのコストがほぼ同じなので、スロットの LD/ST をコピーに置き換えるだけでは命令数が減らず、呼出しごとの保存・復元の分だけ呼出しの多いコードが遅くなった（Foundation のカーネルが数 % 遅くなり、シェルのテストが停止 tick までに終わらなかった）。caller-saved の組を作ってリーフ関数と一時値の保存をなくした。

## 割り当てとコード生成

- 32 bit 以下のスカラー（整数・ポインタ・float）だけ。i64/double/集約型はスロットのまま。生存区間はブロック順の位置で凸包をとる（定義、使用、live-in/live-out のブロック境界。φ は各前任ブロックの終わりで書かれ、φ のオペランドもそこで読まれる）。端点が 1 か所でも重なれば干渉とみなすので、命令の結果レジスタは必ずオペランドのレジスタと別になる（`destination` がこれに依存）。線形スキャンで足りなければ終わりが最も遠い値をスロットへ戻す。
- `load`/`store` はレジスタ値ならレジスタ間コピー。よく使う経路は `operand()`（定数 0 は R0、レジスタ値はそのレジスタ、それ以外は作業用レジスタへ読む）と `destination()`/`finish()` で直接レジスタを使う: 32 bit 二項演算（定数加減算は `ADDI`）、32 bit 比較、スカラーの load/store（ポインタのレジスタをそのままベースに）、条件分岐、GEP（定数添字は畳み込み、2 のべき乗の要素サイズは `SLL`）。符号拡張など入力をその場で書き換える処理は作業用レジスタへコピーしてから行う（値のホームレジスタを壊さない）。φ のエッジコピーは 8 ワード以下なら R1〜R8 に全部読んでから書く。`_Code.imm` は 16 bit に収まる整数を `ADDI Rr, R0, imm` の 1 命令にする（fixup 付きの値は従来どおり LDIH/LDIL）。
- スカラー値でスロットを直接触る経路があると、レジスタ値にはスロットがないので KeyError（コンパイルエラー）になる。黙って誤ったコードにはならないが、新しい lowering では `load`/`store`/`copy_into` を使うこと。集約型の箇所（insertvalue、オーバーフロー intrinsic の構造体結果）はスロット前提のまま。

## Foundation シェルテストのタイミング依存

- `test_line_editing_and_history` は `echo nope\L\L\C` を先行入力していたが、コンソールは Ctrl+C をキーが届いた時点で処理する（§8.4: 実行中のコマンドを kill し、溜まったキーと編集中の行を捨てる）。結果がゲストの速度で変わり、変更前のコードでも `--type` の間隔を 14 フレーム以上にすると落ちていた。Ctrl+C を別のまとまりにして、先行コマンドが全部終わった後（430 フレーム）に送るようにし、`STOP_TICKS` を 800 → 1000 にした（各コマンドは spawn を含めて約 40 フレームかかる）。速度と関係なく比べるには、停止 tick を大きくして各まとまりを十分あけて送る（このとき変更前と新しいコード生成の画面出力は一致した）。
- 「止まった」ように見えても、停止 tick（`--stop-ticks`）に達しただけのことがある。プロンプト数をフレーム数ごとに数えて進み方を比べると切り分けられる。

## 効果と検証

- `examples/bench-c-rust/run.py`（C -O2、pipeline、ゲストのタイマーで測るサイクル数）の C 合計: 14,348,379 → 約 6.6M（約 2.1 倍速）。最も深いスタックは約半分以下。チェックサムは C/Rust とも一致。flappy-c の描画は DMA 待ちが大半で 1% しか変わらない。
- 検証: `python -m unittest test_toolchain`（57 件、`test_register_allocated_values_survive_calls_spills_and_longjmp` を追加。保存処理を外す／setjmp をまたぐ値を許すと失敗する）、`test_rust.py` と `foundation/tests/rust`、Foundation の shell（通常と `FND_MM=nommu`）・disk、`test_foundation_vm_kernel`/`fork`/`wax_loader`、flappy-c。`foundation/tests/wax` はこの環境では skip、`foundation/tests/cpu` はホストの C コンパイラがなく変更前から setUpClass で失敗、`sdk/cpt32/smoke/run.py` の `display` は変更前から失敗。
