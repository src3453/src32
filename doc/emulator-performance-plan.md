# エミュレータ性能調査と改善計画

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
