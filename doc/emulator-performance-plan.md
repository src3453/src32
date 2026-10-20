# エミュレータ性能調査と改善計画

## 48 MHzへ向けた段階的最適化（2026-10-18）

目標はシステムクロック48 MHzの継続動作。48 MIPSとは区別し、
`running_cycles / CPU実行秒 / 1,000,000` を実効MHzとして測る。
48 MHzは平均20.83 ns/cycle、60 Hzでは800,000 cycles/frameに相当する。
HALT後のidle cyclesを含むguest wall timeから稼働中CPUのMHzを計算しない。
既定のPipelineを維持し、正確性を保つ改善と実験的Cached/JITの改善は別々に評価する。

| 段階 | 対象と実施順 | 次段階へ進むための確認 |
| --- | --- | --- |
| 0: 計測基準 | release・profilerなしでRAMのみ／待機platform構成を測定。実効MHzと48 MHz達成率、cycles・stalls・flushesを記録 | 同じguest・mode・build設定で複数sampleの中央値とばらつき、実行状態を照合 |
| 1: Bus/調停 | 単独requestのBMC探索を省略（今回）。次にDMAC request取得、IRQ集約、入力mutex、RefCell/Rc往復を個別計測 | grant・待機履歴・MMIO・IRQ・DMA結果が一致。active DMA/IDC/カードの経路も維持 |
| 2: Pipeline fetch | byteアクセスごとの負荷を分解し、decode・dispatch・命令取得経路を一つずつ改善 | 自己書換え・DMA書換え・page境界・MMIO fetch・分岐先・Short切替・bus errorの結果とアクセス順が一致 |
| 3: CPU内部 | register MMIO同期、forwarding、stall bookkeepingを計測し、不要なコピー／分岐を削減 | cycle予算途中、単一step、IRQ/IRET/HALT、MMIO経由のregister変更と既存pipelineカウンターが一致 |
| 4: 実験的高速方式 | Cached/JITのShort・memory・乗除算fallbackとnative dispatchを改善 | 通常Pipelineとの契約差を明示し、cached-relaxed単体も比較。速くなることを理由に既定modeや精度を暗黙に変えない |
| 5: 実用48 MHz | DOS操作・cube定常描画・音楽・active DMA/disk・WASMカードを含むGUIでCPU/render/audioを別計測 | cycleを落とさず48 MHzを維持し、frame時間の中央値/p95、入力応答と音声を確認 |

各変更は単独で前後比較し、再現できる改善がなければ取り込まない。
今回は64 KiBページ表によるBus lookupも試したが、DOS起動で約3%短縮する一方、
RAM主体の合成では遅くなったため見送った。
次回は段階1の残りの負荷を測ってから対象を選ぶ。各段階の倍率は保証しない。

GUI既定の8 ms time sliceで800,000 cyclesを毎frame処理するには、CPU部分だけで
100 MHz相当の処理能力が必要となる。48 MHz相当のCPUベンチ達成だけでGUIの
リアルタイム動作を達成したとは扱わない。`best-effort`のcycle打切りや
`cycle-independent`の進行を48 MHz達成の証拠にせず、最終的には`strict`で測る。

計測コマンド（引数は反復数とsample数）:

```sh
cargo run --release --bin cpu_bench -- 100000 9
cargo run --release --bin cpu_bench -- 100000 9 --platform
```

`--platform`はRAM/BMC/DMAC/SGU/VDP/PeC input・timer/RNG/未接続IDCを接続する。
CPU構築がIRQCを接続する。デバイスは待機状態で、active DMA/disk・カード、
VDP描画、音声合成、GUI/GPUを含まない。初期化・program loadは計測外、
最初の命令fetchは計測内。各CPUを64 cyclesずつrunし、開始順を巡回する。

### 第1段階: BMC単独要求の高速経路

単独requestかつgrant可能な場合、8 masterの優先度探索を省略する。
`pending`、`active`、`next_master`、全masterの待機履歴を従来と同じ状態にする。
調停無効時のDMA拒否を先に判定し、複数requestは従来の優先度・aging・
ラウンドロビン処理を使う。CPUのbyteアクセス回数、デバイス更新、IRQ、cycle数は変えない。

同一Windowsホスト・release・profilerなし、9 samplesの中央値で測定。
platform合成は900,006命令、Pipelineは1,400,008 cycles、300,000 stalls、
99,999 flushesで変更前後が一致した。前→後、後→前の順に実行した。

| platform合成 | 変更前 | 変更後 | 時間短縮 |
| --- | ---: | ---: | ---: |
| Pipeline、1組目 | 410.082 ms / 3.414 MHz | 378.282 ms / 3.701 MHz | 7.8% |
| Pipeline、逆順2組目 | 408.667 ms / 3.426 MHz | 378.914 ms / 3.695 MHz | 7.3% |

保存した変更前exeと変更後exeによる実guest比較（Pipeline、process起動込み、
7 samplesの中央値、sampleごとに前後順を反転）:

| guest | 変更前 | 変更後 | 時間短縮 |
| --- | ---: | ---: | ---: |
| 作業ツリーの`dos.wax`、起動5 frames | 113.456 ms | 106.281 ms | 6.3% |
| 現在の`3d_cube.sol`、起動10 frames | 2044.499 ms | 1900.649 ms | 7.0% |

空configのdirectoryで`--allow-running --frames N --skip-render-checks`を使用。
cubeは今回のsourceをsolc既定size最適化→asm `--strip`で生成し、両exeで同じbinaryを実行した。
各sampleでcycles・PC・runningが一致、DOSはR1も一致した。
cubeのhost TRNGを使う値は一致を要求しない。DOSはHALT後のclock進行を含み、
cubeはstartupのみなので、どちらも定常描画やGUIの48 MHz動作を表す値ではない。

48 MHzに対して約7.7%で、目標は未達。合成の改善率をGUI性能へ外挿しない。
RAMのみの構成にはBMCがなく、この高速経路による改善は想定しない。

差分回帰テストは、256要求mask × 調停有効/無効 × 8探索開始位置 ×
8種類のpriority/待機履歴を一般調停と比較する（32,768状態）。待機履歴は
agingの255/256境界、最大昇格、u16飽和を含む。

検証はreleaseでBMC 4件、DMAC 4件、Bus 3件、CPU/選択/Pipeline/relaxed/
memory-map 59件、JIT 13件が通過。変更前後両exeのcube 60-frame実行でも、
GP noise・VPU描画・白いSGC glyphのassertionが通過し、48,000,000 cyclesで
HALTした。これはheadless描画確認で、GUI/GPUの表示や音声試聴は未実施。
変更したRustファイルの`rustfmt --check`と`git diff --check`も通過し、
`cargo build --release --features jit --bin cpt32`で本体を再buildした。

## JITの追加改善（2026-10-18）

待機device batching導入後のJITから、bus-free区間のnative命令を最大16個まとめ、Rust helper／CPU register同期の往復を削減した。cold fetch、code変更、DMA／IDC／拡張カード、IRQ pendingとtimer期限には保守的処理を残す。通常nativeは16 visits、一括用は128 visitsで段階的にcompileし、1命令blockは解釈実行する。Short modeや乗除算待ちのJIT呼出しも省いた。追加の精度緩和は行っていない。

変更前JITからの比較（同一Windowsホスト、release、profilerなし）:

| 対象 | 変更前 | 変更後 | 解釈 |
|---|---:|---:|---|
| 合成ALU、warm 800k cycles | 19.099 ms | 11.877 ms | 約1.61倍 |
| 合成ALU＋MUL＋memory、warm 800k cycles | 37.705 ms | 32.318 ms | 約1.17倍 |
| DOS boot、5 frames | 53.791 ms | 54.442 ms | ほぼ横ばい |
| cube startup、10 frames | 458.432 ms | 434.584 ms | 約5%短縮 |
| cube完了、60 frames | 926.754 ms | 867.300 ms | 約6%短縮 |

合成は9 samples、guestは7 samplesの中央値。guestは保存した変更前exeと変更後exe、cached-relaxedの実行順を巡回し、process起動込み。cubeはhost TRNGのR1以外の実行状態と各方式の描画assertionを確認した。60-frameはHALT後を含む。GUI/GPUは未測定、ホスト負荷による分散も残る。実ゲストのcached-relaxedはDOS 34.085 ms、cube 372.873 / 764.259 msで依然JITより速い。

最終cube smokeのnative 1,795,618命令／fallback 11,136,733 cyclesが示す通り、残る改善対象はShort mode・memory・乗除算のfallbackとdispatch。通常Cachedに対する倍率だけではJIT固有の効果を判断できないため、`cpu_jit_bench`はcached-relaxed比較と、待機platform deviceを付ける`--platform`を追加した。条件・安全な一括実行の根拠・検証結果は[cpu-jit-design.md](cpu-jit-design.md#jit往復削減と段階的compile)を参照。以下はそれ以前の調査記録。

## 今回の計測

Windows の同一ホストで release build を使い、中央値を比較した。数値はこのホストと今回のゲストに限る。

| ケース | Scalar | Pipeline | Cached | Cached の対 Pipeline |
|---|---:|---:|---:|---:|
| CPU 合成ループ、90,006 命令、7 サンプル | 41.780 ms | 30.786 ms | 11.213 ms | 2.75x |
| `dos.wax`、headless 5 フレーム、7 サンプル | 183.7 ms | 123.2 ms | 58.0 ms | 2.12x |

`dos.wax` は作業ツリー直下のゲストを使い、空の一時 working directory から `src32_testbench` を起動した。リポジトリの `config.yaml` がホスト上にないディスクイメージを指していたため、比較時にその設定を読み込ませないようにした。各モードとも同じゲスト、5 フレーム、8e5 cycle/frame、release binary、7 回で計測した。プロセス起動も含む wall time だが、サンプル幅は小さく、モード間の差は安定していた。

同じ 5 フレームを `--profile-cpu` 付きで実行した stage 内訳は次のとおり。計測器の `Instant` 呼び出しが実行を遅くするため、この内訳を wall-time 比較には使わない。

| CPU stage | Pipeline | Cached |
|---|---:|---:|
| Device / IRQ | 21.6% | 42.0% |
| Instruction fetch | 40.6% | 17.6% |
| Execute | 12.0% | 19.3% |
| Memory access | 9.2% | 0.0% |
| Stall / bookkeeping | 5.8% | 21.1% |
| その他 | 10.9% | 0.0% |

割合は測定された CPU stage 時間に対するもの。Cached の Fetch には命令 cache lookup と decode が含まれ、CPU 種別間で同じ処理を比較しているわけではない。`src32_testbench` は VDP tick と CPU を実行するが、GUI 描画、GPU 実行、音声合成は含まない。したがって今回言えるのは CPU/device 経路が大きな候補というところまでで、GUI の遅さの原因を確定したわけではない。

CPU とは独立した `emulator_component_bench` の最新 release 結果（中央値、warm-up 1回後に7 samples）は以下。Bus は400,000 tick、VDP tick は1,000,000回、frame workload は120 frame（VPUのみ10 frame）、audio は800 stereo sample framesで計測した。

| Component workload | Median | 換算 |
|---|---:|---:|
| Bus tick、mapped deviceなし | 5.920 ms / 400,000 | 14.8 ns/tick |
| Bus tick、RAMのみ | 5.894 ms / 400,000 | 14.7 ns/tick |
| Bus tick、RAM + BMC/DMAC | 7.479 ms / 400,000 | 18.7 ns/tick |
| Bus tick、core devices idle | 7.534 ms / 400,000 | 18.8 ns/tick |
| Bus tick、core + PeC input | 14.284 ms / 400,000 | 35.7 ns/tick |
| Bus tick、core + empty IDC | 8.740 ms / 400,000 | 21.9 ns/tick |
| Bus tick、standard devices idle | 12.732 ms / 400,000 | 31.8 ns/tick |
| VDP tick、SGC disabled | 3.223 ms / 1,000,000 | 3.2 ns/tick |
| VDP full-frame composition | 139.320 ms / 120 | 1.161 ms/frame |
| PCG full-frame composition | 185.762 ms / 120 | 1.548 ms/frame |
| Graphics + PCG overlay composition | 214.697 ms / 120 | 1.789 ms/frame |
| SGC screen fill + VDP tick | 0.477 ms / 120 | 4.0 μs/frame |
| SGC render、8 sprites | 154.241 ms / 120 | 1.285 ms/frame |
| SGC render、32 sprites | 203.255 ms / 120 | 1.694 ms/frame |
| VPU flat triangles、1/frame | 3.716 ms / 10 | 0.372 ms/frame |
| VPU flat triangles、8/frame | 29.586 ms / 10 | 2.959 ms/frame |
| SGU inactive mix、800 stereo frames | 11.748 ms / 120 | 0.098 ms/frame |
| SGU noise、1 active channel | 12.760 ms / 120 | 0.106 ms/frame |
| SGU noise、8 active channels | 17.128 ms / 120 | 0.143 ms/frame |

Bus のデバイス構成別の差は各経路を同じ合成テストで比較した値で、デバイスごとのコストを厳密に加算できるとは限らない。PeC inputとSGC 32 spritesは前回測定から変動が大きく、追加サンプルが必要。standard構成はRAM、BMC/DMAC、SGU、VDP、PeC、RNG、未接続IDCを持つが、attached IRQ controller、active disk/DMA、expansion cardはない。実機能の割り込み・転送負荷を含む上限ではない。VDP composition はCPU側で全ソース画素を生成する計測で、Presenterの拡大/余白処理、WGPU upload、GPU execution、VSync待ちは含まない。SGUは1 chipだけでIDC disk audioも含まない。

実ゲストを使った確認では、`dos.sol` をrelease用にビルドしてPipeline CPU上で最大256件のDSFS directoryを `DIR` 表示した。これは80列、空ファイル256件、`-- MORE --` ごとにキー入力して最後まで進める条件。profilerなしでは5.107秒、profiling有効では11.756秒、stage合計8.201秒、18,917,418 tickだった。profile shareはFetch 38.2%、Device/IRQ 24.7%、Execute 9.2%、Memory 9.0%。profilingはこのworkloadのwall timeを約2.30倍にしたため、stage比率だけを使う。時間計測はframe単位の固定cycle実行、ページ送りキー投入、screen buffer確認を含むheadless harness値で、通常の人間の入力間隔やGUI表示時間そのものではない。

`3d_cube.sol` は現在のソースからraw binaryにして、空configのheadless harnessで実行。Pipeline 10 frame（8,000,000 cycles）のunprofiled wall timeは2.114秒。profile有効時は3.300秒で、Fetch 36.8%、Device/IRQ 26.0%、Execute 9.4%、Memory 8.9%。1 frameだけのprofileもstage shareはほぼ同じ（Fetch 36.7%、Device/IRQ 25.6%）で、起動時のnoise setupを含む初期frameから高コスト。profilingは約1.56倍のwall-time増加を招いた。どちらの実ゲスト測定もheadlessであり、WGPU upload/GPU rendering/VSyncや実GUIイベントloopの負荷は計測していない。CPU stage内の時間だけで、noise生成ループと通常のcube描画を完全には分離していない。

## 優先する調査と修正

1. **Bus tick の実機能負荷を測る。** 今回の構成別 bench を基準に、attached IRQ controller、active disk/DMA、expansion card を含めた実構成を追加する。system-cycle advance、IDC/DMAC polling/arbitration、IRQ aggregationのコストは各構成差だけでは分離できないため、必要なら個別区間を計測する。既存の `cycles_until_next_event` と IRQ hot-path 最適化に重複しないよう、着手前に現行コードを確認する。
2. **描画の残る負荷を測る。** SGCの8/32 sprites、PCG overlay、VPUの1/8 triangle workloadを追加済み。次はVPUのtexture/light/depth経路やPCG scrollなど、実ゲストに近い workload を追加する。CPU側ではVPU 8 trianglesが約2.96 ms/frameを占めるが、GPU upload/executionとVSync待ちは計測外なのでGPU timestamp/OS profilerで確認する。
3. **音声を実負荷で測る。** 今回のSGUは1 chipのinactive mixとnoise channelsで、2 chip + IDC disk audioを含まない。実際の複数voiceとdisk soundを再現する workload を追加し、出力の正しさを確認してから評価する。1 active channelの測定差はノイズの範囲として扱い、速いという結論には使わない。
4. **Pipeline fetch を調べる。** CPU stageではInstruction fetchが最大。命令fetchのBus/RAM呼出し、decode値、分岐後の再fetchをサブ計測し、Pipeline fetch/cache構造を候補化する。命令 cacheはRAM書換え、自己書換えコード、page境界、MMIO fetch、DMA/BMC arbitrationの観測差を検証してから採用する。既存Cached経路は性能比較の参考値だが、Pipelineとサイクル/バス副作用が異なるため、既定CPUを単に置き換えない。
5. **変更を一つずつ評価する。** 同じゲスト、同じ release 設定、同じ CPU モード、同じサンプル数で前後を比較する。wall timeとstage shareに加え、guest architecture、cycles、IRQ、DMA、描画結果が一致する回帰確認を行い、実時間比とフレーム落ちで利用者への改善を判定する。

次の計測は、3d_cubeをframeごとに区切ったstartup/steady-state比較、DOSのページ単位のDIR計測、同じROMでのGUI CPU-frame/render-submit時間、IRQ controllerとactive DMA/diskを含むBus構成を優先する。GPU側present経路と2 chip + IDC audioも別途測る。実ROMではFetch/Device/IRQが大きいが、これだけでユーザーが見るUI stallの最終原因とは断定せず、GUI frame profileとGPU側計測を合わせて修正順を決める。
## 実験的cached-relaxedモード

JITも同じ一括処理を自動で使う。profiler無効、JIT feature付きの同一release buildで、process起動込み7-sample medianを再計測した。倍率は通常Cached時間 / 対象モード時間。

| 対象 | Cached | cached-relaxed | JIT | JIT倍率 |
|---|---:|---:|---:|---:|
| DOS起動、5 frames | 59.950 ms | 37.092 ms | 61.639 ms | 0.973x |
| cube起動、10 frames | 978.480 ms | 367.928 ms | 449.574 ms | 2.176x |
| cube描画完了、60 frames | 1671.373 ms | 712.581 ms | 867.408 ms | 1.927x |

この同一buildではJITは通常Cachedよりcubeで速いが、cached-relaxed単体よりcube起動で約22%、60 framesで約22%遅い。DOSではJITが通常Cachedより約3%遅い。したがってJITに一括処理を接続したことと、JITが最速であることは別の結果である。CPU JIT合成ベンチ (`cargo run --release --features jit --bin cpu_jit_bench -- 800000 9`) は、ALU＋branch warmでCached 87.088 ms / JIT 22.456 ms（3.878x）、ALU＋MUL＋memory warmで82.822 / 40.355 ms（2.052x）。cold compileも別に出力する。

2026-10-18。`--cpu cached-relaxed` を追加した。待機中のデバイス更新を最大32サイクル（2026-10-20に256へ拡大）にまとめるCached派生モードで、JITも同じ一括処理を使う。RAMを含むバスアクセス（2026-10-20以降はタイミング依存デバイスへのアクセスのみ）、既知のタイマーIRQ期限、HALT／IRET、`run` 終端で同期する。DMA busy／IDC active／拡張カードありでは従来処理を使う。単命令stepとprofilerは通常のCached処理に戻る。入力IRQの観測遅延と待機中のBMC調停履歴の違いを許容し、デバイスのIRQ記録や転送内容は維持する。

同じWindows x64ホスト、release、profiler無効で比較した。倍率はCached時間÷relaxed時間。

| 対象 | Cached | cached-relaxed | 倍率 |
|---|---:|---:|---:|
| 合成ALU＋branch、800,000 cycles | 80.902 ms | 16.595 ms | 4.875x |
| 合成ALU＋RAM、800,000 cycles | 128.049 ms | 75.737 ms | 1.691x |
| 作業ツリーのdos.wax、起動5 frames | 54.624 ms | 33.532 ms | 1.629x |
| 現在の3d_cube.sol、起動10 frames | 982.097 ms | 365.282 ms | 2.689x |
| 同cube、60 frames、描画完了＋HALT後の進行 | 1661.848 ms | 687.675 ms | 2.417x |

合成比較は `cargo run --release --bin cpu_relaxed_bench -- 800000 9`、9 samplesの中央値。CPU作成・program load・1,024 cyclesのcache warmupは計測外。BMC／DMAC／PeC入力・timer／空IDCを接続し、active DMA／disk／expansionはない。実行順を交互にして同じcycle予算でregisters、PC、retired instructions、RAM結果を照合する。メモリアクセスでも同期するため、ALUだけのケースほどの改善は期待しない。

実ゲストは `src32_testbench` のprocess起動込み、7 samplesの中央値、空configのdirectoryから起動し、1 frameは800,000 cycles。各sampleで実行順を交互にした。cubeは現在のsourceをsolc→asm（`--strip`）でraw binaryにした。DOS／cubeの起動途中には `--allow-running --frames N --skip-render-checks` を使い、PC・cycles・停止状態を照合した。DOSのR1も一致する。cubeはhost TRNGからnoise seedを取るため、R1の一致とbit単位の描画一致は要求していない。

60-frame cubeは `--allow-running --frames 60 --expect-sgc-overlay --expect-gp-noise-background` でCached・cached-relaxed・JITの3方式がGP noise・VPU cube・白いSGC glyphの描画assertionを通過し、実行停止状態も一致した。これは初期化とHALT後のclock進行を含む測定であり、定常アニメーションのframe latencyではない。GUI／GPU／音声の実時間性能や入力体感は未測定。active disk／DMAの改善率もここからは推定しない。

回帰確認は専用CPUテスト（予算途中、乗算、MMIO時刻、timer IRQ、HALT／IRET、DMAによるcode変更とIRQ、profile／step／reset）、Bus batchingテスト、DOS HEX→Q→HELP→EXIT（800 cyclesと通常予算）、実WASMカードのactive loop／HALTからのIRQ復帰を含む。HALT後に追加でIRQを受理する経路ではCPU MMIOのPC/statusも同期しないと、次cycleでhandler位置を古い値に上書きするので注意する。

## Bus／デバイスのサイクル毎オーバーヘッド削減（2026-10-20）

全CPU方式に共通する `Bus::tick_devices` とバイト単位バスアクセスの固定費を削った。ゲストから見える意味（IRQ edge latch、BMC状態、byte調停、DMA、bus error、タイマー値）は変えていない。

一時的なマイクロベンチ（idle platform＋IRQC、release）で測った内訳と結果:

| 経路 | 変更前 | 変更後 |
|---|---:|---:|
| `refresh_device_irqs` 1回 | 28–30 ns | 3.3 ns |
| `tick_devices` 1回 | 80–85 ns | 14 ns |
| RAM `read_u8` | 44 ns | 12 ns |
| RAM `read_u32_be`（命令fetch） | 180 ns | 18 ns |

主因は `refresh_device_irqs` が12個の `IrqController::set_source` を個別に呼んでいたことで、これがtick毎に2回、バスアクセス毎に1回（32-bit fetchで4回）走っていた。

- `IrqController::set_source_levels(mask, levels)` で1回のbit演算にまとめた。レベル変化がある時だけ従来の `set_source` を番号順に呼ぶので、edge latchとPeC traceは同一。
- PeC入力の保留判定は `PeCState` が `event_len != 0` を `Arc<AtomicBool>` に反映し、Busはそれを読む（Mutexを取らない）。ホスト入力の観測タイミングは元々非決定的。
- `Device::is_plain_memory()`（RAM／BIOS ROM）はアクセス後のIRQ再サンプルを省く。32-bitアクセスが単一plain memory内に収まり、DMACの要求が無いときは調停を1回にして4 byteを読み書きする（beat間にDMA要求は発生し得ず、BMC最終状態は同じ）。DMA要求がある・境界を跨ぐ・debugger経路では従来の4回byteアクセス。
- `arbitrate_cpu_access` はDMAC要求0のとき `grant(1)` → `claim_cpu()` のみ（grant→release→claimと同じ状態）。
- `tick_devices` はbusyなDMA channelもactiveなIDCコマンドも無いとき、IDC clockとBMC要求latch（`grant(0)`）だけを進める。
- `advance_system_cycles(1)` はdeadline探索を省く（全deadlineは `now + 1` 以上）。
- PeC timer prescalerとRTC（既定で有効）は桁あふれしない限りu64で計算し、u128除算を避ける。

`cargo run --release --bin cpu_bench -- 100000 7 --platform` の中央値: Scalar 544.5→117.6 ms、Pipeline 359.6→76.8 ms、Cached 146.0→42.1 ms。`src32_testbench test.wax --frames 60`（空config、5回の最良値、process起動込み）: Pipeline 3042→769 ms、Cached 1349→465 ms。`dos.wax` Pipeline 143→64 ms。Pipeline／Cachedは20 framesの最終PC・cycles・R1が変更前と一致。Scalarの `test.wax` R1は変更前同士でもhost TRNGで揺れる。GUI／GPUの実時間は未測定。

## I/Oバウンド処理: 待機中デバイスとスピンループ（2026-10-20）

IDCを使うゲストはsol／Cの両ドライバとも転送完了をMMIOポーリングで待つ。FDD既定値では64 KiBの読み込みが約1.15秒（約55M cycles）続き、その間はDMA／IDCがactiveなので待機device一括処理もHALT skipも働かず、全CPU方式が10〜17 MHz（実時間の3〜5倍遅い）だった。CachedやJITもScalarより遅く、ポーリングのMMIO読み出し（4 byte×調停・device探索・IRQ再確認）が主なコストだった。

1. **待機中のIDC/DMAの高速tick（全方式、厳密）**: `Idc::cycles_without_event` が、次のtickがcycle counterの加算以外に何も起こさないことを判定する（sector deadline前、abort要求なし、DMA進行待ちはDMAC側で判定）。DMACの要求maskが0なら `tick_devices` はIDC clockとBMCの`grant(0)`だけを進める。
2. **副作用のないレジスターの32-bit読み出し（全方式、厳密）**: `Device::peek_u32_be`（DMAC全レジスター、DATA以外のIDCレジスター）を持つレジスターは、DMA要求がなければ調停1回で読む。`find_device` は直前のdeviceを覚える。
3. **スピンループの早送り（全方式、厳密）**: 仕様は`doc/spec_CPU.md` §1.4。Busは副作用のあるアクセス数（`effectful_accesses`）と静穏区間（`quiet_cycles`）を提供し、CPUは後方ジャンプ先ごとに最大8個のprobeでループ先頭の状態を比較する。1回一致したら静穏区間を測り、次の1反復がその区間内で同じ状態へ戻れば、区間かrun予算の終わりまで反復をまとめて飛ばす（`advance_quiet_cycles`）。

計測（Windows x64 release、`cargo run --release --features jit --bin idc_bench`、3 samples中央値。FDD 32 sectors／HDD 2048 sectorsのpolled DMA READ）:

| ケース | 開始時 | 1+2のみ | 最終 |
|---|---:|---:|---:|
| FDD Pipeline | 1466.9 ms | 1065.7 ms | 6.4 ms |
| FDD Scalar | 1002.4 ms | 627.4 ms | 3.3 ms |
| FDD Cached | 1509.4 ms | 772.9 ms | 4.1 ms |
| FDD JIT | 1748.3 ms | 1003.4 ms | 5.1 ms |
| HDD Pipeline | 1146.6 ms | 881.4 ms | 410.9 ms |
| HDD Scalar | 700.8 ms | 501.6 ms | 205.0 ms |
| HDD Cached | 1005.7 ms | 598.4 ms | 265.5 ms |

FDDは98.6%のcycleを飛ばせる。HDDは転送速度が速く、残る約2.8M cyclesは毎cycle DMA beatが実際に動く区間なので、ここは実作業として残る。

実ゲスト（`sol/cpt32/idc.sol`の`idc_read`で64 KiBをFDDから16 sectorずつ読み、全byteを合計する。`src32_testbench --frames 150`、process起動込み、3回の最良値。R1=8,355,904は全方式・変更前後で一致）: Pipeline 7408→747 ms、Scalar 4833→911 ms、Cached 5067→334 ms、cached-relaxed 5138→248 ms、JIT 261 ms。CPU-boundな`cpu_bench`／`cpu_jit_bench`は変化なし。

## タイミング非依存MMIOデバイスのフラグ（2026-10-20）

`Device::is_timing_insensitive()`（仕様は`doc/spec_CPU.md` §1.5）を追加し、RAM／ROM、VDP（VRAM・レジスター・SGC）、VPU、SGU（レジスター・PCM RAM）、RNG、UARTに付けた。フラグはデバイス登録時にBusの`DeviceMap`へ記録し、アクセスごとの動的呼び出しを避ける。用途は、relaxed／JITでのflush省略（RAMを含む）、全方式でのIRQ再確認省略と32-bitアクセスの調停1回化、JIT batch内でのnative実行。

relaxed区間がRAMアクセスで切れなくなった結果、JIT batchは区間を使い切りやすくなった。区間切れでbatchを辞退するとstrict tierの遅延compileを誘発してcompile時間が増えたため、非連結のbatch開始時に区間を使い切っていれば、その命令境界でflushして新しい区間を開く。連結中に切れた場合はchainを終えるだけで辞退として数えない。

計測（Windows x64 release、`cargo run --release --features jit --bin mmio_bench -- 100000 5`、VRAM 32-bit store×2、VDP register byte store、RNG word load、ALUの100,000反復。変更前binaryと交互に3回、中央値）:

| CPU | 変更前 | 変更後 |
|---|---:|---:|
| Pipeline | 97 ms | 86 ms |
| Scalar | 159 ms | 140 ms |
| Cached | 61 ms | 49 ms |
| cached-relaxed | 50 ms | 34 ms |
| JIT | 61 ms | 16 ms |

結果のchecksumと実行命令数は変更前と一致。Busの単体計測ではVRAM 32-bit書き込みが65.9→20.9 ns、RAM 32-bit読み書きが17.0→16.0 ns／19.8→16.8 ns。`test.wax`（cube、20 frames、7回の最良値）はcached-relaxed 318→295 ms、JIT 222→199 msで、JITのbatch tier compile数は80→202に増えた（区間端でのentryが増えるため）。CPU-boundな`cpu_bench --platform`はScalar／Pipelineで約2〜5%遅く見えるが、同一sourceの再build間でも最大5%程度揺れ、使っているBus primitiveは単体で同等以上に速いため、主にcode layoutの差と判断した（未解決の可能性として記録）。
