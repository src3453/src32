# Rust で CPT32 / Foundation のプログラムを書く

`no_std` の Rust を [`tools/llvm-src32`](../../tools/llvm-src32/README.md) の SRC32 バックエンドでコンパイルします。rustc は C と同じ 32-bit big-endian MIPS o32 のデータレイアウトで LLVM IR を出すだけで、MIPS の機械語は作りません。機械語・WAX・WAX v3 は C と同じ `driver.py` が作ります。

| クレート | 用途 |
|---|---|
| [`cpt32`](cpt32/) | ベアメタル（BIOS なし、S モード）。HALT／Extension P の intrinsic、MMIO、PCG 文字画面、UART、panic handler |
| [`fnd`](fnd/) | Foundation OS のプログラム。libfnd（`/lib/fnd.1.wal`）の束縛、引数・環境変数、`print!`/`println!`、ファイル、コンソール、panic handler、`alloc` 用 global allocator |

実例: [`examples/rust-baremetal`](../../examples/rust-baremetal/README.md)、[`examples/rust-foundation`](../../examples/rust-foundation/README.md)。

## 必要なもの

- `tools/llvm-src32` の要件（Python、llvmlite、Clang）。Clang は Rust から呼ばれる `memcpy` などのランタイムと SoftFloat のコンパイルに使います。
- Rust（rustup）と `rust-src`: `rustup component add rust-src`。`core`（と `alloc`）をこのターゲット用にソースからビルドします。
- **rustc の LLVM メジャー版が llvmlite と同じこと**（`rustc -vV` の `LLVM version` と `python -c "import llvmlite.binding as l; print(l.llvm_version_info)"`）。違うとテキスト IR を読めないので、`cargo_src32.py` が止まります。Rust 1.96 stable（LLVM 22）と llvmlite 0.50（LLVM 22）で検証しています。
- stable の Rust では `-Zbuild-std` と JSON ターゲット仕様が使えないため、`cargo_src32.py` は `RUSTC_BOOTSTRAP=1` を付けて Cargo を起動します（nightly ならそのまま）。

## ビルド

```sh
python tools/llvm-src32/cargo_src32.py --manifest-path path/to/Cargo.toml -o game.wax
```

- プログラムは **ライブラリクレート**（`src/lib.rs`）です。`#[unsafe(no_mangle)] pub extern "C" fn main() -> i32` が入口で、C と同じく戻り値を R1 に置いて HALT します（引数 `u32` 2 つまでで起動時の R1/R2 を受け取れます）。`Cargo.toml` の `crate-type` は不要です（`staticlib` として固定でビルドします）。
- `cargo_src32.py` は指定したプロファイル（既定 `release`）に `panic = "abort"`、`lto = "fat"`、`codegen-units = 1`、`debug = false` を強制します。最適化レベルは `Cargo.toml` の `opt-level` に従います（`--profile dev` は 0、整数オーバーフロー検査付き）。
- 自分のオプション（`--manifest-path`、`--profile`、`--features`、`--no-default-features`、`--build-std`、`--target-dir`、`--cargo`、`--emit-rust-ir`、`--foundation-wal`、`-v`）以外は `driver.py` へそのまま渡します（`-o`、`--origin`、`--stack-top`、`--clang`、`--protected`、`--wal --exports`、追加の `.c` ソースなど）。
- 仕組み: `cargo rustc --crate-type staticlib --target tools/llvm-src32/rust/src32-none.json -Zbuild-std=core` に `--emit=llvm-ir` を付けます。fat LTO のため、rustc が書く IR は LTO 後の 1 モジュールで、クレート・依存クレート・`core` を全部含みます。その IR から未使用の関数（opt-level 0 では `f16` を使う関数なども残る）を LLVM の globaldce で除いて `driver.py` に渡します。`compiler_builtins` は LTO に入らないので、`memcpy` などは `driver.py` のランタイムが、浮動小数点は SoftFloat が提供します。

### ベアメタル

```toml
[dependencies]
cpt32 = { path = "/path/to/src32/sdk/rust/cpt32" }
```

```rust
#![no_std]
use core::fmt::Write;

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    let mut pcg = cpt32::pcg::Pcg::init();
    let _ = writeln!(pcg, "1 + 2 = {}", 1 + 2);
    42
}
```

- `cpt32` の既定の feature `panic-handler` は、位置（`panic at src/lib.rs:12`）を UART に書いて HALT し続けます。自分の `#[panic_handler]` を使うときは `default-features = false`。
- `cpt32` の関数を何も使わないクレートでは、Rust が依存をリンクせず panic handler も見つかりません。`use cpt32 as _;` を書いてください。
- `cpu::syscall::<N, CTX>(…)`、`cpu::mfsr::<SR>()` は番号を const generic で受け取ります（命令の即値なので定数でなければならない）。
- IRQ／SYSCALL ハンドラーは C と同じ名前の関数を定義します（`#[unsafe(no_mangle)] pub extern "C" fn irqc_dispatch()`、`syscall_dispatch(frame: *mut [u32; 32])` など）。契約は [ツールチェーン README](../../tools/llvm-src32/README.md) の「Extension P」。
- C の SDK（`sdk/cpt32`）の関数も、`extern "C"` で宣言して `.c` ファイルを `cargo_src32.py` の引数に足せばリンクできます（`-I sdk/cpt32/include` も渡す）。

### Foundation

```toml
[dependencies]
fnd = { path = "/path/to/src32/sdk/rust/fnd", features = ["global-allocator"] }
```

```rust
#![no_std]
extern crate alloc;

fnd::main!(run);

fn run() -> i32 {
    let words: alloc::vec::Vec<&str> = fnd::args().collect();
    fnd::println!("{} argument(s)", words.len());
    0
}
```

```sh
python foundation/run.py --build-only               # target/foundation-boot/root/lib/fnd.1.wal を作る
python tools/llvm-src32/cargo_src32.py --manifest-path Cargo.toml --build-std core,alloc \
    --foundation-wal target/foundation-boot/root/lib/fnd.1.wal -o bin/hello.wax
python foundation/run.py --bin-dir bin               # /bin/hello.wax を入れたディスクで起動
```

- `foundation/boot/build.py OUT --bin-dir bin` でも同じディスク（`OUT/foundation.img`）を作れます。

- `--foundation-wal WAL` は `driver.py` の `--wax3 --page-align --pie --import-wal WAL` の省略形で、`foundation/bin` の C コマンドと同じ再配置可能な WAX v3 になります。libfnd の関数は import され、`/lib/fnd.1.wal` は起動時にカーネルが読み込みます。
- `fnd::main!(f)` は `fnd_init` を呼び、`f()` の戻り値で `fnd_exit` します（C の `CMD_MAIN`）。
- `alloc`（`Vec`、`String`、`format!` など）を使うときは `--build-std core,alloc` と feature `global-allocator`（`fnd_mem_alloc`/`fnd_mem_free`。プロセスのヒープは既定 128 KiB、`--heap-hint BYTES` で変更）。
- `fnd` の既定の panic handler は `panic at <file>:<line>: <message>` を STDERR に書き、終了コード 101 で終わります。
- `fnd::sys` が生の束縛です。WAL 境界を越えられるのは `u32`/`i32`/ポインタだけです（[WAX v3 §6](../../doc/spec_WAX_v3.md)）。

## 制約

`tools/llvm-src32` のバックエンドの制約がそのまま当てはまります。特に Rust では:

- `no_std` のみ（`std` はありません）。`alloc` は Foundation では `fnd` の allocator、ベアメタルでは自分で `#[global_allocator]` を用意すれば使えます。
- アトミック: ターゲットの `max-atomic-width` は 0 です。`core::sync::atomic` の型と `alloc::sync::Arc` は使えません（`Rc`、`Cell`、`RefCell` は使えます）。
- `i128`/`u128`、`f16`/`f128`、SIMD、インラインアセンブリ（空の `asm!("")` バリアだけは可。`core::hint::black_box` が使う）は使えません。使うとコンパイル時にエラーになります。
- 浮動小数点は SoftFloat の呼び出しになり遅く、全 SSA 値を spill するバックエンドなのでコードは大きくなります（`core::fmt` を使うと数十 KB）。速度とサイズが重要なら `opt-level = "s"` か 2 を使い、`core::fmt` を避けます。
- panic は `abort` のみ（unwind なし）。
