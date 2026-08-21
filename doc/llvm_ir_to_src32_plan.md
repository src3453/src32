# LLVM IR → SRC32アセンブリ変換基盤 計画書

- 作成日: 2026-08-21
- 対象リポジトリ: `/mnt/c/users/y2k34/remote/src32`
- 状態: 事前調査に基づく初期計画

## 1. 目的

LLVM IR（当面はテキスト形式 `.ll`）を入力として、SRC32アセンブリを安定して生成する基盤を整備する。最初からLLVM全仕様を実装するのではなく、SRC32のABI・命令仕様・ランタイム制約を明文化し、検証可能な段階的サブセットとして拡張する。

## 2. 調査結果

### 2.1 既存資産

- `tools/csc/llvm_ir_parser.py`
  - 独自トークナイザ、LLVM IR AST相当、`CodeEmitter`への変換を実装済み。
  - 対応を表明している主な命令は `add/sub/mul/sdiv/and/or`、`icmp`、`alloca/load/store`、`call`、`br`、`phi`、`ret`、一部の整数型変換。
  - 型は実質的に `i32` 中心で、グローバル、配列・構造体、浮動小数点、ベクトル、例外、inline asmは未対応。
- `tools/csc/backend_src32.py`
  - 中間のスタック型バイトコードをSRC32アセンブリへ変換する既存バックエンド。
  - `R28`をSP、`R30`をGP、`R31`をリンクレジスタ、戻り値を`R1`とする前提。
  - `main`呼び出し、分岐、算術、比較、関数呼び出し、変数領域を生成する。
- `tools/csc/csc.py`
  - `--from-llvm`でLLVM IR入力を受け付けるCLIが既に存在する。
- `tools/csc/test_llvm_ir_parser.py`
  - パーサ、コード生成、アセンブリ生成、CLIのテストが既に存在する。
- `tools/asm/asm.py`
  - SRC32アセンブラ。生成物をバイナリ化する検証面として利用できる。
- `doc/spec_CPU.md`
  - SRC32 ISA Revision 2.1。32-bit標準命令、Extension S、算術・分岐・ロードストア・割り込み等の仕様が定義されている。
- `doc/spec_csc.md`
  - 既存CSCの対象言語と今後の拡張方針を定義している。

### 2.2 現状の重要なリスク

1. **独自LLVMパーサの適用範囲**: LLVM IRは属性、メタデータ、opaque pointer、識別子の多様な形式を持つため、正規表現中心の独自実装を拡張し続けると入力互換性と保守性が低下する。
2. **CFGとphiの扱い**: 現在の`phi`処理は先頭候補を格納する簡略化であり、制御フロー意味論を満たさない。基本ブロック終端とphiのエッジコピーを分離して扱う必要がある。
3. **メモリモデル**: `alloca`をローカル変数番号へ置き換える実装は、ポインタ演算、エスケープ、配列、アラインメントを表現できない。
4. **ABIの未固定部分**: 引数、戻り値、caller/callee-savedレジスタ、スタックフレーム、外部関数、`i1`/各種整数幅、符号拡張の規約を正式に固定する必要がある。
5. **即値・命令生成**: 32-bit定数、シンボル、分岐距離、ロードストアのオフセット、`LDIH/LDIL`の組み合わせを一貫した lowering規則で扱う必要がある。
6. **既存作業の保護**: 作業ツリーには広範囲の未コミット変更があるため、実装時は変更範囲を限定し、既存変更を上書きしない。

### 2.3 ツール環境

- Rust/Cargoは導入済みで、プロジェクトの`cargo build`は成功済み。
- `python3`は利用可能。
- `gcc` 15.2.0は利用可能だが、`gcc -S -emit-llvm`はLLVM IRを生成せず、通常のx86-64アセンブリを出力した。GCCをLLVM IR生成ステップとして直接使う方針は成立しない。
- `clang`、`llc`、`opt`、`llvm-as`は調査時点でPATH上に確認できなかった。LLVM IR生成にはClang等の導入、またはGCCのGIMPLEからの別変換が必要。
- 外部Web検索は調査環境で利用できなかったため、LLVM公式資料の最新内容は未検証。

## 3. 推奨アーキテクチャ

### 3.1 段階1: LLVM IRサブセット用の実用コンパイラ

当面はLLVM全体のターゲットバックエンドではなく、次のパイプラインにする。

```text
.ll
  ↓
LLVM IR parser / validator
  ↓
typed module + CFG + data-flow information
  ↓
normalized IR (SRC32 lowering対象)
  ↓
ABI lowering / instruction selection
  ↓
virtual-register assembly IR
  ↓
register allocation / stack spill
  ↓
SRC32 assembly
  ↓
SRC32 assembler
```

既存の`CodeEmitter`をすぐ廃止せず、互換用の経路として残す。新基盤では、解析・変換・命令選択・レジスタ割り当てを分離し、各段階を単体テストできる構造にする。

### 3.2 将来の選択肢

- LLVM公式のout-of-tree target backend（LLVM TableGen、SelectionDAGまたはGlobalISel）: LLVMの最適化と正確なIR処理を活用できるが、ターゲット記述・ABI・ランタイム整備のコストが大きい。
- 現行のPython実装を強化: 小さく始められ、SRC32独自仕様に合わせやすい。まずはこちらを推奨する。
- 推奨方針は、Pythonで検証可能なサブセット基盤を完成させ、ABIと命令選択が安定した段階でLLVM公式バックエンド化の費用対効果を再評価すること。

## 4. 目標スコープ

### MVP（最初の完成点）

- モジュールと複数関数
- `i32`、`i1`、`void`（必要最小限）
- 整数定数、SSA値、基本ブロック、正しいCFG
- `ret`、`add/sub/mul/sdiv`、`and/or/xor`、`icmp`
- `br`（無条件・条件付き）
- `call`（定義済み関数。外部関数は明示的なruntime宣言のみ）
- `alloca/load/store`の非エスケープなローカル`i32`
- SRC32アセンブラで再アセンブル可能な出力
- エラー位置と未対応命令を明確に示す診断

### 次段階

- `zext/sext/trunc`の型正当性を伴う実装
- `phi`のエッジコピー
- `i8/i16/i64`と符号付き・符号なし演算
- グローバル変数、文字列、配列
- 外部関数とruntime ABI
- ループ、再帰、レジスタ割り当て改善
- `getelementptr`、関数ポインタ、readonly属性等の必要部分
- Extension Sを使用した安全な命令短縮

### 対象外（初期段階）

浮動小数点、SIMD、原子操作、例外、GC、デバッグ情報完全保持、全LLVM属性、任意の最適化パス。

## 5. ABI案（実装前に仕様として確定）

- `R0`: hardwired zero
- `R28`: SP
- `R29`: FP（フレームポインタを使う段階で導入）
- `R30`: GPまたは静的データ基準
- `R31`: return address
- `R1`: 32-bit戻り値
- 引数: MVPではスタック渡し、または固定レジスタ渡しのどちらかに統一する。既存実装との互換性を優先するなら、まずスタック渡しを正式化する。
- caller/callee-saved集合、スタック成長方向、アラインメント、関数入口・出口、`void`戻り、外部シンボル規約を仕様書に追加する。

## 6. 実装フェーズ

### Phase 0: 仕様固定と安全な作業境界

1. 未コミット変更を基準として記録し、対象ファイルを限定する。
2. `doc/spec_CPU.md`から命令・分岐・メモリの生成制約を抽出する。
3. `doc/spec_llvm_ir_to_src32.md`（新設）に型、ABI、対応命令、エラー方針を定義する。

**完了条件:** ABI表とMVP対応表が文書化され、実装上の未決定事項が列挙されている。

### Phase 1: 入力解析と検証の分離

1. 現行パーサのASTを、module/function/block/instruction/type/valueの明示的な型へ整理する。
2. 命令ごとの型検証、定義前使用、重複定義、終端命令、CFG参照先を検証する。
3. unsupported構文を黙って解釈せず、行番号付きエラーにする。
4. 既存の簡易入力との互換テストを維持する。

**完了条件:** 不正なIRがコード生成前に診断され、正常なMVPサンプルが正規化IRまで到達する。

### Phase 2: CFG・SSA lowering

1. 基本ブロックの終端を正規化する。
2. `phi`を各 predecessor edge のコピーへloweringする。
3. 定数とSSA値を仮想レジスタ／スタックスロットで追跡する。
4. `alloca`は非エスケープローカルだけをslotへ割り当て、対象外のポインタ利用を拒否する。

**完了条件:** 分岐、合流、ループ、phiを含むテストが期待値を返す。

### Phase 3: SRC32命令選択とABI lowering

1. 二項演算、比較、分岐、ロード・ストア、定数ロードの選択規則を実装する。
2. `LDIH/LDIL`、符号付き即値、アドレスシンボルを統一的に処理する。
3. 関数プロローグ・エピローグ、引数、戻り値、call/returnをABIに従って生成する。
4. 初期版は安全性と正しさを優先し、必要なら全値をスタックにspillする。

**完了条件:** 生成アセンブリを`tools/asm/asm.py`で再アセンブルでき、CPUテストまたはVM実行でMVPケースが一致する。

### Phase 4: CLI・診断・回帰テスト

1. `csc.py --from-llvm`を新パイプラインへ接続する。
2. `--dump-ir`、`--dump-lowering`、`--dump-asm`相当のデバッグ出力を追加する。
3. `.ll → .s → .bin`のE2Eテストを追加する。
4. Cコンパイラ経路とLLVM経路の共有部分・分離部分を整理する。

**完了条件:** CI相当の一括テストで、parser/lowering/backend/assemblerの各段階が検証される。

### Phase 5: 最適化とLLVM公式バックエンド評価

1. 命令数、スタック使用量、spill数を計測する。
2. 定数畳み込み、不要コピー削除、簡易peephole、Extension S短縮を追加する。
3. 実用上不足が明確になった場合のみ、LLVM out-of-tree target backendの試作を行う。

**完了条件:** 最適化前後のベンチマークと、Python基盤継続／LLVM公式バックエンド移行の判断記録がある。

## 7. テスト計画

### 単体テスト

- tokenizer/parser: コメント、識別子、型、属性、数値、エラー位置
- verifier: 型、SSA、終端、CFG、phi
- lowering: 各LLVM命令から中間表現への変換
- instruction selector: 命令とオペランドの期待値
- ABI: 引数、戻り値、再帰、caller/callee保存

### E2Eテスト

- return定数
- 算術と比較
- if/else
- whileループ
- ループ内phi
- 関数呼び出しと再帰
- alloca/load/store
- assembler再変換
- CPUまたは既存VMによる実行結果

### 失敗系

- 未対応命令
- 不正な型
- 未定義SSA値
- 呼び出し先不明
- 不正なCFG
- エスケープするalloca
- 即値・分岐距離・アラインメント違反

## 8. 受け入れ基準

1. MVP対応範囲の有効なLLVM IRを、決定的にSRC32アセンブリへ変換できる。
2. 生成アセンブリを既存アセンブラでバイナリ化できる。
3. 分岐・phi・関数呼び出しを含むテストで、LLVM実行意味論とSRC32実行結果が一致する。
4. 未対応入力は、入力位置・命令名・理由を含むエラーになる。
5. 既存のC→SRC32経路と既存の未コミット変更を壊さない。
6. ABIと対応範囲が仕様書に反映されている。

## 9. GCCを含む現実的な入力パイプライン

GCCはLLVM IRを標準形式として生成しない。実測では `gcc -S -emit-llvm sample.c -o sample.ll` はLLVM IRではなくx86-64 GNUアセンブリを出力した。

推奨パイプラインは次のとおり。

```text
C → clang -S -emit-llvm -O0 → LLVM IR (.ll)
  → parser/verifier → SRC32 lowering/ABI
  → SRC32 assembly (.a) → tools/asm/asm.py → binary (.bin)
  → emulator/CPU test
```

GCCを必須にする場合は、GIMPLEダンプを入力にする別系統（GIMPLE→SRC32またはGIMPLE→独自IR）となり、LLVM IRパイプラインとは別物として設計する。

## 9. 最初に着手する具体的タスク

1. 現行LLVMテストを実行し、ベースラインを記録する。
2. `gcc -S -emit-llvm`がLLVM IRを生成しないことを記録する。
3. Clangを導入可能か確認し、Clang生成IR（`dso_local`、`ptr`、`align`、debug metadata等）を固定する。
4. `llvm_ir_parser.py`の現行出力と、`backend_src32.py`の既存ABI前提をテストで固定する。
5. LLVM→SRC32専用の型・ABI仕様書を追加する。
6. `verify_module()`を追加し、コード生成前検証を導入する。
7. phiを含むCFGテストを追加する。
8. `.c → .ll → .a → .bin`の最小E2Eテストを追加する。

## 10. 判断

既存リポジトリにはLLVM IRからSRC32へ変換する試作が既にあり、ゼロからの新規作成ではない。したがって、最初の目標は「LLVM公式バックエンドを即時実装」ではなく、既存試作を検証可能な parser / verifier / lowering / backend の層へ分解し、MVP ABIを固めることとする。その後、入力互換性や最適化要求が独自実装の限界を超えた時点で、LLVM公式バックエンドへの移行を再判断する。
