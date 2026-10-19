# CPT32 guest C libraries

[`sol/cpt32`](../../sol/cpt32/) の実装済み機能を、[`tools/llvm-src32`](../../tools/llvm-src32/README.md) 用の freestanding C API へ移植しています。ホスト用ライブラリやWASMカードSDKではありません。solの関数ABIとはバイナリ互換ではありません。

## モジュール

| ヘッダー (`<cpt32/...>`) | API | 契約 |
|---|---|---|
| `vpu.h` | `vpu_*` | FIFO待機、行列・照明、テクスチャ・モデル、三角形描画。binary32は`uint32_t`ビット列 |
| `pcg.h` | `pcg_*` | 40/80列と80×30の8×16フォント表示、専用フォント領域定数、文字・属性、物理ring rowを使うスクロールとカーソル |
| `print.h` | `print_*` | UART文字列出力。改行はCR/LF |
| `pec_rng.h` | `pec_rng_*` | TRNGでPRNGをseedし、初回読出しを破棄 |
| `colors.h` | `colors_tocolor` | 元ソースどおり`b + (g << 2) + (r << 4)`。RGB332への仕様変更ではない |
| `irqc.h` | `irqc_*` | 割り込み登録、マスク、ack、IRQ番号、VSyncカウンタ・待機 |
| `cpu.h` | `cpt32_halt` | SRC32の`HALT`を出力し、有効なIRQ受理後に再開するLLVM/SRC32専用intrinsic |
| `idc.h` | `idc_*` | IDC/DMAC ch0の同期sector I/O。戻り値0=成功、1=DMA失敗、2=IDC失敗 |
| `dsfs.h` | `dsfs_*` | 単一mount状態、ディレクトリ・bitmap、ファイル読書き・削除、timestamp |
| `wax.h` | `wax_*` | WAX v2検証、予約RAMへのstaging、コード・stackを置換できるロード |
| `sgub.h` | `sgub_*` | SGUB v1/v2/v3全体検証、初期化、フレーム再生、slot 0カード出力、repeat・終端停止 |

元の`colors.sol`の`hsv2color`は空の関数なのでAPIに含めません。`sol/solstd`のsoftfloat等はこのSDKの対象外です。述語は通常のC規約 (`true != 0`)。`dsfs_name_eq`と`wax_ult`もCの`bool`を返します。エラーコードは各ヘッダーを参照してください。

## CMakeで使用

Clang 20.1.8、Python 3.15 + llvmlite 0.50、Ninjaで検証しています。必要ツールの導入は[ツールチェーンREADME](../../tools/llvm-src32/README.md)を参照してください。

```cmake
cmake_minimum_required(VERSION 3.20)
project(my_guest LANGUAGES NONE)
add_subdirectory("/path/to/src32/sdk/cpt32" cpt32)
src32_add_executable(my_guest
    SOURCES main.c
    LIBRARIES cpt32
    OPTIMIZATION 0)
```

`cpt32`は全モジュール、`cpt32_display`、`cpt32_irqc`、`cpt32_storage`、`cpt32_music`は個別グループです。`cpt32_storage`は安全なWAXロードのためIRQライブラリにも依存します。各ターゲットは既存の`src32_add_library`によるソースライブラリです。

`OPTIMIZATION 0` / `1` / `2`を使用できます。最適化時の内部関数の`fastcc`も、SRC32のスタック渡しへ変換します。標準のホストコンパイラでMMIOプログラムを実行しないでください。

## IRQとVSync

```c
#include <cpt32/irqc.h>
int main(void) {
    irqc_init();
    irqc_enable(1);  /* IRQ0 = host VSync */
    for (;;) {
        irqc_wait_vsync();
        /* frame update */
    }
}
```

- LLVM/SRC32バックエンドは`void irqc_dispatch(void)`がリンクされている場合、`main`より前に全16ベクタへ`JR R30`を設定します。R30はdispatcher入口用に予約されます。
- IRQ入口は128-byteの8-byte整列フレームへSP以外の全汎用レジスタを保存し、C dispatcherを呼び、復元して`IRET`します。SPはフレーム解放で元へ戻ります。
- Dispatcherは有効な保留IRQを優先度順に全件処理します。登録済みcallbackの後にpending bitをackするため、callbackではdevice固有の割り込み条件を先に解除してください。callback未登録IRQはdispatcherがackします。下位優先度のIRQもIRET前に処理し、CPUのエッジ検出が再度立ち上がらない状態を防ぎます。
- callbackで割り込みのネストを有効にしないでください。callbackは引数なし・戻り値なしの通常のC関数です。SGUBの再生はcallbackから`sgub_step()`を呼ぶか、VSync待機後に呼びます。
- `irqc_wait_vsync()`は次のIRQ0をカウンタで待ちます。CPUは実行を続けます。CPUサイクル由来の独自VSyncや、ハンドラ未登録のIRQ有効化は使いません。


## CPU HALT

`cpt32_halt()`はSRC32の`HALT`命令を出力するLLVM/SRC32専用intrinsicです。有効な割り込みが受理されると呼出し直後から再開します。CPUを起こす有効な割り込みsourceがある状態でのみ使ってください。ホスト向けsleep関数ではありません。

## バッファと状態

- MMIOはvolatileアクセス。CPUのword/halfwordとファイル整数はbig-endian。PCG scroll MMIOのbyte laneはlow byteからです。
- `pcg_cls(color)`は`pcg_vram_base`から文字面4 KiBを0、前景・背景属性面各4 KiBを指定色で埋め、スクロールで隠れた行も消去します。glyph 0が空でなくても指定色で塗りつぶされます。カーソル位置・スクロール量と、以後の出力用の`pcg_fg_color`/`pcg_bg_color`は保持します。
- VPUの配列長・テクスチャサイズなどの前提条件は元のsol APIと同じです。APIコメントを確認してください。
- DSFSは単一mountです。ディレクトリbufferは8192 byte、bitmapは`dsfs_bitmap_blocks()*512` byte、ファイルbufferはsector切り上げサイズが必要です。書込みは入力bufferの最終sector末尾をゼロ埋めします。名前はドットなし・space-paddingの大文字11 byteです。
- `idc_identify(slot, &capacity)`は成功時0、失敗時1を返し、全範囲の`uint32_t`セクタ数を出力引数へ格納します。失敗時は出力を変更しません。
- DSFS mountは元のsol版同様superblockのmagic/version/layoutを検査します。完全な不正媒体検査ではありません。未信頼画像は先に`tools/dsfs/dsfs.py verify`で検査してください。更新順はbitmap占有→payload→entry更新→旧領域解放、削除はentry解除→bitmap解放です。
- WAX loaderは元の16 MiB RAMゲストの予約域を維持します。`0x00FE0000..0x00FFFFFF`をstaging/span map/trampolineに使用するため、16 MiB以上のRAMが必要です。最大ファイル64 KiB、予約域へのloadは拒否。ロード時に全IRQC sourceを無効化・ackします。成功したプログラム切替は呼出元に戻りません。
- SGUBはcaller所有の不変streamと正確なbyte数を受け取り、全ヘッダー・pool・操作・frameを検証してから初期MMIOを書きます。`sgub_init`後は`sgub_step`で1フレームずつ適用します。repeatで初期化列を再実行せず、非repeat終端の次stepで全voiceを停止します。
- 各モジュールのsingleton状態は再入可能ではありません。同じdeviceをmainとIRQから同時更新しないでください。

## 検証

リポジトリルートで、依存パッケージとClangを用意して実行:

```sh
python sdk/cpt32/smoke/run.py --clang "C:/Program Files/LLVM/bin/clang.exe"
```

CMake buildとnative `src32_testbench`を使い、Scalar/Pipelineの5ゲストを実行します。すべてR1=42を検査します。PCGのring row・cursor、40列/80列/8×16のCLSによる文字・属性の全消去、IRQ0連続callbackと中断された計算の保持、SGUB v1/v2・不正streamの書込み禁止、DSFSの成長/縮小/削除、WAXが実行中のコードとstackを上書きして起動する経路を含みます。
`--optimization 1`または`--optimization 2`で最適化した全ライブラリとゲストを検証できます（既定は`0`）。

`--cpu cached --cpu cached-relaxed`を追加するとCached系で実行できます。この2方式でも全5ゲストとDSFSのリーク0検査を通しています。

各実行はBIOSなしの一時directoryを使い、storage用64-sector画像は毎回新規作成します。ホストのディスク画像を変更しません。更新後にhost DSFS validatorでリーク0を確認します。

描画デモは[`examples/3d-cube-c`](../../examples/3d-cube-c/README.md)。同じライブラリを使い、120フレームのcube/overlay/背景検査をScalar/Pipelineで通しています。
