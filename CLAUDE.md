# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

作業規則・保つべき契約・focused なテストコマンドは `AGENTS.md` にあり、ここに取り込みます。重複させず、この文書には全体像と横断的な注意点だけを書きます。

@AGENTS.md

`MEMORY.md` は過去の作業で得た知識と落とし穴の記録（日付付きの節）です。触る領域の節を先に読み、作業後は追記します。

## 全体構成

このリポジトリは「仮想ハードウェア」「その上で動くゲストソフト」「ゲストを作るツールチェーン」の 3 層です。仕様は `doc/spec_*.md` が正で、実装とテストはそれに合わせます。

- **エミュレータ（Rust, `src/`）**: `bus.rs` が MMIO バスで、`src/devices/` の各デバイス（BMC/DMAC、IRQC、PeC（タイマー・キーボード・IDC ディスク・RNG）、VDP（PCG 文字画面・GP・CLUT 合成）、SGC（2D）、VPU（3D）、SGU（音声）、拡張カード（wasmtime の WASM カード））をつなぎます。CPU は `cpu.rs`（スカラー）、`cpu_pipeline.rs`（既定、Extension p）、Cached／cached-relaxed、`cpu_jit.rs`（`--features jit`、Cranelift）があり、`cpu_select.rs` で切り替えます。特権レベルと SYSCALL は `cpu_privilege.rs`（`doc/spec_CPU_priv.md`）。
- **バイナリ（`src/bin/`）**: `cpt32`（GUI、`src/main.rs`）、`src32_testbench`（ヘッドレス実行。`--frames`、`--expect-mem-u8`、`--type FRAME=TEXT` のキー入力、`--dump-pcg` の画面ダンプなど。Python 側のゲストテストはほぼ全部これを使う）、`cpt32-tool`、`music_editor`、各種ベンチ。
- **ゲストの作り方は 2 系統**:
  - sol（スタック言語）: `tools/solc/solc.py compile X.sol -o X.a` → `tools/asm/asm.py X.a -o X.bin|.wax`。標準ライブラリは `sol/solstd`、`sol/cpt32`。
  - C/C++: `tools/llvm-src32/driver.py`（Clang で LLVM IR → llvmlite で SRC32 アセンブリ）。`--protected`（ユーザーモード WAX v2）、`--wal`／`--wax3`／`--import-wal`（WAX/WAL v3）。C ライブラリは `sdk/cpt32`。
- **実行形式**: raw `.bin`、WAX v2（`doc/spec_WAX.md`）、WAX/WAL v3（`doc/spec_WAX_v3.md`、位置独立ライブラリと import）。
- **BIOS**（`bios/`, `doc/spec_BIOS.md`）: `config.yaml` に `bios` があると BIOS が起動し、渡した RAM プログラムは実行されません。ゲストを直接動かすときは BIOS のない config の directory から実行します。

## Foundation（`foundation/`）

CPT32 用 OS。契約は `doc/spec_Foundation.md`（カーネル ABI）、`doc/spec_Foundation_API.md`（サービスのプロトコルと libfnd）、`doc/spec_Foundation_shell.md`、`doc/spec_FS2.md`。開発順序は `doc/foundation_os_plan.md`。現状と暫定部分の一覧は `foundation/README.md` にあり、暫定部分は横に新しい仕組みを足さず置き換えます。

- 起動経路: ディスク起動は BIOS → `boot/stage1.a`（510 byte 以内）→ FNDBOOT（`boot/fndboot.py` が作るモジュール表付きイメージ）→ カーネル。直接ロードの `foundation.wax` も同じモジュール表のコードを通ります。カーネルは console（slot 1）、fs（slot 3）、init（slot 0）を起動し、init が `/bin/shell.wax`（slot 4）を起動します。
- シェルはディスパッチャで、組み込みは `cd`/`set`/`exit`/`help` だけ。他のコマンドは `foundation/bin` の `/bin/<name>.wax`（slot 5）で、シェルとコマンドは WAL「fnd」1.0（`/lib/fnd.1.wal`、export は `lib/fnd/fnd.exports`）を import する WAX v3。init と各サービスは libfnd を静的リンクした WAX v2。
- `services/bootfs` は埋め込みの `/bin`・`/lib` を返す暫定の読み取り専用 fs で、本物の fs サービス（FS2 ドライバ）で置き換える予定。
- ビルドと実行: `python foundation/boot/build.py`、`python foundation/run.py [--disk] [--build-only] [--clang PATH]`（リポジトリルートで）。
- llvm-src32 の生成コードはバイト単位ループが非常に遅い（約 25 命令/byte）。カーネルのコピー・ゼロ埋めはワード単位で書きます。

## 開発環境の注意

- testbench のビルドには `libasound2-dev` と `libudev-dev` が必要です（Linux）。
- C ツールチェーンは `pip install -r tools/llvm-src32/requirements.txt`。Clang は llvmlite と同じ LLVM メジャー版を使います（`python -c "import llvmlite.binding as l; print(l.llvm_version_info[0])"`）。ずれると -O1/-O2 で "icmp constexprs" エラーになります。
- Python のゲストテストは環境変数 `SRC32_TESTBENCH` と `CLANG` で実行ファイルを指定できます（CI はこれを使用）。testbench は debug ビルドだと非常に遅いので、ローカルでは `cargo build --release --bin src32_testbench` を使うか、CI と同じく `CARGO_PROFILE_DEV_OPT_LEVEL=2` を設定します。
- `cargo test` の lib 単体テストは外部リポジトリ（s3w2_core）の golden ファイルが必要でビルドできないことがあります。統合テストは `cargo test --test <name>` で個別に実行します。`dos_smoke`／`tinybasic` などは、先に `dos.sol` などから `dos.bin` をリポジトリルートへビルドしておく必要があります（`.github/workflows/ci.yml` の手順参照）。
- CI（`.github/workflows/ci.yml`）の `known-failures` ジョブは既知の失敗（strek_*、tb_load_live）を非ブロッキングで回します。修正したら通常ジョブへ戻し、新しい失敗をここへ逃がしません。
- 単一テスト: テストディレクトリはパッケージではないので、`python -m unittest discover -s foundation/tests/shell -k <テスト名の一部>` のように `discover -s` と `-k` で絞ります。pytest のものは `python3 -m pytest tools/solc/test_sol_compiler.py -k <名前>`。
