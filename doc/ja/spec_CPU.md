---
lang: ja
translation_of: ../spec_CPU.md
---

# SRC32仕様

Revision 2.7 (2026-10-18)

# 変更履歴
- 2.7 (2026-10-18): 待機中のデバイス処理をまとめる実験的な任意選択モード `cached-relaxed` を追加しました。ISAは変更しません。
- 2.6 (2026-10-18): Cachedの命令意味・タイミングを基準にした、任意選択の実験的Cranelift JITを追加しました。ISAは変更しません。
- 2.5 (2026-10-18): 書き込み整合性を持つ命令デコードキャッシュを使うCached Interpreterモードを追加しました。ISAは変更しません。
- 2.4 (2026-10-15): 拡張p（小文字、パイプライン処理）をマイクロアーキテクチャ拡張として追加。[[doc/ja/spec_CPU_p|拡張p仕様]]を参照。ISAの変更はありません。パイプラインCPUは`CPU_FEATURES` bit 7（`0xBF`）を報告し、scalarおよびcached interpreterは`0x3F`を報告します。
- 2.3 (2026-09-17): CPUID、割り込みベクター、実装済み拡張、CPU予約MMIOをRust CPUに合わせた。
- 2.2 (2026-09-17): 将来の拡張として拡張Fの定義を追加。現行CPUはFを実装していない。
- 2.1 (2026-08-09): 拡張S（短縮命令形式）を追加。
- 2.0 (2026-08-08): 命令セットとエンコーディングを更新する大幅改訂。
- 1.1 (2026-07-02): 拡張M（乗算・除算）命令、および拡張Aの`SLTU`命令を追加。
- 1.0 (2026-06-20): 初版。

## 1. 概要

SRC32（**S**calable **32**-bit **R**ISC **C**PU）は32ビットの教育用RISC CPUです。
Normal Modeでは固定長32ビット命令を使用し、拡張Sは16ビット命令のShort Modeを追加します。
このリポジトリには次が含まれます。

- Rust製CPUエミュレーター（`src/main.rs`）
- Python製アセンブラー（`asm.py`）

マイクロアーキテクチャの複雑さよりも、明快さと実験を重視します。

## 1.1 CPUマイクロアーキテクチャ

`--cpu scalar|cached|pipeline`で実行方式を選択できます。`config.yaml`の`cpu.microarchitecture`でも指定でき、既定はpipelineです。Cached Interpreterは、命令世代を公開するデバイスに対して1024エントリーの直接マップ型デコードキャッシュを使います。RAMの4 KiBページ内のbyteが変更されると世代が変わり、そのページ内のキャッシュ命令は再取得・再デコードされます。CPU store、デバッガー書き込み、DMA書き込みがこの無効化を行います。キャッシュヒットでは命令フェッチのバスアクセスを省くため、Scalar/Pipelineとバス調停タイミングが異なる場合があります。キャッシュ非対応デバイスは通常のfetch/decodeを続けます。Cachedは通常命令を1サイクル、MUL/MULHを3サイクル、DIV/DIVU/MODを16サイクルでcommitし、pipeline由来のhazardや分岐flush遅延はありません。

### 実験的cached-relaxedタイミング

`--cpu cached-relaxed` またはYAMLの `cpu.microarchitecture: cached-relaxed` で選択します。JIT featureは不要です。命令とCPUID（`0x3F`）はCachedと同じですが、待機中のデバイス更新を最大32 CPUサイクル単位にまとめます。RAM・cold fetchを含むバス読み書き、タイマーIRQ期限、HALT／IRET、`run` の予算終端で未処理サイクルを同期します。CPUのサイクル予算と乗除算の遅延は維持します。busyなDMAチャネル（全体disable中を含む）、動作中のIDCコマンド、拡張カードがある場合は毎サイクル処理に戻します。待機中のIDC内部時刻も経過分だけ進め、動作中のIDC更新やDMA調停をシステム時刻の加算だけで代用しません。

このモードはホスト入力IRQの観測遅延と、待機中のバス調停履歴の変化を許容します。IRQ pendingの記録・ACK・入力レベル遷移と入力キューは維持し、既知のタイマー期限ではバッチを分割します。VSync／IRQ0は引き続きホストのフレームループから発生します。profilerと直接tick／デバッグの単命令実行は通常の毎サイクル処理を使うため、profilerでは一括処理の速度を測れません。既定はPipelineで、JITとは別の選択肢です。

## 1.2 実験的Cranelift JIT

`--features jit`でビルドすると、`--cpu jit`またはYAMLの`cpu.microarchitecture: jit`を選択できます。featureなしのbuildではエラーになります。既定は引き続きPipelineです。CPUIDのfeaturesは`0x3F`で、命令・IRQ・バス・cycle budgetの意味はCachedに合わせます。乗算3サイクル、除算16サイクルもfallbackで維持します。

同じ開始PCが16回dispatchされると、Normal modeの整数ALU、half immediate、CPUID、通常の分岐・call・returnを最大16命令のnative blockにします。memory、乗除算、Short mode、mode切替、HALT、IRET、先読み非対応の命令領域はCached経路を使います。先読みは副作用なしpreviewと世代追跡を公開したmemoryだけに限定します。

各命令のdevice/DMA更新後にCached fetchを実行し、世代検証済みdecode値と生成時の命令が一致する場合だけnative演算を実行します。不一致なら実際の命令をそのサイクルにCachedで実行してblockを無効化します。cold fetchのバス副作用、CPU store・DMA・debugger・program loadによる変更、page境界やblock途中の変更をこの経路で扱います。

命令境界のIRQ、CPU register MMIO同期、ホストframe loopからのVSync、`run`のcycle予算を維持します。単一stepと`--profile-cpu`はCachedを使い、後者はfallback側の処理を計測します。compile失敗もログと統計を残してCachedへ戻します。dispatch entryと累積native定義数の上限は各1024です。まず未compileのcold entryを破棄し、それでも満杯の場合やnative定義数が上限に達した場合は、native実行終了後にmodule全体を解放・再作成します。CPU resetでもnative cacheを解放します。

`jit_stats()`はcompile時間・回数、native命令数、fallback cycle、無効化、cache reset、active block数、native code bytesを返します。code bytesはallocatorのpadding・metadataを含みません。統計はCPU resetをまたいで累積し、active blockとcode bytesは現在のcacheを表します。Cachedより高速になる保証はありません。設計・検証条件は`doc/cpu-jit-design.md`を参照してください。

## 2. レジスター

- 汎用レジスター: `R0`～`R31`（32本、各32ビット）
- `R0`はゼロに固定（読み出しは`0`、書き込みは無視）
- 慣例上の別名:
  - `R28`: `SP`
  - `R29`: `FP`
  - `R30`: `GP`
  - `R31`: `LR`
- プログラムカウンター: `PC`（32ビットのバイトアドレス）

## 3. 命令エンコーディング

すべての命令は4バイト（32ビット）で、ビット配置はビッグエンディアンです。

### 3.1 標準形式

- レジスターモード:
  - `[op:6][rd:5][rs1:5][rs2:5][unused:11]`
- 即値モード:
  - `[op:6][rd:5][rs1:5][imm16:16]`
- メモリーモード:
  - `[op:6][rd:5][base:5][off16:16]`

ビット位置:

- `op`: bits `31..26`
- `rd`: bits `25..21`
- `rs1/base`: bits `20..16`
- `rs2`: bits `15..11`（レジスターモードのみ）
- `imm16/off16`: bits `15..0`（即値／メモリーモード）

### 3.2 拡張形式
- 現在、拡張形式が必要なのは次の場合です。
  - 浮動小数点命令（拡張F）
  - その他の将来の拡張
- EXTR/EXTI/EXTMはサブオペコード（`op2`）を使用し、`op`値は`0x39`～`0x3B`です。

- EXTR:
  - `[0x39][op2:6][rd:5][rs1:5][rs2:5][rs3:5]`
- EXTI:
  - `[0x3A][op2:6][rd:5][rs1:5][imm16:16]`
- EXTM:
  - `[0x3B][op2:6][rd:5][base:5][off16:16]`

## 4. ISA

### 4.1 ベースISA

- `0x00 (reg)`: `NOP`: 何もしない
- `0x01 (mem)`: `LD rd, [base + off16]`: `base + off16`のメモリーから32ビットワードを読み込み、`rd`に格納
- `0x02 (mem)`: `ST [base + off16], rd`: `rd`の値を`base + off16`のメモリーに格納
- `0x03 (reg)`: `ADD rd, rs1, rs2`: `rs1`と`rs2`を加算し、結果を`rd`に格納
- `0x04 (imm)`: `ADDI rd, rs1, imm16`: `rs1`と符号拡張した`imm16`を加算し、結果を`rd`に格納
- `0x05 (reg)`: `SUB rd, rs1, rs2`: `rs1`から`rs2`を減算し、結果を`rd`に格納
- `0x06 (reg)`: `SLT rd, rs1, rs2`（符号付き）: `rs1 < rs2`なら`rd = 1`、それ以外は`rd = 0`
- `0x07 (imm)`: `BEQ rs1, rs2, off16`: `rs1 == rs2`なら`PC + 4 + sign_extend(off16)`へ分岐
- `0x08 (imm)`: `BNE rs1, rs2, off16`: `rs1 != rs2`なら`PC + 4 + sign_extend(off16)`へ分岐
- `0x09 (imm)`: `JMP off16`: `PC + 4 + sign_extend(off16)`へジャンプ
- `0x0A (imm)`: `JAL off16`: ジャンプしてリンク。`R31 <- PC + 4`、続いて`PC <- PC + 4 + sign_extend(off16)`
- `0x0B (reg)`: `JR rd`: `rd`内のアドレスへジャンプ。`PC <- R[rd]`
- `0x21 (reg)`: `JALR rd`: レジスター間接ジャンプしてリンク。`R31 <- PC + 4`、続いて`PC <- R[rd]`
- `0x39 (ext)`: `EXTR`エンコーディング用に予約（拡張レジスターモード）
- `0x3A (ext)`: `EXTI`エンコーディング用に予約（拡張即値モード）
- `0x3B (ext)`: `EXTM`エンコーディング用に予約（拡張メモリーモード）
- `0x3C (imm)`: `LDIL rd, imm16`: 下位16ビットを`rd`に読み込み、上位16ビットを保持
- `0x3D (imm)`: `LDIH rd, imm16`: 上位16ビットを`rd`に読み込み、下位16ビットを保持
- `0x3E (reg)`: `CPUID`: CPU ID／機能を固定レジスターに書き込む:
  - `R1 <- CPU_ID`
  - `R2 <- CPU_FEATURES`
- `0x3F (reg)`: `HALT`: 有効なIRQが受理されるまで命令実行を停止。`IRET`で`HALT`の次から再開

`BEQ`/`BNE`に関する注記:

- エンコーディングでは、即値モードの`rd`フィールドに`rs2`を格納します。

`CPUID`に関する注記:
- `CPU_ID`はCPUモデルを識別する固定32ビット値です。
  - `CPU_ID = 0x53524332`（ASCIIの"SRC2"）
- `CPU_FEATURES`は対応拡張を示す固定32ビットビットマスクです。
  - Bit 0: ベースISA
  - Bit 1: 拡張A（ALU）
  - Bit 2: 拡張L（Load/Store）
  - Bit 3: 拡張M（乗算／除算）
  - Bit 4: 拡張S（Short Mode）
  - Bit 5: 拡張I（割り込み）
  - Bit 6: 拡張F（未実装であり、このCPUからは報告されない）

現行CPUが実装するのはベースISAと拡張A、L、M、S、Iのみです。`CPU_FEATURES = 0x0000003F`を報告します。拡張Fは将来の命令セット定義であり、現行CPUではデコードも実行もされません。

### 4.6 拡張I（割り込み）

CPUは独立した`IRQC`デバイスから外部割り込み入力を1つ受け取ります。

- `irq_valid`: 割り込み要求レベル
- `irq_number[3:0]`: 選択されたソース番号

CPU入力は立ち上がりエッジでトリガーされます。LowからHighへの遷移で要求をラッチし、CPUは命令完了後にのみサンプリングします。受理時:

```
EPC = PC                 # 完了した命令の次の命令
CAUSE = irq_number
IRQ_ENABLE = 0
PC = 0xFFFF0100 + irq_number * 4
```

`IRET`は割り込み開始時に有効だった命令モードを復元し、`PC = EPC`に設定して割り込みを再有効化します。
このリビジョンではネストした割り込みは無効です。ソースの調停はCPUの機能ではありません。外部の`IRQC`が、有効な保留IRQのうち番号が最小のものを選択して`irq_valid`/`irq_number`を駆動します。

IRQCのレジスター、edge latch、pending ACK、source配線については[[doc/spec_IRQC|IRQC仕様]]を参照してください。

### 4.2 拡張A（ALU）

- `0x0C (reg)`: `AND rd, rs1, rs2`: `rs1`と`rs2`のビット単位ANDを計算し、結果を`rd`に格納
- `0x0D (reg)`: `OR rd, rs1, rs2`: `rs1`と`rs2`のビット単位ORを計算し、結果を`rd`に格納
- `0x0E (reg)`: `XOR rd, rs1, rs2`: `rs1`と`rs2`のビット単位XORを計算し、結果を`rd`に格納
- `0x0F (reg)`: `SLL rd, rs1, rs2`（論理左）: `rs1`を`rs2`ビット左シフトし、結果を`rd`に格納
- `0x10 (reg)`: `SRL rd, rs1, rs2`（論理右）: `rs1`を`rs2`ビット右シフトし、結果を`rd`に格納
- `0x11 (reg)`: `SLA rd, rs1, rs2`: `SLL`（論理左シフト）と同じ動作。シフト量は`rs2`の下位5ビット
- `0x12 (reg)`: `SRA rd, rs1, rs2`（算術右）: 符号を保持して`rs1`を`rs2`ビット右シフトし、結果を`rd`に格納
- `0x17 (reg)`: `SLTU rd, rs1, rs2`（符号なし）: 符号なし比較で`rs1 < rs2`なら`rd = 1`、それ以外は`rd = 0`（注: Revision 1.1で追加）

### 4.3 拡張L（拡張Load/Store）
- `0x13 (mem)`: `LDB rd, [base + off16]`（byte読み込み）: `base + off16`のメモリーから最下位byteを読み込み、ゼロ拡張して`rd`に格納
- `0x14 (mem)`: `LDH rd, [base + off16]`（halfword読み込み）: `base + off16`のメモリーから最下位halfwordを読み込み、ゼロ拡張して`rd`に格納
- `0x15 (mem)`: `STB [base + off16], rd`（byte格納）: `rd`の最下位byteを`base + off16`のメモリーに格納
- `0x16 (mem)`: `STH [base + off16], rd`（halfword格納）: `rd`の最下位halfwordを`base + off16`のメモリーに格納

### 4.4 拡張M（乗算と除算）（注: Revision 1.1で追加）
- `0x18 (reg)`: `MUL rd, rs1, rs2`: `rs1`と`rs2`を乗算し、下位32ビットを`rd`に格納
- `0x19 (reg)`: `DIV rd, rs1, rs2`: `rs1`を`rs2`で除算し、商を`rd`に格納。`rs2`がゼロなら`0`を格納し、`INT_MIN / -1`は`INT_MIN`を返す
- `0x1A (reg)`: `MOD rd, rs1, rs2`: `rs1`を`rs2`で除算し、剰余を`rd`に格納。`rs2`がゼロなら`0`を格納し、`INT_MIN % -1`は`0`を返す
- `0x1B (reg)`: `MULH rd, rs1, rs2`: `rs1`と`rs2`を乗算し、上位32ビットを`rd`に格納
- `0x1C (reg)`: `DIVU rd, rs1, rs2`: `rs1`を`rs2`で符号なし除算し、商を`rd`に格納。`rs2`がゼロなら`rd`に`0`を格納

### 4.5.1 拡張S（Short Mode命令形式）（注: Revision 2.1で追加）
拡張Sは、一般的な操作向けにコンパクトな16ビット命令エンコーディングの「Short Mode」を提供します。
これらの命令は、特にタイトなループや頻繁に実行されるコードのサイズ削減を目的として、主にコンパイラーが生成することを想定しています。
アセンブラー構文では、すべてのShort Mode命令に`S.`を前置します。
Short Modeに入るため、次の3つの新しいオペコードを追加します。
- `0x1D (imm)` : `JMPS off16`: ジャンプしてShort Modeに入る
- `0x1E (imm)` : `JALS off16`: ジャンプしてリンクし、Short Modeに入る
- `0x1F (reg)` : `JRS rd`: レジスタージャンプしてShort Modeに入る
- `0x22 (reg)` : `JALRS rd`: レジスタージャンプしてリンクし、Short Modeに入る

### 4.5.2 Short Mode命令
CPUに現在の命令モードを示す読み取り専用レジスター`INSTR_MODE`を追加します。
- `INSTR_MODE = 0`: Normal Mode（32ビット命令）
- `INSTR_MODE = 1`: Short Mode（16ビット命令）
Short Modeには2つのアドレッシングモードがあり、16ビット命令形式を使用します。
- レジスターモード: `[op:4][rd:4][rs1:4][rs2:4]`
- 即値モード: `[op:4][rd:4][imm8:8]`
- 12ビット即値モード: `[op:4][imm12:12]`
- `op`: bits `15..12`
- `rd`: bits `11..8`
- `rs1`: bits `7..4`
- `rs2`: bits `3..0`
各Short Mode命令の後、PCは自動的に2増加し、次の命令は次の16ビット境界アドレスからフェッチされます。
（`PC = PC + 2`）
レジスター0～14は`R0-R14`に対応し、レジスター15は`R31`（LR）に対応します。

- 対応するShort Mode命令:
- `0x0 (reg)`: `S.MOV rd, rs1`: `rs1`を`rd`に移動
- `0x1 (reg)`: `S.ADD rd, rs1, rs2`: `rs1`と`rs2`を加算し、結果を`rd`に格納
- `0x2 (imm)`: `S.ADDI rd, imm8`: 符号拡張した`imm8`を`rd`に加算
- `0x3 (reg)`: `S.LD rd, rs1`: `rs1`内のアドレスから32ビットワードを読み込み、`rd`に格納
- `0x4 (reg)`: `S.ST rd, rs1`: `rs1`の値を`rd`内のアドレスのメモリーに格納
- `0x5 (imm)`: `S.BZ rd, off8`: `rd == 0`なら`PC + 2 + sign_extend(off8)`へ分岐
- `0x6 (imm)`: `S.BNZ rd, off8`: `rd != 0`なら`PC + 2 + sign_extend(off8)`へ分岐
- `0x7 (reg)`: `S.JR rd`: `rd`内のアドレスへジャンプ
- `0x8 (imm12)`: `S.JAL off12`: ジャンプしてリンク。`R31 <- PC + 2`、続いて`PC <- PC + 2 + sign_extend(off12)`
- `0x9 (imm)`: `S.LDI rd, imm8`: 符号なし`imm8`を`rd`に読み込む（`rd`の他のビットはクリア）
- `0xF (imm)`: `S.RET`: Short ModeからNormal Modeに戻る（`rd`と`imm8`フィールドは無視）。`PC <- PC + 2`、続いて`INSTR_MODE <- 0`

## 4.6 拡張I（割り込み）
拡張Iは外部割り込み、追加の読み取り専用レジスター、および割り込みハンドラーから戻る`IRET`命令をサポートします。

- `0x20 (reg)`: `IRET`: 割り込みから復帰（`PC <- EPC`、IRQを有効化）

## 4.7 拡張F（浮動小数点命令、FPU。将来の定義、未実装）
次のエンコーディングは将来の拡張のみを記述します。現行CPUは拡張Fをデコードも実行もせず、CPUIDでも広告しません。

FPUは専用の浮動小数点レジスターを設けず、すべてのレジスターを浮動小数点レジスターとして使用します（RISC-V Zfinxと同様）。浮動小数点命令は拡張形式でエンコードされます。
単精度（32ビット）の浮動小数点演算のみをサポートし、浮動小数点値はすべてIEEE 754形式で格納されます。

- `0x00 (EXTR)`: `FADD rd, rs1, rs2`: 浮動小数点加算: `rd = rs1 + rs2`
- `0x01 (EXTR)`: `FSUB rd, rs1, rs2`: 浮動小数点減算: `rd = rs1 - rs2`
- `0x02 (EXTR)`: `FMUL rd, rs1, rs2`: 浮動小数点乗算: `rd = rs1 * rs2`
- `0x03 (EXTR)`: `FDIV rd, rs1, rs2`: 浮動小数点除算: `rd = rs1 / rs2`
- `0x04 (EXTR)`: `FABS rd, rs1`: 浮動小数点絶対値: `rd = |rs1|`
- `0x05 (EXTR)`: `FNEG rd, rs1`: 浮動小数点符号反転: `rd = -rs1`
- `0x06 (EXTR)`: `FSQRT rd, rs1`: 浮動小数点平方根: `rd = sqrt(rs1)`

## 5. Normal Modeの実行セマンティクス

Normal Modeでは命令サイズは常に4バイトです。

デフォルトの次PC:

- `next_pc = PC + 4`

分岐およびジャンプのターゲット:

- `target = next_pc + sign_extend(off16)`

規則:

- `JAL`: `R31 <- next_pc`、続いて`PC <- target`
- `JR`: `PC <- R[rd]`
- `BEQ`: `R[rs1] == R[rs2]`なら分岐
- `BNE`: `R[rs1] != R[rs2]`なら分岐
- `SLT`: 符号付き比較
- `LD`/`ST`: 32ビットのビッグエンディアンアクセス
- `CPUID`: ID／機能を固定レジスターに書き込む
  - `R1 <- CPU_ID`
  - `R2 <- CPU_FEATURES`
- `HALT`: 有効なIRQが受理されるまで命令実行を停止。`IRET`により`HALT`の次の命令から再開

## 6. メモリーモデル（現行エミュレーター）

- アドレス空間: 32ビット
- 実装済みRAM: `0x00000000..0x00FFFFFF`（16 MiB）（拡張可能）
- バスは境界／デバイスをチェックし、マッピングされていないアクセスでpanicする。

エンディアン:

- バイトアドレス方式のメモリー
- 複数バイト値はビッグエンディアンのバイト順。非アラインアクセスを許可

### 6.1 CPU予約MMIO

CPUベクターと状態はバイトアドレス方式のMMIOを使用します。複数バイト値はビッグエンディアンのバイト順です。アクセスはbyte、halfword、word単位で可能で、非アラインでも構いません。

- `0xFFFF0000..0xFFFF000F`: リセットおよび例外ベクターワード。
- `0xFFFF0100..0xFFFF013F`: 書き込み可能なIRQベクター命令スロット16個。
  IRQエントリー時にPCを`0xFFFF0100 + CAUSE * 4`に設定し、そのスロットに格納された命令を実行します。
- `0xFFFF0040..0xFFFF004F`: IRQCのpending／enableレジスター。ベクター領域間の空き領域はMMIOデバイスに利用できます。
- `0xFFFF0200..0xFFFF0293`: CPU状態。`R0..R31`はオフセット`0x200..0x27C`を4バイトずつ占有し、PC/EPC/CAUSE/STATUS/INSTR_MODEはそれぞれオフセット`0x280/0x284/0x288/0x28C/0x290`を占有します。

`R0`はゼロを読み出し、書き込みを無視します。`STATUS` bit 0はIRQ有効化ビットです。
`INSTR_MODE` bit 0はNormal modeで0、Short modeで1です。書き込み時はbit 0のみを保持します。CPU所有の状態は命令commit時に公開されるため、CPU状態レジスターへのCPU storeはそのcommitで上書きされます。ベクターワードへのstoreは保持されます。

## 7. アセンブラー仕様（`asm.py`）

### 7.1 対応構文

- ラベル:
  - `start:`
- 命令:
  - `ADD R1, R2, R3`
  - `ADDI R1, R2, -4`
  - `LD R1, [R2 + 16]`
  - `ST R3, [R4 - 8]`
  - `BEQ R1, R2, loop`
  - `JAL func`
  - `LDIH R5, 0x1234`
  - `LDIL R5, 0x5678`
- `LDIH`/`LDIL`は即値としてラベルを受け取ります。`LDIH`は上位16ビットを読み込み、`LDIL`は下位16ビットを読み込みます。両方を使って32ビット絶対アドレスを構成します。
- ディレクティブ:
  - `.ORG <address>`
  - `.BYTE <value>`
  - `.WORD <value>` (16-bit)
  - `.DWORD <value>` (32-bit)
  - `.DB <v1>, <v2>, ...` (数値または引用文字列)
  - `.STRING "text"`

### 7.2 数値リテラル

- 10進数: `42`
- 16進数: `0x2A`
- 2進数: `0b101010`
- 意味がある場合、10進数／16進数／2進数には符号を付けられます。

### 7.3 分岐オフセット

`BEQ`、`BNE`、`JMP`、`JAL`、`JMPS`、`JALS`、`S.JAL`のラベルオペランドまたは絶対数値ターゲットについて:

- `offset = label_address - (current_pc + instruction_size)`。`instruction_size`はNormal modeでは4、`S.JAL`では2です。

数値ターゲットは絶対バイトアドレスとして扱われるため、`target`がアドレス`0x100`にある場合、`JMP 0x100`と`JMP target`は同じエンコーディングになります。

数値オペランドに`R!`を前置すると、アセンブラーは絶対アドレスではなく明示的な相対オフセットとして扱います。

- `JMP R!0`は次の命令へジャンプします。
- `JMPS R!0`は次の命令でShort Modeに入ります。

## 8. ビルドと実行

Rust CPUデモ:

```bash
cargo run
```

Pythonアセンブラー:

```bash
python asm.py input.s -o output.bin
```

## 9. 状況

このリポジトリで実装済み:

- CPUのフェッチ／デコード／実行コアループ
- ベースISAおよび拡張A、L、M、S、I（拡張Fは未実装）
- 16 MiB RAM を備えたバス
- CPU予約ベクター／状態MMIO
- ラベル／ディレクティブ対応の2パスアセンブラー

計画中（将来）:

- 現行の不正命令／バスpanic動作を超える例外モデル
- ROMローダー／起動フロー
- アセンブル済みプログラムによる統合テスト

計画中の拡張:
V: 仮想メモリー、MMU、TLB
C: L1 Cache
P: 保護／特権レベル（拡張Iに依存）
p: パイプライン処理（拡張pとして実装済み。[[doc/ja/spec_CPU_p|拡張p仕様]]を参照）
v: ベクトル命令、SIMD
F: 浮動小数点命令、FPU
a: Atomic命令、メモリーバリア、SMT
