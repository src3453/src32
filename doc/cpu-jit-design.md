# Cranelift Dynarec 導入案

最新の改善は「dispatchボトルネック、乗除算・memory対応（2026-10-20）」、その前は「JIT往復削減と段階的compile」を参照。関連する後続実験として `--cpu cached-relaxed` とJITへの待機device batchingを追加した。JITも最大32 cycles（2026-10-20に256へ拡大）にまとめるが、CPU IRQ sampling、各命令のコード世代検証、native blockのcommit、run budget確認は命令境界ごとに保つ。MMIO／timer deadline／HALT／IRETとactive DMA／IDC／expansion時は同期する。[性能調査の結果](emulator-performance-plan.md#実験的cached-relaxedモード)を参照。

2026-10-18。初期実装済み。以下に実装状況と設計方針を記録する。

## 初期実装の状況

`--features jit` 付き build で `--cpu jit` / YAML の `cpu.microarchitecture: jit` が使える。Cranelift 0.117.2 を既存 Wasmtime 系列と揃えて直接利用する。`src/cpu_jit.rs` がコード生成・dispatch・ABI 境界を持ち、Cached と cycle 開始処理・fallback issue を共有する。

ブロック命令上限は `cpu.jit_block_size` で2〜256（2026-10-20に64から拡大）に設定し、既定値は16。Launcherの詳細設定はSmall=8、Medium=16、Big=32を提示し、具体値も入力できる。previewが長くなっても分岐命令・未対応命令でblockを終え、命令ごとの検証契約は変えない。大きな値は生成コード量を増やすため、性能と安定性を環境ごとに確認する。

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

## JIT往復削減と段階的compile

2026-10-18、既に待機device batchingを有効にした作業ツリーから追加改善した。上の初期導入の数値と今回のbeforeは異なる。遅さの主因は、native ALUごとのRust ABI呼出し、Cached fetch、CPU register MMIOの往復コピーとIRQ確認、および未対応命令へのfallbackだった。例えばcubeの60-frame最終smokeではnative 1,795,618命令に対してfallback 11,136,733 cyclesであり、ALUだけの高速化には限界がある（cycleと命令数を同一視しない）。

改善内容:

- device更新が起きない待機区間だけ、最大16命令のprefixを純粋なnative関数で実行する。命令ごとのRust helperとregister同期を省く。Busが返す予算は既存32-cycle更新／timer期限の**1 cycle手前まで**。区間を使い切る最後のcycleは保守的経路へ戻す。
- prefix全体が既存decode cacheにhitし、命令identityと先頭・末尾の世代が一致することを実行前に確認する。cold fetchや変更があれば従来経路へ戻す。DMA／IDC動作中、拡張カード、有効なIRQ pending時には一括実行しない。命令内のload/storeも対象外なので、区間内でcodeやCPU MMIOを書き換える主体がない。IRQは入口で観測し、register MMIOと実行カウンターは出口で同期する。run予算とcache hit数は従来通り。
- 通常native関数は16 visits、一括実行用は128 visitsで生成する。1命令blockは解釈実行に残す。Short／HALT／乗除算待ちではrun loopからJIT呼出しを省く。二段目の失敗は保守的native関数を保持し、同じentryで再試行しない。
- `native_batches` / `batched_instructions` / `batch_compilations` を統計に追加する。`compilations`は一段目block数、`compile_time`と`native_code_bytes`は両段階の合計。1024の上限はblock定義数で、各blockは最大2 native関数を所有する。関数名はcompile時のserialに紐づけ、遅延compile時の最新serialを使い回さない。

最初から両方の関数を生成してCraneliftのspeed最適化を有効にした試行では、DOS compileが約27 msとなり起動が悪化した。最終版は段階的生成と従来のCranelift既定設定を使う。精度許容幅は既存の待機device batchingから増やしていない。VSyncはホストframe loopのまま。

Windows x64、release、profiler無効。合成は800,000 cycles/phase・9 samples中央値。beforeは変更前exeを保存して連続実行した値で、ホスト負荷差は残る。CPU/module作成とprogram loadは計測外、coldはcompile込み、warmは同じCPUを継続する。

| 合成ケース | 変更前JIT cold / warm | 変更後JIT cold / warm | warm倍率 |
|---|---:|---:|---:|
| ALU＋branch、RAMのみ | 19.493 / 19.099 ms | 13.340 / 11.877 ms | 1.608x |
| ALU＋MUL＋memory、RAMのみ | 37.967 / 37.705 ms | 32.467 / 32.318 ms | 1.167x |

変更後の同一ベンチでは、warm ALUは通常Cached 70.614 / cached-relaxed 18.901 / JIT 11.877 ms。混在は80.133 / 32.480 / 32.318 ms。`--platform`で待機BMC／DMAC／PeC／timer／空IDCを接続すると、warm ALUは83.977 / 19.201 / 12.017 ms、混在は99.950 / 39.343 / 38.758 ms。一括device処理の効果をJIT固有の効果と混同しない。混在でのJITとcached-relaxedの差は小さい。

```powershell
cargo run --release --features jit --bin cpu_jit_bench -- 800000 9
cargo run --release --features jit --bin cpu_jit_bench -- 800000 9 --platform
```

実ゲストは既存の`dos.wax`と`target/relaxed-cube.bin`をbefore/afterで共用し、空configのdirectoryからheadless harnessを起動した。プロセス起動込み、7 samples中央値、実行順を3方式で巡回。各frameは800,000 cycles。

| 実ゲスト | 変更前JIT | 変更後JIT | cached-relaxed |
|---|---:|---:|---:|
| DOS boot、5 frames | 53.791 ms | 54.442 ms | 34.085 ms |
| cube startup、10 frames | 458.432 ms | 434.584 ms | 372.873 ms |
| cube描画完了、60 frames | 926.754 ms | 867.300 ms | 764.259 ms |

cubeは約5〜6%短縮、DOSは約1%遅くほぼ横ばい。ホスト負荷による外れ値もあり、厳密な一定倍率は主張しない。実ゲストでは依然cached-relaxedが速く、残る改善候補はShort mode・memory・乗除算fallbackとdispatch費用。cubeはhost TRNGでR1が変わるため、PC／cycles／runningと各方式の60-frame描画assertionを照合した。60-frameはHALT後の時間進行を含み、GUI／GPUや定常animationの測定ではない。

検証はreleaseのCPU/JIT/relaxed/selection/pipeline/DOS/expansion計98件、Bus batching unit test 2件、featureなしCPU/selection/pipeline計49件。debug buildでもJIT 13件を通過し、idle予算のdebug assertionを確認した。JIT追加testは1〜65 cycle予算、page跨ぎ、CPU register MMIO結果、block後半の自己書換え、1〜65 cycleのtimer期限と外部IRQをCachedと比較し、実際の一括native実行もassertする。既存のDMA途中のcode変更／IRQ、拡張カードdeadline、panic transport、HALT／IRET、debug/profiler回帰も維持する。

## dispatchボトルネック、乗除算・memory対応（2026-10-20）

`cpu_jit_bench --platform` のwarm ALUでJITは14.5 ns/cycleかかり、生成コードの速さを生かせていなかった。一時的な`rdtsc`計測でbatch経路を分解すると、命令ごとのdecode cache検証（命令ごとに世代lookupを2回、各lookupがdeviceの線形探索）が52%、dispatch `lookup`（SipHashの`HashMap`、blockあたり最大3回hash）が31%、register blockの同期in/outが各6%で、**生成コード自体は6%**だった。

改善内容:

- `Bus::instruction_cache_generation`は最後に応答したdeviceを`Cell`で覚える（regionは重ならないので全走査と同じ答え）。`Device::instruction_cache_unit`（RAMは4 KiB page）で、同じ世代tokenを共有する範囲を返す`instruction_cache_generation_span`を追加した。Cached interpreterのfetchも速くなる。
- blockの検証結果をstampする。全命令がhitしたblockは、CPUの`instruction_cache_fills`（decode cache slotへの書込みごとに増加）が変わらず、覆うpage（最大2単位）の世代も同じなら、次回は命令ごとの検証を省く。
- dispatch mapはPC用の乗算hasherを使い、確立済みblockのhot pathは1回のprobeで返す。compile／prune／resetが必要な場合だけ従来経路に入る。
- MUL/MULH/DIV/DIVU/MODをnative化した。0除算と`INT_MIN / -1`はtrapしないselectで解釈実行と同じ結果にする。batchのprefixは命令数でなくCached cycle数（1/3/16）で待機区間に収める。strict関数では乗除算を解釈実行のstall FSMへ渡す。
- LD/LDH/LDB/ST/STH/STBをbatchで扱う。helperが経過cycleを`defer_native_idle_cycles`で渡してから解釈実行の`execute_normal`を呼ぶので、Busのflush・調停・RAM世代更新は`cached-relaxed`と同じ。plain memoryに収まらないアドレスはアクセス前に、IRQC出力の変化や自block code pageの世代変化はアクセス直後にbatchを止める。
- 連続batchはregister blockを同期せず連結する。間に起きうるのはplain memoryアクセスだけで、CPU register MMIOに触れず、IRQC変化ではbatchが止まるため。
- relaxed待機区間を32から256 cycleに拡大した（`RELAXED_DEVICE_WINDOW`）。区間ごとに1命令がstrict経路でdeviceをflushするので、32ではALU loopのfallbackが約1%残った。ホスト入力の観測遅延は最大5.3 µs。

実ゲストでは別の問題が見えた。`test.wax` 60 framesでJIT（block 16）が385 ms、cached-relaxedが356 msで、JITのほうが遅く、compile時間が183 msを占めた。strict関数は命令ごとに`prepare`を呼ぶため解釈実行と大差なく、それなのにcompile時間の約63%を使っていた（実行されたnative命令8.2Mのうちstrictは約15万）。対策:

- batch関数だけを128 visitsで生成し、strict関数はhot blockがbatchを32回辞退した後に遅延生成する。テストは`AnyCpu::set_jit_tier_thresholds(16, 0)`（doc hidden）で従来どおり両段階を早期生成して検証する。
- release buildでCranelift IR verifierを無効化した（debug buildでは有効）。`opt_level=speed`は効果がなく、single-pass register allocatorはcompileが約40%速いが実績が少ないため採用しない。
- `finalize_definitions`は1回5 µs程度で、支配的なのはCraneliftの`define_function`そのものだった。

Windows x64 release、profiler無効。合成は800,000 cycles/phase・9 samples中央値（同一build内で比較）。

| 合成（platform） | 開始時JIT warm | 最終JIT warm | Cached | cached-relaxed |
|---|---:|---:|---:|---:|
| ALU+branch | 11.580 ms | 2.349 ms | 31.369 ms | 19.218 ms |
| ALU+MUL+memory | 19.167 ms | 6.593 ms | 29.752 ms | 20.637 ms |

mixedのfallback cyclesは640,132から563近くへ減った。実ゲスト（`src32_testbench … --frames 60`、空config、process起動込み、7回の最良値）: `test.wax` JIT 312 ms、cached-relaxed 370 ms、Cached 473 ms、Pipeline 766 ms。`dos.wax`はほぼHALTでJIT 55 ms、relaxed 52 ms。20 frames時点のPC・cycles・R1はJIT／relaxed／Cachedで一致。block sizeは16→32→64→256で`test.wax`が302→322→337→343 msと悪化したため既定は16のまま（上限のみ256へ拡大）。

## 導入時の設計方針

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

`Cpu::tick_cached` はデバイス更新、IRQ 入力取得、CPU MMIO 状態同期、命令実行、commit と IRQ 受理を所定の順に行う。JITは各native命令境界でIRQ入力・CPU MMIO同期とコード世代を確認するが、既定のJIT実行では待機デバイス更新だけを最大256 cycleまとめる（実験的cached-relaxedと同じ契約）。Known deadline、MMIO、active DMA/IDC/expansion、run予算境界で同期する。block開始時だけのIRQ確認では不十分である。`run(max_cycles)` の予算境界、debug step、IRQ、外部からの PC/status 更新で正確に退出できるようにする。

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
