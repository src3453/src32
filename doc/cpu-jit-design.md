# Cranelift Dynarec 導入案

2026-10-18。設計提案であり、`--cpu jit` はまだ実装されていない。

## 方針

SRC32 命令を Cranelift IR に変換し、ホストのネイティブコードとして実行する独立した JIT CPU を追加する。最初の互換性基準は Cached CPU とし、Pipeline のハザード・flush timing の再現は対象にしない。既定の Pipeline は維持する。sol の VM を JIT 化する案ではなく、コンパイル済みゲストの CPU 実行を高速化する案である。

Cranelift は命令セットの意味、割り込み、バス、コード無効化を自動で実装しない。これらはエミュレータ側で保証する。既存の Wasmtime は拡張カード用であり、CPU 用には `cranelift-codegen`、`cranelift-frontend`、`cranelift-native`、`cranelift-module`、`cranelift-jit` を直接利用する構成を候補とする。現在の Cargo.lock の Cranelift は 0.117.2。導入時は同系列で必要な API とホスト対応を確認し、Wasmtime 更新と混在させない。

## 初期実装の契約

- optional Cargo feature `jit` と `CpuKind::Jit` / `AnyCpu` の経路を追加する案。feature 無効時の `--cpu jit` は明示的なエラーとし、黙って別 CPU にしない。
- 命令意味と CPUID は Cached に合わせる（features `0x3F`）。単純命令 1 cycle、MUL/MULH 3、DIV/DIVU/MOD 16。予算途中で停止した乗除算は次の run で続行し、早期 commit しない。
- 最初は RAM 上の Normal mode の整数 ALU と分岐を対象にする。未対応命令、Short mode、MMIO、IRET、HALT は状態を同期して既存 Cached 経路へ戻す。fallback 後の PC/mode と消費 cycle を正しく引き継ぐ。
- 分岐、mode 切替、store、未対応命令、block 長上限で block を終了する。先読みには副作用のない RAM 読出しを使い、MMIO の投機的 fetch はしない。
- R0、32-bit wrap、比較結果、shift count、符号、big-endian memory は既存 CPU と一致させる。後で除算を直接変換する際は、ゼロ除算・signed overflow によるホスト trap を防ぎ、既存の結果を再現する。
- Rust の `Cpu` 全体のレイアウトを生成コードから直接参照しない。明示的な ABI の実行状態と helper を定義し、unsafe は呼出し・所有権境界に限定する。helper から生成コードをまたいで panic を unwind させない。

## デバイス・IRQ・コード変更

`Cpu::tick_cached` はデバイス更新、IRQ 入力取得、CPU MMIO 状態同期、命令実行、commit と IRQ 受理を所定の順に行う。初期 JIT もこの順序を維持し、各 cycle のデバイス処理を省略しない。block 開始時だけの IRQ 確認では不十分である。`run(max_cycles)` の予算境界、debug step、IRQ、外部からの PC/status 更新で正確に退出できるようにする。

RAM の既存 4 KiB page generation を再利用する。block key は PC と instruction mode を含み、block 全体が参照する全 page の世代を記録する。入口検証だけでなく、途中の device tick で DMA が後続命令を書き換える場合にも、変更済み命令の実行前に退出する。初期版は各命令前に世代を再検証する保守的な方式でよい。CPU store は block 終端とし、debugger・DMA・load/reset による変更も検証する。追跡不能な領域は fallback とする。

生成コードのキャッシュ容量に上限を置く。無効化はまず lookup から除去し、実行中コードのメモリを解放しない。初期版は実行終了後に module 単位で破棄・再作成する方式を候補とする。reset 時には dispatch と世代情報の両方を捨てる。

VSync/IRQ0 の発生元は引き続きホスト frame loop。JIT 内に独自の cycle ベース VSync を追加しない。

## 実装順序と合格条件

1. Cached と共有する命令境界・cycle 処理を整理し、既存回帰を維持する。仕様変更と高速化を同時に入れない。
2. optional JIT と bounded block cache、整数 ALU/分岐、fallback、世代検証を実装する。compile 失敗はゲスト状態を変えず Cached 実行へ戻し、失敗 block の再 compile を抑制する。
3. Cached との differential test で各命令境界の registers、PC/mode、memory、cycles、retired count、IRQ 状態を比較する。乱数 seed を固定し、Normal/Short 切替、R0、算術境界、予算途中の乗除算、HALT 復帰、IRET、自己書換え、複数 page、DMA による block 途中の変更、reset と debugger 操作を含める。既存 `cpu_select`、`cpu_pipeline`、DOS smoke も実行する。
4. release で `cpu_bench`、DOS DIR、3d_cube を Cached/Pipeline と比較する。compile を含む cold 時間と warm 時間、compile 回数・時間、cache memory、fallback/無効化回数を別記する。初期化と定常 frame を分離し、同じ cycle 予算で描画・IRQ/DMA 結果も照合する。profiler 無効の wall time を採否に使う。
5. 正しさと実ゲストの改善を確認してから load/store の直接変換、hot block のみの compile、block linking を検討する。デバイス更新の一括化は別変更とし、イベント deadline と DMA arbitration を保てる条件を先に定義する。

## 期待値と制約

既存の性能調査では Cached の Device/IRQ が測定 stage の 42% を占めたケースがある。JIT は decode/dispatch と演算の費用を減らせるが、毎 cycle の Bus 処理と同期が残る。profile 自体にも大きな overhead があるので、この割合から速度倍率を約束しない。初期の厳密な JIT が Cached より遅い可能性も含めて測る。結果は `emulator-performance-plan.md` の測定条件と合わせて記録する。

## 参考

- [Bytecode Alliance の JIT example](https://github.com/bytecodealliance/cranelift-jit-demo)：IR から JITModule へ渡す構成。
- [JITModule API](https://docs.rs/cranelift-jit/0.117.2/cranelift_jit/struct.JITModule.html)：生成関数の確定、取得、メモリ寿命。導入時は採用する版の API で検証する。
- リポジトリ内の根拠：`doc/spec_CPU.md`、`src/cpu.rs`、`src/cpu_select.rs`、`src/bus.rs`、`src/devices/ram.rs`。
