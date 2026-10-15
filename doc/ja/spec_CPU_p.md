---
lang: ja
translation_of: ../spec_CPU_p.md
---

# SRC32拡張p（小文字）— パイプライン処理

Revision 0.1 (2026-10-15). 状態: 実装済み（`src/cpu_pipeline.rs`）。

## 1. 拡張体系における位置付け

大文字と小文字は区別されます。大文字の`P`（Protection/Privilege Levels）と小文字の`p`（Pipelining）は別の拡張です。

- `P`: 保護／特権レベル（拡張Iに依存）。未実装。
- `p`: パイプライン処理。マイクロアーキテクチャのみで、**ISAの変更はありません**。実装済み。

`p`はCPUID bit 7で通知されます。

- `EXT_P_SMALL = 0x80`（名前は小文字`p`の意味を保持しますが、定数名はRustの大文字規約に従います）。
- パイプラインCPUは`CPU_FEATURES = 0xBF`（`Base|A|L|M|S|I`に`p`を加えたもの）を報告します。スカラー参照CPUは引き続き`0x3F`を報告します。
- Bit 6は拡張F用に予約されたままです（定義済み、未実装）。

## 2. マイクロアーキテクチャ（`p`が保証するもの）

5段、インオーダーのパイプライン: `IF / ID / EX / MEM / WB`。

- 32ビットのNormal命令と16ビットのShort命令はすべて同じステージを通ります。フェッチ幅はフェッチモードに従います。モード切り替え（`JMPS`/`JALS`/`JRS`/`JALRS`、`S.RET`）はEXで解決される分岐です。
- 各ステージは`valid`と、その命令固有のPCおよびモードを保持するため、commitは正確な状態を保ち、flush時にbubbleと実際のNOPを取り違えません。
- `R0`は読み出し時にゼロを返し、commit時の書き込みを無視します。forwardingとhazardチェックの対象外です。
- 理想状態ではパイプライン充填後に1サイクルあたり1命令を処理します。stall、flush、複数サイクル演算により、実際のIPCは1未満になります。

### 2.1 Forwarding

- EX入力は`EX/MEM`、次に`MEM/WB`、最後にレジスターファイルから選択します（EX時にライブ読み出しするため、ID後にcommitしたproducerの値も参照できます）。
- `EX/MEM`にあるloadはforwardingしません（データが到着するのはMEM時）。このケースは以下のload-use stallで処理します。

### 2.2 Load-use hazard: 1サイクルstall

`LD`の直後にロード先レジスターを読む命令がある場合、fetchとIDを1サイクル停止し、loadが`EX -> MEM`へ進む間に`ID/EX`へbubbleを挿入します。待機していた命令はその後に再デコードされ、`MEM/WB`から値をforwardingします。

### 2.3 分岐: EXで解決、予測なし

`BEQ`/`BNE`/`JMP`/`JAL`/`JR`（およびShort modeの対応命令）はEXで解決します。分岐成立時はfetch先を変更し、`IF/ID`と`ID/EX`をflushします（2命令分のペナルティ）。`JAL`/`JALR`はcommit時にfall-through PCを`R31`へリンクし、リンク値はforwardingに使用できます。

### 2.4 複数サイクルのALU演算

- `MUL`/`MULH`はEXを3 tick占有し、`DIV`/`DIVU`/`MOD`は16 tick占有します（スカラー参照CPUと同じレイテンシ）。フロントエンドを停止し、`MEM`/`WB`をdrainしている間、演算は`ID/EX`で待機します。結果と端点動作（ゼロ除算→0、`INT_MIN / -1`→`INT_MIN`、`INT_MIN % -1`→0）はスカラーCPUと完全に一致します。

### 2.5 正確な割り込み、HALT

- IRQは命令commit境界でのみサンプリングされます。`EPC`は最後にcommitした命令の次のアドレス（`retire_pc`）と保存モードを保持し、`IRET`はその両方を復元します。
- `HALT`はcommit時に有効になり、先読み済みのfall-through命令を破棄します（スカラーCPUはそれらをフェッチしません）。IRQによるwake-up時は`EPC`から再フェッチするため、`HALT`後のコードはwake-up後にちょうど1回実行されます。

### 2.6 メモリーとバス

- 各命令の`MEM`アクセスは、各commit経路でちょうど1回実行されます。stall時はfetchを保持し、バス要求を再発行しません。`EX/MEM`は命令の`MEM`を再実行せずbubbleへdrainするため、MMIO副作用は1回だけ発生します。
- 単一バス規則: 1 tick内では`MEM`が`IF`より優先されます（次のfetchより先に`MEM`を処理）。既存のバス調停によって、CPUアクセスに先立ちDMAのdrainが継続されます。
- 将来のwait-state／バスエラー対応では`IF`を保持し（`MEM`を決して重複させない）、エラーを障害命令のPCに結び付ける必要があります。

### 2.7 カウンター

`cycles`、`instructions_executed`（WBでのcommit数）、`stall_cycles`（load-useおよびMUL/DIVによる停止）、`flush_events`（分岐成立によるredirect）。

## 3. 参照モデルとテスト

- スカラーの`Cpu`（`src/cpu.rs`）が参照モデルです。同じプログラムと同じ初期メモリーを与えれば、レジスター、アーキテクチャ上のPC、メモリーが一致します。`tests/cpu_pipeline.rs::differential_mixed_program_matches_scalar`（34命令の混合プログラム）および端点テスト（除算端点、MULH/DIVU、Short-mode往復、HALT/IRQ wake-up、CPUID）で実証されています。
- エミュレーターはデフォルトで`PipelineCpu`を起動します。選択処理は`src/cpu_select.rs`（`Cpu`/`PipelineCpu`を同じharness APIで扱う`AnyCpu`）にあります。優先順は、まず`--cpu scalar|pipeline` CLIフラグ、次に`config.yaml`の`cpu.microarchitecture`キー、最後にpipelineデフォルトです。スカラーの`Cpu`は動作参照モデルおよび差分テスト用に残されています。

## 4. ファイル

- `src/cpu_pipeline.rs` — 実装。
- `tests/cpu_pipeline.rs` — TDDテストスイート（12テスト）。
- [[doc/ja/spec_CPU|SRC32 ISA仕様]] — ISAの基準仕様（セマンティクス変更なし）。
