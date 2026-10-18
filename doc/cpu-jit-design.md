# Cranelift Dynarec 導入案

2026-10-18。初期実装済み。以下に実装状況と設計方針を記録する。

## 初期実装の状況

`--features jit` 付き build で `--cpu jit` / YAML の `cpu.microarchitecture: jit` が使える。Cranelift 0.117.2 を既存 Wasmtime 系列と揃えて直接利用する。`src/cpu_jit.rs` がコード生成・dispatch・ABI 境界を持ち、Cached と cycle 開始処理・fallback issue を共有する。

同じ開始 PC が16回 dispatch されると、最大16命令をコンパイルする。対象は Normal mode の整数 ALU、half immediate、CPUID、通常分岐・call・return。未対応命令の手前でブロックを終え、Short mode・memory・乗除算・HALT・IRET などは Cached を使う。生成コードは明示的な register array と helper だけを参照し、helper の panic は捕捉して native call から戻った後に Rust 側で再送出する。

設計案の「全 page の世代を block に保存する」方式に代えて、**各命令の device/DMA 更新後に既存 Cached fetch を実行し、世代検証済みの decode 値と生成時の命令を比較する**。これにより cold fetch の bus 副作用も維持する。途中で変わった命令はそのサイクルに Cached で実行し、block を無効化する。MMIO 先読みは追加せず、RAM 専用の副作用なし preview API で compile 対象を読む。block key は Normal mode 専用の PC で、他 mode は dispatch 前に除外する。

dispatch entry と module 内の累積定義数は各1024まで。まず未compileのcold entryを破棄し、それでも満杯の場合やnative定義数が上限に達した場合は、実行終了後に module 全体を解放・再作成する。CPU reset でも解放する。無効化だけでは実行中の code や metadata を解放しない。失敗 block は cache reset まで、未対応開始位置は entry 破棄まで fallback を使う。`jit_stats()` と headless harness が compile/fallback/無効化などを表示し、native code bytes は allocator の padding・metadata を含まない。fallback cyclesは実際のinterpreter tick数で、HALT中のまとめ進行は含まない。統計は CPU reset をまたいで累積する。

debug step と `--profile-cpu` は Cached 経路を使う。JIT の wall time は profiler 無効で測る。合成比較用の `cpu_jit_bench` は cold と warm を分け、Cached と architectural state を照合する。実行例:

```powershell
cargo run --release --features jit --bin cpt32 -- program.bin --cpu jit
cargo run --release --features jit --bin cpu_jit_bench -- 400000 7
cargo test --features jit --test cpu_jit --test cpu_select --test cpu_pipeline
```

以下は導入時の設計方針。load/store の直接変換、block linking、device 処理の一括化は後続の性能検証を前提とする。

## 初期版の検証・実測

Windows x64 の同一ホスト、release build、profiler 無効で計測した。下表の倍率は Cached 時間 / JIT 時間で、1未満はJITの方が遅い。7 samplesの中央値であり、このホストとworkloadに限定する。

| 合成ケース（400,000 cycles/phase） | Cached | JIT | 倍率 |
|---|---:|---:|---:|
| ALU + branch、cold | 34.518 ms | 34.284 ms | 1.007x |
| ALU + branch、warm | 34.284 ms | 33.730 ms | 1.016x |
| ALU + MUL + memory、cold | 39.734 ms | 42.188 ms | 0.942x |
| ALU + MUL + memory、warm | 39.729 ms | 41.524 ms | 0.957x |

`cpu_jit_bench 400000 7`。CPU/module作成とprogram loadは計測外で、coldにはblock compileを含む。warmは同じCPUの続きの400,000 cyclesを測る。CPU実行以外の処理やcompileを伴う別workloadへ外挿しない。ALUの数%以内の差は以前の試行では逆転しており、安定した高速化とは判断しない。

| 実ゲスト、headless、process起動込み | Cached | JIT | 倍率 |
|---|---:|---:|---:|
| 作業ツリーの dos.wax、boot 5 frames | 58.325 ms | 75.512 ms | 0.772x |
| 現在の 3d_cube.sol、startup 10 frames | 968.814 ms | 1065.546 ms | 0.909x |
| 同ゲスト、60 frames（HALT後の進行を含む） | 1667.552 ms | 1835.725 ms | 0.908x |

空configのworking directoryからreleaseの`src32_testbench`を起動し、各frameは800,000 cycles、Cached/JITの実行順を交互にした。3d_cubeは今回のsourceをsolc→asmでraw binaryにした。10 framesでは初期化途中で、SGC glyph / VPU cubeの描画assertionはCached/JITとも未成立だった。60 framesでは両方ともノイズ背景・VPU cube・SGC overlayのassertionが通過し、CPUはHALTしていた。60-frame時間は定常animationのframe latencyではない。GUI/GPUは計測していない。

最終60-frame JIT smokeは186 blocks、compile 17.0342 ms、native 2,671,075 instructions、code 27,823 bytes、compile failure / cache reset は0。cold entryでhot codeを破棄していた修正前は同じゲストで3121 compiles / 94 resetsとなったため、cold entryの先行破棄を回帰テストに追加した。

最終結果は、機能するJIT基盤だがCachedを置き換える性能上の根拠はまだない。次はfallbackが多いload/store経路と、各命令のCached fetch・device/IRQ・register同期・ABI呼出しを個別に測る。cycleごとの観測契約を維持する方法を定義してから最適化する。

検証:

- `cargo test --release --features jit --test cpu_jit --test cpu --test cpu_select --test cpu_pipeline --test dos_smoke`：69 passed（JIT固有10、既存CPU21、selection11、pipeline17、DOS10）。追加のMMIO PC同期・HALT復帰確認後にもJIT固有10件を再実行。
- `cargo test --no-default-features --test cpu --test cpu_select --test cpu_pipeline`：49 passed。featureなしの`jit`選択エラーも確認。
- JIT固有testは固定seedのALU differential、各cycleと可変run budget、分岐/call/link register、Short/memory/乗除算fallback、IRQ/IRET、HALT復帰、自己書換え・page境界・load/reset、DMAによるblock途中の変更とIRQ、非preview領域、panic回収、cold pathのcache圧迫を確認する。

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
