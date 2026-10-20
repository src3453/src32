## 作業で得た知識・注意点

- WAX v2 (`doc/spec_WAX.md`) は 64B big-endian header と 12B block table entry
  (`load_address`, `file_offset`, `size`) を使う。各 payload は 4B file alignment、load range は
  昇順・非重複・u32 範囲内。`base` は `.wax` 最小 load address、entry は実在 block 内のみ有効。
  Loader は block のみを書き、hole をゼロ埋めしない。v1 single text/rodata 形式とは互換なし。
  Python pack/parse は `tools/wax/wax.py`; Rust `WaxImage.blocks` は main/monitor/testbench/BIOS でロード。
- `tools/asm/asm.py -o output.wax` は拡張子で WAX を選択し、forward `.ORG` ごとに sparse block を生成する。
  `.bin` と他 suffix は従来どおり flat raw output。`--entry` は label または absolute address。
- `cargo test` (lib) は `src/devices/sgu/sqv4.rs` の `include_str!("../../../../s3w2_core/tests/...")`
  がリポジトリの親ディレクトリ側 `s3w2_core` を指し、存在しないためコンパイル失敗する。wax 単体は
  空スタブを親側に一時配置して `cargo test wax::` で通した。スタブは必ず削除すること。
- testbench の `--expect-mem-u8` で `ST` (32-bit ワード格納) を確認する際は big-endian のため
  LSB は +3 番地に来る (例: 42 を 0x2000 に ST したら 0x2003=42 で確認)。
- BIOS ROM (src/devices/bios.rs): config `bios.image` (config 基準の相対 path) を
  `0xFF000000` の 64 KiB read-only window に zero-pad して map し、entry を返す。
  raw は先頭配置・PC=`0xFF000000`、WAX は extent が window 内のものだけ受付。
  ROM への guest write は silent-ignore (CPU は device 側 bus_error を見ないため fault 化しない)。
  section 欠落・null・config 不在は `Ok(None)` で RAM 起動に fallback。起動経路は
  main raw guest / monitor / testbench の全てで BIOS entry を優先する。
- BIOS POST が一瞬で消える件: GUI も testbench も 1 ホストフレーム毎に IRQ0 パルス→
  `cpu.run(CYCLES_PER_FRAME=800k)`→描画なので、待ち無しの BIOS は最初の 1 フレーム内で
  POST+IPL が完結し POST が描画されない。対策は BIOS 側で VBlank(IRQ0)待ちを入れること
  (sol/bios.sol v0.3: `fn irq0` で ack+計数し `while halt ... end` で 120 フレーム保持)。
  CPU サイクル数のビジーウェイトではホストフレームを跨げない。ハンドラは PENDING bit0 を
  必ず ack すること (ack 無しだと IRQC 出力が High のまま edge が来ず HALT から二度と起きない)。
  IPL直前に `0 IRQC_ENABLE sth` + PENDING クリアを忘れずに (ベクタは BIOS dispatcher を指した
  ままなので)。GUI と testbench は共に毎フレーム IRQ0 を供給するため HALT 待ちは両方で動作する。
  assets/bios.rom は sol/bios.sol ヘッダのコマンドで再構築する (64KiB 上限、`--strip` 必須)。
- デバッグ GUI (`--debug-gui`) のメインメニューバー重なり対策: `src/render.rs` の
  `game_viewport(size, top_reserve_px)` が描画とマウス変換の共通計算を持つ。
  `DebugGui::render_frame` は `ui.frame_height()` (論理 px) を保持し
  `display_framebuffer_scale` で物理 px 化した値を `render_with_overlay` の予約量に渡す。
  ゲーム画面は予約量の下に中央寄せし、通常 GUI (`render()`) は予約 0 で従来通り。
  マウス絶対・相対も同じ予約量で換算する。初フレームは高さ 0 のため 1 フレームだけ
  被る可能性がある。
- TinyBASIC インタプリタ (`tinybasic.sol`, リポジトリ直下): sol 言語で記述した
  TinyBASIC 実装。PCG をコンソール出力、PeC をキーボード入力に使い、IDC/DMAC 経由の
  `LOAD lba sectors` / `SAVE lba sectors` (既定 128 セクタ、LBA 0) も実装する。
  行はプレーンテキストで 0x00200000 に 128B  stride で保管 (1-9999 行、<=120 文字)、
  変数 A-Z は 0x00170000、配列 @(0..999) は 0x00180000。ビルドは
  `python3 tools/solc/solc.py compile tinybasic.sol -o /tmp/tinybasic.asm` →
  `python3 tools/asm/asm.py /tmp/tinybasic.asm -o tinybasic.bin`。
  検証は SolVM 回帰 (tools/solc/sol_vm.py, 19 ケース) と実 CPU テスト
  `cargo test --test tinybasic` (起動時 help/プロンプト描画、キーボード入力で
  `10 print 2^10` を実行して 1024 を描画)。テストは tinybasic.bin が既存である
  ことを要求する (*.bin は .gitignore 対象なので先に組み立てる)。
- sol の真偽規約は比較が真=0・偽=1 で、`if`/`while` は条件 0 でブロック実行、
  `and`/`or` はビット積/ビット和。よって「比較A かつ 比較B」は
  `aA bB or`(両方 0 のときだけ 0)、「比較A または 比較B」は `aA bB and`(いずれか
  0 なら 0) と書く。`tb_key_ascii` のシフト判定や範囲判定でこの写法を使う。
  間違えると分岐が反転し無限ループや死んだエラー分岐になる。
- sol の `while ... end` は do-while: 本体を先に実行し、`end` でスタック
  トップを消費して 0 なら先頭へ戻る (本体は必ず 1 回実行される)。本文の
  最後に条件値 (継続=0・脱出=非0) を残す。`fn` の `local` 初期値は単一の
  数値リテラルのみで、式 (`local x 1+2`) は先頭の数値だけ初期値になり残り
  が本体に残る。計算値は別行で `>x` 代入する。関数引数への代入 (`>arg`) は
  対応済みで現在フレームのスロットを更新し、呼び出し元には影響しない。
- アセンブラ (tools/asm/asm.py) にファージャンプ・トランポリンを追加: ±32767B を
  超える JMP/JAL/BEQ/BNE は、直後に `LDIH/LDIL R14, target; JR/JALR R14` の
  トランポリンを置き、ブランチをそこへ向ける。条件分岐はフォールスルー保護の
  `JMP end` を挟む。R14 はコンパイラがブランチ前にスタックキャッシュをフラッシュ
  するため全分岐サイトで dead であり、破壊して安全。トランポリン挿入はアドレスを
  ずらすため pass1 を反復して不動点まで進める。短いモード分岐 (S.BZ/S.BNZ/S.JAL)
  はコンパイラが小ブロック内にしか出さないため対象外 (従来の範囲チェックのまま)。
  これで ~74KB の sol プログラム (TinyBASIC) も組み立て可能になった。
- PCG レガシーモード (SCREEN_MODE=0, pcg.sol の initPCG) のテキストプレーンは
  VRAM 0x10000000 から 1 セル 1 バイト (文字コード=glyph index)、FG/BG 属性は
  +0x1000/+0x2000 の 1 バイト (下位 6 bit が CLUT index)。モード 2 (0xF003=2)
  のみ UTF-16BE 2 バイト/セル。sol VM で VRAM を 40x30 として再構成するとテキスト
  出力を読める。
- solc でハマりやすいエッジケース (TinyBASIC 実装で多発):
  - **真偽値の反転 (最多発)**: 比較は真=0・偽=1。`if`/`while` は条件 0 で
    実行/継続。`and`/`or` はビット積/ビット和なので「比較A かつ 比較B」は
    `aA bB or`、「比較A または 比較B」は `aA bB and`。入れ替えると分岐が
    反転し無限ループや死んだエラー分岐になる。
  - **前置記法の引数順**: 呼び出しは `arg1 arg2 fnname`。ヘルパ呼び出しで
    順序を逆にしやすい (`tb_pos tb_src_byte >ch` のように「対象 関数」)。
    呼び出し側は引数を左から右に push する。
  - **local 初期値は単一数値リテラルのみ**: `local x 5` は可。式
    (`local x 1+2`) は先頭の数値 (1) だけが初期値になり、残り (`+2`) が
    本文のストレイトークンとして残る。計算値は別行で `1 2 add >x` 等とする。
  - **while は do-while**: 本文を先に実行し `end` でスタックトップを消費、
    0 なら先頭へ戻る (本文は必ず 1 回実行される)。本文末尾に条件値を残す。
  - **ret/retn の静的スタック均衡検査**: `ret` は戻り値を 1 個、`retn` は
    0 個残して終了する必要があり、コンパイラが全経路のスタック深さを検査
    する。ループ内でスタック深さが増えると「stack overflow: loop increases
    depth」でコンパイルエラーになる。
  - **ラベルと raw jump は不可**: `@label` / `jmp` / `jz` / `jnz` は
    コンパイラが拒否し、`if`/`else`/`while`/`end` の構造化構文に変換される。
  - **暗黙の retn**: 関数本体に明示的な `ret`/`retn` がなければ終端に
    暗黙の `retn` が挿入される (戻り値なしで復帰)。
  - **drop**: `drop` はスタックトップの値を破棄する (例: `tb_read_line drop`)。
  - **文字列リテラル**: `"..."` は UTF-8+NUL、`u16"..."` は UTF-16BE+2B NUL
    で rodata に配置され、先頭アドレスがスタックに置かれる。
  - **ディレクティブ**: `!include "file" [as ns]` でライブラリ統合
    (pcg.sol / pec_rng.sol 等)。`!const` / `!var` / `!stack_top` /
    `!var_base` で定数・変数・スタック先頭・グローバル変数ベースを指定。
  - **同一スコープで同名の local 変数は禁止**、`irq0`〜`irq15` という名前の
    引数なし関数は IRQ ハンドラとして扱われ R30 が予約される。
- **PCG カーソルはタイルマップ座標** (pcg.sol の改行カーソルずれ):
  CURSOR_POS_X/Y (0xF005/0xF006) はスクロール後のタイルマップ座標。
  表示は画面 (X,Y) から ((X+dX) mod W, (Y+dY) mod H) を読み、カーソルも
  その座標系で描画される (doc/spec_VDP.md、src/devices/vdp/vdp.rs の
  合成パス)。したがってスクロール後に論理スクリーン行 pcg_cur_y を書くと
  カーソルが scroll_y/8 行だけ上にずれる。正しい値はリング物理行
  pcg_vram_row = (scroll_y/8 + cur_y) mod height。`_pcg_update_cur` は
  pcg_vram_row を書く。検証は sol VM で MMIO シャドウ (0x8000F00E/F,
  0x8000F005/6) を読み、表示行 (cursor_y - scroll_y/8) mod 30 が論理行
  と一致することを確認する (test_sol_vm.py の
  test_pcg_cursor_register_follows_tilemap_row_when_scrolled)。
- **入力待ちは HALT、IRQ3 は PeC** (tinybasic.sol):
  tb_read_line はイベントなし時に `halt` で CPU 停止。PeC キー
  ボードは IRQC source 3。IRQC の PENDING/ENABLE は 16-bit レ
  ジスタを BE の 2 バイト lane に分割するため、source 0-7 は
  `mask IRQC_PENDING sth` (W1C) / `mask IRQC_ENABLE sth` (許可)
  で操作する (bios.sol / mouse_cursor.sol / sgu_music_driver.sol
  と同様)。irq handler は必ず IRQC pending を W1C で消すこと:
  消さないと IRET 後も irqc_line が high のままになり、FIFO を
  使い切って source level が下がった後も CPU の立ち上がり edge
  検出が働かず、HALT から永遠に復帰できない (deadlock)。irqN
  handler があれば起動コードが全 16 vector に `JR R30` を書き、
  dispatcher が CAUSE で番号判定→全レジスタ保存→handler 呼出→
  IRET。handler が無い番号 (例: IRQ0 VSync) も save/restore して
  無害に復帰する。なお sol VM の halt は IRQ モデルが無いため単
  なるプログラム終了であり、VM ベースのテストは tb_read_line
  に到達してはならない (tests/tinybasic.rs は実 CPU で検証)。
- **IDC `incomplete_sector_policy`** (src/devices/pec/idc/idc.rs):
  disk 要素に `incomplete_sector_policy: reject|pad|truncate`
  を追加し、512 の倍数でない image の末尾端数byteの扱いを
  変えられるようにした。`reject` が既定で既存契約（起動時
  エラー）を維持。`pad` は capacity を ceil にし、最終
  sector の READ は端数の後に0埋め、WRITE は端数 prefix の
  み commit して image を延長しない（`Drive::partial_tail`
  と `trailing_bytes` で実装）。`truncate` は端数を無視して
  capacity を floor にする（完全 sector が 0 個なら起動時
  エラー）。guest には常に 512B sector として見え、DMAC
  転送量や deadline 式は変わらない。cartridge パッケージ
   image は従来通り 512 の倍数のみ許可。テストは
  `cargo test --lib idc::`（s3w2_core golden 欠落環境では
  一時 stub を ../s3w2_core/tests に置いて実行し直後に削除）。
  仕様は doc/spec_IDC.md §3/§7、使用ガイドは
  doc/DMA_IRQ_PeC_IDC_howtouse.md。
- **TinyBASIC LOAD は capacity までクランプ** (tinybasic.sol):
  LOAD/SAVE の sector count 既定値は 128（64 KiB）な
  ため、小さい .bas image を disk 0 に直接アタッチ
  して `LOAD` を実行すると IDC の LBA 範囲検査
  （ERROR_CODE=3）で "DISK READ ERROR" になって
  いた。修正は `tb_disk_capacity`（IDENTIFY コマンド
  で CAPACITY_SECTORS = TB_IDC+0x30 を読む。事前に
  STATUS の sticky DONE/ERROR を 6 で W1C 消去する
  こと）を追加し、LOAD の sector count を
  `capacity - lba` までクランプする。lba >= capacity
  の場合も DISK READ ERROR。SAVE は既定値のまま
  （明示指定が前提）。検証は tests/tinybasic_load_disk.rs
  （pad policy の非 512 倍数 image で LOAD→LIST）。
  再現・修正確認は tinybasic.sol を編集→
  solc compile→asm.py→cargo test --test の順。
- **TinyBASIC エラーに行番号表示** (tinybasic.sol):
  実行中の `tb_line` は常に現在行を指す（`tb_jump_line`
  はエラー時に tb_line を更新しないため GOTO 飛び先
  不正でも発生元の行が残る）。`tb_report_error` を追加
  し program 実行のエラーは "? ERROR IN LINE <n>"、
  直接モード（行番号を持たない・tb_line は前回 RUN の
  残留値）は "? ERROR" とした。旧メッセージ
  "? SYNTAX/LINE ERROR" は廃止。検証は
  tests/tinybasic.rs の tinybasic_error_reports_line_number
  （"10 print 1/0" + run → テキストプレーンに
  "? ERROR IN LINE 10" が描画されることを確認）。
  再現・修正確認は tinybasic.sol 編集→
  `python3 tools/solc/solc.py compile tinybasic.sol -o
  tinybasic.sol.a` → `python3 tools/asm/asm.py
  tinybasic.sol.a -o tinybasic.bin` →
  `cargo test --test tinybasic`。
- SRC32 Extension p (小文字=パイプライン、大文字 P=Protection と区別。
  定数は `EXT_P_SMALL = 0x80`)。`src/cpu_pipeline.rs` に 5 段
  IF/ID/EX/MEM/WB のインオーダーパイプラインを実装。ISA 変更なし。
  パイプライン CPU は CPUID bit7 を立て `CPU_FEATURES=0xBF`、スカラー
  `Cpu` は参照モデルとして `0x3F` のまま残す。仕様は `doc/spec_CPU_p.md`、
  テストは `tests/cpu_pipeline.rs` (12 件、スカラーとの差分比較含む)。
- パイプライン実装の 3 つの落とし穴 (いずれもテストで検出):
  1. tick 内では旧パイプラインレジスタを全段同時読みすること。
     WB コミット直後に `mem_wb` を無効化すると同 tick の EX が
     MEM/WB フォワーディング元を失う (ADD 連鎖が古い値を計算)。
  2. ロードユース停滞では ID/EX にバブルを入れ、ロード自体は
     EX→MEM へ進めること。ID/EX にロードを保持するとハザードが
     永遠に解消せず無限ストールする。
  3. ID でサンプルしたレジスタ値を EX でそのまま使わないこと。
     MUL/DIV の複数サイクル遅延中に生産者がコミットするとサンプルが
     陳腐化する。フォワーディング不発時はライブのレジスタファイルを
     読む (インオーダー確定なので若い命令の値を見る心配はない)。
- HALT はコミット時にパイプラインをフラッシュし、EPC には投機フェッチ
  ポインタではなく `retire_pc` (最終コミット命令の次アドレス) を使うこと。
  そうしないと HALT を越えて先取りした命令が IRQ 復帰後に二重実行・
  または消失し、スカラーとずれる。割り込みはコミット境界でのみ受理。
- Early-return MUL/DIV stall paths must accept a pending IRQ before syncing the CPU
  register block. Reversing that order leaves stale interrupted PC/status in the
  block; the next empty-pipeline tick overwrites the IRQ vector and skips the guest
  handler. Regression `tests/cpu_pipeline.rs::irq_during_multicycle_mul_stall_enters_handler_and_resumes`
  injects IRQ3 during both the initial and continued MUL stalls, then checks the
  handler marker, MUL result, resumed marker, and IRET-enabled state.
- `cargo fmt -- src/cpu_pipeline.rs tests/cpu_pipeline.rs` で自ファイルのみ
  整形 (全体 `cargo fmt --check` は未変更部の差分で失敗するため対象外)。
  検証は `cargo test --test cpu --test cpu_pipeline` (+ bus_memory_map,
  bios_rom)。フル `cargo test` は sqv4 の親側 golden 欠落で失敗する既知 issue。
- エミュレータ既定のパイプライン化 (Extension p rollout):
  `src/cpu_select.rs` に `CpuKind::{Scalar,Pipelined}` (既定 pipeline)、
  `resolve_cpu_kind(CLI > config.yaml cpu.microarchitecture > 既定)`、
  `split_cpu_arg`、`AnyCpu` (Scalar(Cpu)/Pipelined(PipelineCpu) の委譲
  enum) を追加。GUI/monitor/testbench は `AnyCpu` に統一し、スカラー
  `Cpu` は参照のまま。`--cpu scalar|pipeline` (`--cpu=` 形式も可) を
  GUI/monitor/testbench に追加。`config.example.yaml` と README に記録。
  `PipelineCpu` にデバッガ用 API (`decode_at/read_u32/read_u16_be/
  read_debug_mem_u8/return_state_text`) を追加 (表示用 format は
  cpu.rs を触らず複製)。`bus_memory_map` の `impl Iterator` は enum 越境
  できないため `AnyCpu` 側は `Box<dyn Iterator>` で返す。
- tests/wasm_expansion の testbench サブプロセス呼び出しは config 無しで
  動くため、そのままパイプライン既定のスモークになる。実ゲスト検証は
  `tests/pipeline_guest_boot.rs` (tinybasic.bin 起動→banner/OK描画、
  tests/tinybasic.rs と同条件) を常設化。`cargo test --bin cpt32` (GUI
  スキャフォールド 11 件) も通る。lib 単体テストは sqv4 golden 欠落の
  既知 issue で不可 (本件と無関係)。
- TinyBASIC 構造化フロントエンド (`tinybasic.sol` 全面書き換え):
  LEXER (行テキスト→`TB_TOK_BUF` の (type,value) 8B トークン列) +
  PARSER (トークン列→20B AST ノード) + INTERPRETER (AST ツリーウォーク)。
  行テキスト保管・番地 (`TB_LINE_BASE` 等 A-Z/`@(n)`/GOSUB/FOR 域) は旧来通り、
  新規域は TOK 0x001B1000/行内文字列 0x001B2000/行ディレクトリ 0x001C0000
  (lineNo,astRoot の 8B×1024)/AST ヒープ 0x00500000-0x00540000/文字列プール
  0x00540000-0x00580000。RUN 時に全文を lex/parse して行ディレクトリを構築
  し、直接モードも同一 lex/parse/exec 経路 (tb_pc=-1) で実行する。AST を歩く
  `tb_emit_program/stmt/expr` が将来のコンパイラ backend の拡張点 (走査順序が契約)。
  検証は `solc compile`→`asm.py`→`cargo test --test tinybasic
  --test tinybasic_load_disk --test pipeline_guest_boot` +
  `--test strek_micro` + `--test strek_verify` (Star Trek 実ゲームで
  LOAD/FOR/GOSUB/配列/INPUT を網羅、約4分)。`probe_strek_tron` は別マシン絶対
  パス参照の既存環境失敗で本件と無関係。
- 構造化 TB 実装で踏んだ sol の落とし穴 (再発防止):
  1. 条件結合の `and`/`or` 取り違えは即死級: OR 結合に `or` を使うと AND 判定に
     なり、空白スキップも NUL/EOF 停止も function せず字句解析がトークン上限
     (96) まで暴走する。OR→`and`、AND→`or` を機械的に確認すること
     (grep で `eq (or|and)$` 等の全件監査が有効)。
  2. AST ヒープ bump は直接モードのパースより前に初期化すること
     (tb_repl 先頭で `TB_AST_BASE`/`TB_STR_POOL` 設定)。未初期化 (0) のまま
     alloc するとゲスト 0 番地 (プログラム先頭) を破壊する。
  3. LOAD/SAVE の命令コード (IDC 転送の 1/2) と SYS 文コード (7/8) を混同しない。
     `tb_exec_disk` の分岐は `TB_SYS_LOAD` 等の名前付き const で書く。
  4. `tb_store_input_line` は先頭空白を飛ばし、数字なし行は旧実装同様に黙って
     無視する (strek-tb.bas は全行インデント付き。飛ばさないと全行 LINE RANGE)。
  5. `ps_builtin` の ASC 分岐だけが VM/実機とも失敗する現象に遭遇
     (同型の最小再現では正常動作するため solc の if/else 生成 quirk 疑い・未確定)。
     回避策: ネスト if/else をやめ連続 flat-if＋早期 ret (旧 tb_primary と同形)
     に書き換えたら解消。以降この種の分岐は flat-if を優先し、解消しない場合は
     solc の if/else 生成 quirk (上記) を疑うこと。
- sol ゲストの高速単体検証法: 対象関数を正規表現で抜き出し、const/!var を付けて
  `SolVM().run_source` に流す (tools/solc の VM は sparse memory のため絶対番地
  可、halt 不要)。字句→構文→評価の 27 ケースを秒単位で回してから実機テストに
  進むと効率的。`remove_unused_functions` が未到達関数を消すため、到達不能な
  参照があると `unknown word` で落ちる点に注意。
- PeC keyboard hang after LOAD の診断: `EVENT_STATUS` (`0x80040034`) bit0=FIFO nonempty,
  bit1=overflow; `0x03` はFIFOが非空かつoverflow sticky。`MousePosition` は1イベント
  あたりdx/dyの2イベントをFIFOに積むため、マウス移動バーストが256枠を消費する。
  guest `tb_read_line` はキー以外もFIFOからpopする。調査時の一時注入では status=3 を
  scalar/pipeline両方で再現し、FIFO排出後にキー逐次入力→RUN→次コマンドまで成功。
  `CPT32_PEC_TRACE=1` を起動前に設定すると GUI key mapping/send、PeC key FIFO push/pop/
  overflow、高水位、およびIRQC source3 level/enable/pending clearを `[PEC-DBG]` で追跡可。
  現象が続く環境ではCPU pause/fetch停止、またはFIFOが継続的に再充填されるかを別途
  確認する。Backquote HID usage 53 は `tb_key_ascii` がASCII 96 (`) として出力し、
  GUI `KeyCode::Backquote` もusage 53に割当済みなので、日本語半角/全角キーが
  ` とSyntax Errorになるのは現行US-HID変換の既知動作。CPU選択は起動時
  `--cpu scalar|pipeline` で上書き可能。 
- TinyBASIC の LOAD 後入力回帰では、単に `LOADED` を表示するだけでなく、
  LOAD→RUN→ロード済み `INPUT` 文へのキー入力→`PRINT` 出力まで実 CPU で確認する。
  REPL への次コマンド入力と、プログラム実行中の `INPUT` は別経路なので両方を
  個別に検証する。
- `doc/cpt32-architecture.d2` is the architecture diagram source; `doc/cpt32-architecture.svg`
  is its checked-in render embedded by `doc/cpt32-architecture.html`. Regenerate with D2 0.9:
  `d2 --theme=200 doc/cpt32-architecture.d2 doc/cpt32-architecture.svg`. Nested graph references
  must use full IDs (for example `emulator.core.cpu`); an abbreviated nonexistent path creates
  an unintended D2 node and distorts the layout. D2 CLI was introduced with
  `winget install --id Terrastruct.D2 --exact`.
- ドキュメント整理では、`doc/Home.md` と Systems / Hardware / Software / Guides の Obsidian MOC を入口にし、既存仕様ファイルの配置・名前は移動しない。コードや README の相対リンクを保ったまま Wiki 導線を追加できる。リポジトリルートを vault として開く前提で `[[doc/...]]` を使う。
- 英語原文の日本語版は `doc/ja/` 以下（外部の開発 README は隣接する `.ja.md`）に置き、`translation_of` は日本語ノートから原文への実在する相対パスにする。階層が違うと `../` の数も変わるため、リンク検証で実ファイルまで解決する。仕様のコード例・数値・識別子は原文と照合する。
- Pipeline CPU の directed regression は `tests/cpu_pipeline.rs` に置き、タイミング契約は単なる `>=` でなく正確な stall/flush 数を確認する。通常の load-use は1 cycle、MUL/MULH は latency 3 から初回 EX tick を引いた2 stall、DIV/DIVU/MOD は latency 16 から1を引いた15 stall。各多サイクル命令直後に依存する ADD を置くと結果 forwarding も検証できる。branch flush は taken のみ数え、HALT を含む retired instruction 数も確認する。
- Scalar/Pipeline 比較ハーネスは `cargo run --release --bin cpu_bench -- [iterations [samples]]`。同一生成 workload を両CPUで実行し、各 sample の register file / architectural PC / data memory / retired count が一致しなければ失敗する。CPU生成・RAM接続・program load は計測外、実行時間は median、実行順は交互。wall-clock speedup はホスト依存値でありゲスト cycles/CPI と区別する。README と `doc/spec_CPU_p.md` に実行方法を記載。
- ImHex patterns は `tools/imhex/*.hexpat` に配置する。SQV4 は little-endian 16B header と variant-dependent channel fragments、WAX v2 は big-endian header/table と file_offset 上の payload を可視化する。ImHex official Pattern Language では `#pragma endian`, dynamic arrays, struct 内 `@ offset` placement を使用する。検証は `C:\Program Files\ImHex\imhex.exe --pl format INPUT PATTERN OUTPUT.json`。`--pl run` は成功時でも構造を表示しないことがあるため、format JSON の header/blocks/fragments を確認する。
- `examples/test.wax` の文字テーブル payload は `W/S DIST...` から始まり、guest code は文字列を `0x00020000`、`0x00020011`、`0x00020023` などの固定番地で参照する。壊れていた第2 block の `load_address=0x0002002F` を `0x00020000` に直す（file_offset/size は不変）。`tests/wax_sparse_example.rs` は各参照先 byte と sparse hole 不変を確認する。
- asm.py の WAX sparse block 作成では、`.ORG` が現在の PC と一致する場合に `_block_base` を
  更新しない。既に出力済みの byte を含む block の base を動かすと、その後の WAX payload が
  `.bin` と異なる絶対番地に配置される。空 block segment の時だけ base を更新し、`.bin` を
  ゼロ初期化して WAX block を load address に展開した結果との等価性をテストする。
- デバッグ GUI の `Performance` は、`Debug Controls` の `Cycles / frame` と同じ実行予算で CPU 使用率を正規化する。
  `PerformanceStats` の IPC は VSync 間の retired instructions / running cycles。実行サイクルが0なら IPC は0として表示する。
- TinyBASIC の実CPU回帰 `tb_repl_accepts_input_after_list_and_run` は、LIST実行中の入力と、LIST/RUN完了後の次入力を Scalar/Pipeline で確認する。Windows GUI（default Pipeline）も `CPT32_PEC_TRACE=1` で実キー→PeC FIFO→IRQCを追い、`LIST`後と長時間 `RUN`（`print 2^10000`）後に `print 42` が表示されることを確認。最初の試行は残留Backquoteで構文エラーになったため、Enterで破棄してから再試行する。
- TinyBASIC の `LOAD` は最大128 sectors（64 KiB）を同期DMA後に `tb_load_disk_lines` で全走査する。処理中はREPLの `tb_read_line` に戻らずキー表示しないが、PeC FIFOにイベントは蓄積する。`-O speed`生成コードのPipeline回帰はLOAD完了後にCPUが実際にHALTするのを待ち、その後の `print 42` を確認（約105秒）。Debug GUIのWAX起動でも64 KiB LOAD後に `LOADED` を実観測（約100秒）。
- キーボードで HALT 中の CPU が起きない件の原因は debug GUI の一時停止だった。`running==false` だと `execute_cpu` が `cpu.run` を呼ばず `tick`→`tick_devices`→`refresh_device_irqs` が走らないので IRQC source3 が見られず IRQ3 が立たず、`tb_read_line` の `halt` から復帰しない。pec-input スレッドは独立に積むため FIFO は 256 で overflow。`CPU running:` は `cpu.is_running()` で REPL 待機と停止の双方が false とあいまいだったため、`src/main.rs` の Debug Controls に `Emulation: RUNNING/PAUSED` 行を追加し `doc/spec.md` に明記。ハーネス（cpu.run 呼ぶ）では 12/12・300キーでも wake は正常で、pause 再現 probe のみが症状（命令0・IRQ handler 不動作・FIFO overflow）を完全再現した。
- TinyBASIC LOAD 高速化（`tinybasic.sol` + `tools/solc/sol_compiler.py`、一時 probe で 64KiB=20,101,161 命令(307/byte) → 15,051,513 で **-25%**）。(1) `tb_load_disk_lines` を単一パス streaming に変更し、`TB_INPUT_BASE` へバッファして `tb_store_input_line`→`tb_copy_line` で再走査・再コピーしていたのを、行番号を inline パースし body を line slot へ直接書く方式に置換（-20%）。line 終端確定は `tb_finish_load_line(state,line,addr,idx)`、`count` は旧バッファの `TB_MAX_TEXT`(120) 上限を模倣し line slot は `TB_LINE_STRIDE`(128) なので body は slot を溢れない。REPL 経路の `tb_store_input_line`/`tb_copy_line` は不変。(2) solc `_emit_load_imm32` が `LDIH`/`LDIL` の前に入れていた `ADDI reg,R0,0` を削除（`LDIH` は上位16を書いて下位保持、`LDIL` は下位16を書いて上位保持 → 両者で全32bitが決まり旧値は `LDIL` で潰れる。scalar/pipeline 双方で確認、既に IRQ ベクタ生成は無ゼロ化で `LDIH`/`LDIL` を使用）(+5%)。
- `tests/tb_load_live.rs` は**元コードでも 4/7 が失敗する**（`list_and_run`×2, `alive_after_load`×2）。フレーム数予算依存で flaky なので、LOAD 正しさの検証は wait-for-condition 方式（`load`→`LOADED` 待ち→`list`→画面に `10 print 99` 出現を待つ）が確実。`tools/solc/test_sol_language_server.py::test_lsp_accepts_assignment_to_function_arguments` は LSP の Content-Length framing で元々失敗する（codegen 変更と無関係）。`tools/vscode-sol-syntax/server/*.py` は `tools/solc/` より古い別系（`org` 引数や IRQ dispatcher が無い）で、非意味論的 compiler 変更の同期は不要。
- DSFSは仕様と host ツール・テストを整備（`doc/spec_DSFS.md`、日本語・`doc/spec_IDC.md` 風、索引は `doc/Software.md`、ツールとテストは下記）。契約の要点: (1) block=512B=IDC LBA、整数は big-endian（`doc/spec_CPU.md` Endianness に合わせた。`spec_COW.md` の little-endian は host 側 sidecar 専用で参考にしない）。(2) レイアウトは block0 ブート（byte510に必須 `0x55 AA`、コード領域510BはFS非解釈）、block1 スーパーブロック、block2〜 ビットマップ（`bitmap_blocks=ceil(total_blocks/4096)`）、続く16 blocks ディレクトリ（256×32B）、残りデータ領域。式と superblock 記録値の一致を mount 検証で強制する。(3) 連続 first-fit 割り当て＋ビットマップのみなので、ファイルは連続領域1つ。断片化で総空きがあっても失敗しうる（仕様上の既知の制限）。(4) crash 後も mount 可能を保つ更新順序: 「ビット占有→データ→エントリ占有」「エントリ解放→ビット解放」の逆は禁止。異常終了で残るのは主張されない1bit（leak）だけで、これは mount 拒否しない。逆に used 領域の0bit・領域重複・小文字name・reserved非0・padding bit は mount 拒否（strict、WAX 流儀）。(5) 空きエントリは32B全0、name はドットbyteなしの11B space-padding 大文字（A-Z0-9_-のみ）、`size=0` なら `start=0`、最終blockの size 以降は0埋めが canonical。
- DSFS ツールは `tools/dsfs/dsfs.py`（Python 標準ライブラリのみ、argparse は tools/wax/wax.py 流儀）で mkfs/info/ls/put/get/cat/rm/rename/verify の9コマンド。検証は `python3 -m pytest tools/dsfs/test_dsfs.py`（14件、CLI部分は実 subprocess）。実装契約: 書き込み系は保存前に §7 check() を通し、更新は §8 順序（ビット占有→データ+末尾0埋め→エントリ更新→旧領域の解放、delete はエントリ全0→ビット解放）をコード順で守る。first-fit 連続割り当てで詰まると "no contiguous run" エラー（総空きが十分でも断片化では失敗する）。メタデータビット0化（§7-10）と padding bit（§7-11）はツール実装時に仕様へ追記した項目。leak は ls/info/verify が件数表示しエラーにしない。verify は違反を stdout に列挙して終了1、他エラーは stderr の `dsfs: ...` で終了1。Windows のこの開発環境では pytest 内の subprocess.run に stdin=subprocess.DEVNULL が必須（無いと WinError 6 で親の stdin ハンドル複製に失敗）。
- DSFS の sol ゲストライブラリは `sol/cpt32/idc.sol`（DMAC ch0 polled 転送、戻り値 0/1/2 = ok/DMAC err/IDC err、W1C クリアと busy polling を毎回実施）と `sol/cpt32/dsfs.sol`（mount は LBA1 読込＋§7 検証＋geometry キャッシュ、write は §8 順序・最終 sector in-place 0埋め・timestamp は `dsfs_set_time`（既定0）、buffer 契約は dir 8192B / bitmap bitmap_blocks×512B）。回帰は `tools/dsfs/dsfs_smoke.sol`（冒頭にビルド手順）→ src32_testbench と `cargo test --test dsfs_guest`（画像を Rust 側で生成し R1=0 ＋オンデマンド byte 級 assert、`dsfs_guest.bin` は `*.bin` gitignore なのでローカルビルド必須）。**sol の truth 代数の罠（2回踏んだ）**: 比較は sol-truth(0=true)なので複数述語の論理 AND は `or`、論理 OR は `and`（bitwise）で表現する。通常の and/or 感覚で書くと条件が逆になり、write_file の reuse/release gate が壊れて guest 書き込み画像が host 側 strict 検証で leak 違反になった。`and`/`or` を bitwise マスク以外で使ったら必ず疑う。solc のバグも1件修正: while が関数先頭だとループラベルが prologue より前に出て frame save を毎 iteration 再実行し saved frame pointer を破壊する → `sol_compiler.py` で function label＋prologue を先に emit（VS Code 拡張 server コピー同期、`test_sol_compiler.py::test_while_at_function_start_emits_prologue_before_loop_label` で回帰）。guest の magic 定数タイプミス（"DSFS"=0x44534653 の末尾を 0x46 に誤る）は R1 エラー値での段階 bisect（identify→mount→…）が最も速い診断だった。
- `sol/solstd/softfloat.sol` に `sf32_toint`（binary32 ビット列→int32、0方向へ切り捨て。NaN は 0、`+∞` と `0x7FFFFFFF` 超は `0x7FFFFFFF`、`-∞` と `-0x80000000` 未満は `0x80000000`、`|x|<1` は 0）と `sf32_tofloat`（int32→最近接 binary32、近接偶数丸め。`0x80000000` は厳密に `0xCF000000`、`0x01000003` は同点なので偶数側 `0x4B800002`）を追加した。丸めは既存 `sf32_pack` の3ビット丸めにそのまま乗せ、`e>26` の桁落ちは `sf32_shift_jam` で bit0 に jam すれば近接偶数丸めになる（追加の丸め処理は不要）。契約と例外時の挙動はファイル先頭のコメントと `doc/spec_sol.md` の「## 2. 浮動小数点」に記載。回帰は `tools/solc/test_softfloat.py::test_softfloat_integer_conversions`（34ケース。期待値は Python `struct` の IEEE 参照実装と突き合わせ済み）。
- src32_testbench の落とし穴3点。(1) リポジトリ root の `config.yaml` は `bios.image` を設定しているため、`src32_testbench PROGRAM.bin` は BIOS から起動して RAM 0 のプログラムを実行しない。この状態では `--expect-mem-u8` も R1 期待値も常に 0 を返し検証が無効化される（`tools/solc/bench_solc.py` の R1 期待値が現在失敗するのはこれが原因で、softfloat の変更とは無関係）。RAM から直接実行するには BIOS セクションの無い `config.yaml` を置いたディレクトリで testbench を実行する。(2) `ST` は `write_u32_be` のビッグエンディアンなので、`--expect-mem-u8` で 32bit store の結果を見るときはアドレス+3 のバイトを見る。(3) BIOS 無しで実行すればデータスタック先頭は R1 に残る（`5 3 sub` → R1=2、`ret` した戻り値も同じ）。
- sol の `eq`/`neq` は 0=真の述語で、`neq` は等しくないとき 0、等しいとき 1 を返す。不一致件数を数えるには `1 <expr> <want> neq sub` のように 1 から述語を引く（`fails add >fails` で加算）。

## sol ライブラリの名前空間 (2026-10-17)

- `!namespace name -` ... `!end` と `!include "file" as ns` は `tools/solc/sol_vm.py` の
  トークン展開段で実装されている。関数だけでなく `!const` / `!var` も `(namespace, name)` で
  登録されるため、ライブラリの定数・変数も `ns.NAME` で読み、代入は `>ns.NAME` と書く。
  非修飾名は「現在の名前空間 → グローバル」の順に解決し、`ns.` 修飾名はその名前空間だけを
  見る。マクロ (`!define`) はテキスト置換で名前空間の対象外（常にグローバル）。名前空間は
  入れ子にできず、`!namespace name` 末尾の `-` は省略可能な区切りトークンである。
- `sol/solstd/*.sol` と `sol/cpt32/*.sol` はファイル全体を1つの名前空間
  (`array` / `prng_software` / `softfloat` / `print` / `pcg` / `colors` / `irqc` / `pec_rng` /
  `idc` / `dsfs` / `vpu`) で宣言する。ライブラリ内の相互呼び出しは非修飾のままで動く。
  呼び出し側 (`tinybasic.sol`, `3d_cube.sol`, `mouse_cursor.sol`, `vpu_*.sol`, `sol/bios.sol`,
  `tools/dsfs/dsfs_smoke.sol`, テスト) は `pcg.prnscr` / `softfloat.sf32_add` / `idc.idc_read`
  のような修飾名に移行済み。新しいライブラリ関数・定数を呼ぶときは必ず `名前空間.` を付ける。
- 実装の要点: 内部関数名が `__solc_ns_<len(ns)>_<ns>_<len(name)>_<name>` に改名されるため
  配置アドレスが動く。変更後は `assets/bios.rom` と各 `.bin` を再ビルドする（BIOS は
  `solc.py compile --org 0xFF000000 --read-only-data-base 0xFF008000 sol/bios.sol` →
  `asm.py --strip -o assets/bios.rom`; 33101 bytes でサイズ不変）。
- 回帰: `tools/solc/test_sol_vm.py` の namespace テスト（修飾/非修飾の解決順、名前空間内の
  定数・変数の参照と代入、修飾名の未知関数・未定義変数の拒否、`irqc` 経由の
  `irqc_irq_number`）。実機は `cargo test --test tinybasic`(5) / `--test dsfs_guest`(1) /
  `--test bios_rom`(8)、ツールチェインは `python3 -m pytest tools/solc -q`
  (149 passed、失敗1件は既知の LSP Content-Length framing)。
- `tools/vscode-sol-syntax/server/sol_vm.py` は拡張同梱コピー。名前空間の意味論は同期済み
  だが、`!stack_top` / `!var_base` / `!rodata_base` や IRQ dispatcher などは元から分岐した
  古い系列なので、無関係な差分を消さないこと。
## CPT32 DOS (dos.sol) と IDC ライブラリの修正 (2026-10-17)

- `dos.sol`（リポジトリ root、`tinybasic.sol` と同じゲストプログラム）は DSFS/FDD の
  シェル。DIR/TYPE/NEW/REN/DEL/COPY/EDIT/HEX/FORMAT/DRV/FREE/CLS/HELP/EXIT を実装し、
  文字は PCG コンソール、入力は PeC FIFO を IRQC source 3 で待つ（`fn irq3` で HALT →
  起床 → FIFO ドレイン）。ビルドは `solc.py compile dos.sol -o dos.sol.a` →
  `asm.py dos.sol.a -o dos.bin`（`*.bin` と `*sol.a` は gitignore 済み）。回帰は
  `cargo test --test dos_smoke`（3件 = 2ドライブ操作／no-media／write-protected。実機の PeC
  キー入力、PCG テキストプレーンの needle 待ち、書き込み画像のバイト検査で約95秒）。
  README に「CPT32 DOS（DSFS シェル）」節を追加した。
- ゲスト GUI のテストは `tests/dos_smoke.rs` の流儀が使える: Rust 側で DSFS 画像と
  `bios` 無しの `config.yaml` を作り、Bus に IDC を接続して PeC FIFO へキーを押し込み、
  PCG テキストプレーンを needle で待つ。`src32_testbench` にはキー入力が無いので、
  キーボードを伴う検証はこのハーネスで行う。
- `sol/cpt32/idc.sol` のエラー検査が壊れていた: `IDC_STATUS IDC_BIT_ERROR and 0 neq` の
  ように `ld` が抜けており、`idc_identify` は no-media でも -1 を返さず、`idc_read`/
  `idc_write` の戻り値 1/2（DMAC err / IDC err）も絶対に返らなかった。5箇所へ `ld` を
  補い、コメントどおりの契約に直した（no-media の identify は -1、no-media の read は
  DMAC エラーが先に立つので 1）。idc.sol を含む成果物（`dos.bin`, `dsfs_guest.bin`）は
  再ビルドが必要。回帰は `dos_smoke` の write-protected テスト（`read_only: true` の
  スロットで `NEW` → "CREATE FAILED" を待ち、画像にエントリが増えていないことを確認）。
  修正前は書き込み失敗が成功として報告され、このテストは timeout する（実測で
  before/after を確認）。
- 8.3 名は必ず**空白 padding**（`doc/spec_DSFS.md` §6.2）。`dos_name_build` が
  `dos_memzero` で埋めていたため "EMPTY\0\0\0TXT" になり、`dos_name_text` が NUL で
  切れて "CREATED EMPTY" と表示された（host の `dsfs.py verify` も拒否する領域）。
  `dos_memfill` で 32 を書くよう修正。
- `dos_strcpy` は**コピーした長さ**を返す（新しいオフセットではない）。エディタの
  splice で `... dos_strcpy >o` と足し込みを忘れて dos_text_len が 1 バイト足りず、
  行末の改行が NUL 化されず行が壊れた。`o add >o` と書くこと。
- ラインエディタの splice は mode 0=置換, 1=挿入, 2=削除。挿入・置換テキストへ `\n` を
  付ける位置と、旧行を捨てる条件 `i index eq mode 0 eq or`（mode 0 しか落ちない）の
  両方が誤っており、削除が効かず挿入が行を壊していた。現在は「i == index かつ
  mode != 1 のときだけ旧行を捨てる」= `i index neq mode 1 eq and` に統一。
- エディタのエラー・保存メッセージは `dos_msg_err` / `dos_msg_text` で `DOS_MSGBUF` に
  溜め、各エディタの再描画（`dos_cls` を含む）の直後に出す。直接 `dos_puts` すると
  同じループ反復の再描画で消えて人間には読めない（テストの `wait_text` は frame 単位で
  拾えることがあるため、この不具合はテストでは見つかりにくい）。
- sol の 0=真 述語では「述語の AND は `or`、OR は `and`」で、`if` は 0 のとき then を
  実行する。この取り違えが上記 splice の削除条件の原因だった。
- 検証: `cargo test --test dos_smoke --test dsfs_guest`（3件）、
  `python3 -m pytest tools/dsfs/test_dsfs.py`（14件）、さらにゲストが書いた画像を
  `tools/dsfs/dsfs.py verify/info/ls` に通して §7/§6.2 を host 側から確認した
  （disk0 = 編集後 2 file/0 leak、disk1 = FORMAT + COPY で 1 file/0 leak）。
  DOS shell に `MODE 40` / `MODE 80` を追加。PCG の `col40` / `col80` を切り替えてから画面を消去し、選択幅を表示する。REPL prompt は drive number の後に `> `（末尾スペースあり）。回帰テストは両モード切替と prompt の `0> ` を PCG plane で確認。

## sol VS Code 拡張の namespace 対応とアウトライン (2026-10-17)
- 拡張は `tools/vscode-sol-syntax/`。LSP は `server/sol_language_server.py`（同梱コピー）で、
  正式版は `tools/solc/sol_language_server.py`。**この2つはバイト単位で一致させる**（AGENTS.md）。
  同梱コピーは以前 text モードの `sys.stdout.write` で Content-Length を送っており、Windows の
  CRLF 変換で `\r\r\n` になりフレーミングが壊れていた（`tools/solc/test_sol_language_server.py`
  の argument-assignment テストがこれで落ちていた）。`sys.stdout.buffer` + `len(raw.encode())` に
  統一すると通る。同梱 `server/sol_vm.py` は org/irq/stack_top を持たない**古い系列**なので、
  solc 側の sol_vm.py を丸ごとコピーしないこと（必要な修正だけ両方に入れる）。
- アウトライン（DocumentSymbol）はサーバ側だけで完結する。initialize の capabilities に
  `documentSymbolProvider: True` を返せば vscode-languageclient が自動でプロバイダを登録する
  （`src/extension.ts` の変更・tsc 再ビルドは不要）。クライアントは
  `hierarchicalDocumentSymbolSupport: true` を送るので `DocumentSymbol[]`（range/selectionRange/
  name/kind/children）を返す。`range` は `selectionRange` を包含している必要がある。
- 実装は `Server.symbols()`：tokenize は入力途中（未終端文字列・`!asm` に `!end` が無い）で
  例外を投げるため使わず、行ベースのスキャナにした。`blank_literals()` で文字列と `#` コメントを
  空白化してから正規表現を当てる（位置は保たれる）。`!asm`/`!data`/`!db` ブロックは `!end` まで
  読み飛ばす（asm 内の `;` はコメントなので、これを文末と誤認すると関数範囲が壊れる）。
  関数は `;` で終端（spec_sol.md のとおり）、ローカル変数は関数の子、名前空間は
  `!namespace`〜`!end` の子として入れ子にする。kind は namespace=3 / function=12 /
  variable=13 / constant=14（`!define` マクロも constant）。`!end` が無い名前空間は末尾までに補正。
- セマンティックトークン：legend の**末尾**に "namespace" を追加（既存 index を動かさない）し、
  `package.json` の `semanticTokenScopes` で `entity.name.namespace.sol` に割り当てる（未割当の型は
  型名そのものがスコープ扱いになる）。`!namespace X` の別名、`!include "f" as X` の別名、
  修飾参照 `X.Y` は **2 トークン**に分割して `X`=namespace / `Y`=function|constant|variable
  （同一文書内の完全一致がある場合のみ、他は identifier）を出す。`;`/`(`/`)` にはトークンを出さない。
  さらに `RawAssemblyToken` を素通しすると dataclass の repr が巨大な identifier トークンとして
  出ていたので、`!asm` の 4 文字 directive を出して本体は捨てる。
- `sol_vm.tokenize` の桁位置を2箇所修正（solc と同梱の両方）：(1) 開きクォートで `advance(ch)` が
  無く、同じ行の文字列リテラル以降のトークンが文字列1個につき1桁ずれる、(2) `;`/`(`/`)` が
  `append_token` 前の `start_token()` を欠き、直前トークンの開始位置を引き継いでいた。
  回帰: `tools/solc/test_sol_vm.py::test_token_locations_follow_string_literals_and_punctuation`。
  これは LSP のセマンティックトークン位置に直結する（`!include` 別名の桁ずれとして露見した）。
- TextMate 文法（`syntaxes/sol.tmLanguage.json`）に `#namespaces` を追加し、`!namespace NAME -`、
  `!include "..." as NAME`、`X.Y` 修飾子を `entity.name.namespace.sol` にする。存在しない
  `#labels`/`#variables`/`#identifiers` の include は削除した。なお `#directives` には
  コンパイラが実装していない `!required_return_stack_size` / `!force_return_stack_size` が
  残っている（未修正）。
- 検証の型：`python3 -m pytest tools/solc/test_sol_language_server.py tools/solc/test_sol_vm.py` と
  tools/solc 全体（151 件）、`npm run compile`、`npx vsce package --no-dependencies`（15 ファイル）。
  GUI 無しでも次の2つのプローブで実挙動を確認できる（node_modules 直下に置いて実行し、終わったら削除）：
  (1) `vscode-languageserver-protocol` の `DocumentSymbol.is()` と
  `vscode-languageclient/lib/common/documentSymbol.js` から読んだ SupportedSymbolKinds で
  サーバ応答を検証（`vscode` モジュールは拡張ホスト外では require できないので
  documentSymbol.js の import は失敗する）、(2) `npm install --no-save vscode-textmate
  vscode-oniguruma`（package-lock.json は変わらない）で実際に `loadGrammar("source.sol")` して
  スコープを検証。後者で `!namespace`/別名/`X.Y`/`!asm` ブロック/文字列内の `fn` が期待どおりに
  トークン化されることを確認済み。ファイル改行は CHANGELOG.md が CRLF、CHANGELOG.ja.md が LF。
- 既知の制限（セマンティックトークンのみ）：別ファイル（`!include ... as ns` の中身）に定義された
  `ns.member` は member 側を function/constant/variable に解決しない（`tokens()` は同文書内の
  完全一致しか見ないため）。prefix だけ namespace 色になる。**ナビゲーション機能は 2026-10-17 の
  追記で include closure を自前展開して解決できるようになった**ので、この制限はトークン色に限る。
- 検証済みの制限：実 VS Code のアウトライン表示そのもの（GUI）は未確認。プロトコルとクライアント
  変換コードの契約までを確認した。

## tinybasic.sol / dos.sol の namespace 整理 (2026-10-17)
- 対象: リポジトリ直下のゲスト 2 本。関数だけを sol の名前空間へ移し、`!const`/`!var` は global のまま
  残した（解決規則は「現在の namespace → global」なので、global の定数・変数を参照する側は無変更）。
- 名前空間の境界は既存のセクションコメント（`# ---- LEXER ----`）そのもの。
  tinybasic: line / vars / con / kbd / lex / ast / parse / prog / eval / exec / emit / run / disk、
  dos: con / kbd / names / disk / cmds / edit / hex / shell。
- **`irq3` と `main` は必ず global に残す**。solc は global の `irqN` にしかディスパッチャを生成せず
  （sol_vm.py の `irq_match = ... if current_namespace is None else None`）、エントリは末尾の `main` を
  global 解決で呼ぶため。namespace 内に入れると起動も IRQ も無言で壊れる。
- 越境呼び出しだけ `ns.` を付ける（同一 namespace 内は無修飾のまま）。tinybasic 220 箇所 / dos 388 箇所。
- 検証（重要）: 生成した `tinybasic.bin` / `dos.bin` は refactor 前と **md5 が一致（バイト単位で同一）**。
  単一ファイルのゲストでは namespace は内部ラベル `__solc_ns_N_<ns>_M_<name>` を変えるだけで、
  関数の順序と本体が同じなら配置は変わらないため。すなわち「namespace 整理はコード生成に影響しない」
  ことの証明になり、既存 .bin に対するテスト結果がそのまま使える。コンパイラ警告（DCE の unused 集合）
  も前後で完全一致した。
- 手順の落とし穴: (1) tokenize() で fn 定義範囲を取り、行範囲で帰属を決め、`ns.` 挿入は必ず
  「修飾（列位置挿入）→ namespace 行挿入（行 index 降順）」の順に行う。逆順だと列位置がずれる。
  (2) `.bin` を作り直すとき、ソースをサブディレクトリへコピーすると `!include "sol/cpt32/..."` が
  そのファイル基準で解決されて失敗する。バックアップ検証はリポジトリ root に置く。
  (3) `!keepfn` は namespace 内の関数には修飾名（`!keepfn a.one`）が必要。
  (4) `!end` は namespace / raw block の終端で、`if`/`while` の終端は `end`。namespace の中に
  `!asm`/`!data` があるとその `!end` が namespace を閉じてしまう（今回の 2 ファイルには無い）。
  (5) リファクタ差分の監査方法: 現行ファイルから `!namespace`/`!end` 行を除去した本文が、
  ベースラインへ同じ修飾を再適用した結果と一致することを独立スクリプトで確認すると、
  「プレフィックス挿入以外の変更がない」ことを機械的に証明できる。
- 既知の失敗: `strek_smoke::probe_strek_tron` は `/Users/src3453/remote/Tiny-Basic/samples/strek-tb.bas`
  という絶対パスを読むため、この環境では必ず unwrap で落ちる（既存・今回と無関係）。
- 検証済みテスト（baseline = namespace 整理後も同じ）: tinybasic 5 / dos_smoke 3 / pipeline_guest_boot 1 /
  tinybasic_load_disk 1 / strek_smoke 2(+1 既知失敗) / strek_micro 5 / strek_verify 1 / tb_load_live 7。
- .sol は LF、MEMORY.md は CRLF。バイナリは `*.bin`、中間の `*sol.a` は .gitignore 済み。
- ウィンドウにフォーカスを合わせると ` が勝手に入力される件 (2026-10-17 修正): 原因は winit 0.30 の
  合成キーイベント。Windows バックエンドは `WM_SETFOCUS` で「その時点で OS が押下中と報告する全キー」の
  keydown を `is_synthetic: true` で合成する (`keyboard.rs::synthesize_kbd_state`)。**日本語 IME 有効時は
  `GetAsyncKeyState` が IME 自身の仮想キー `VK_DBE_ALPHANUMERIC(0xF0)` / `VK_DBE_SBCSCHAR(0xF3)` /
  `VK_DBE_ROMAN(0xF5)` を押下中として返す** (トグル状態も同様)。winit は `MapVirtualKeyExW(vk,
  MAPVK_VK_TO_VSC_EX)` でスキャンコードへ写像するので、US 相当の写像では 0xF0→0x3A (CapsLock)、
  0xF3→0x29 (Backquote) となり、ゲスト (PeC FIFO) へ usage 0x39 / 0x35 の幽霊押下が届いていた
  (dos.sol は 53→'`' をエコーし、起動直後のプロンプトが `0>` の後に ` を表示していた)。修正は
  `src/main.rs` の `guest_key_input` が `is_synthetic` を入力に変換しないこと + KeyboardInput 分岐で
  合成スナップショットを無視すること (Escape / F1 マウスキャプチャも合成では発火しない)。
  検証: `CPT32_PEC_TRACE=1` で「GUI key focus snapshot ignored physical=Backquote」+ FIFO push 無し、
  実キー (VK_A) は usage 0x04 が push されゲストが 'a' をエコー。dos.sol / TinyBASIC の両方で
  フォーカス後のプロンプトに ` が無いことをスクリーンショットで確認。回帰は
  `cargo test --bin cpt32 focus_key_snapshot_is_not_guest_input`。
  注意: ユーザーが半角/全角キーを **実際に押した** 場合は今も lparam の scancode 0x29 から usage 0x35
  ('`') が届く (US-HID 変換の既知動作、別件)。また `--bin cpt32` のテスト 2 件
  (`package_guest_builder_loads_program_at_address_zero`, `package_drop_replaces_guest_transactionally`)
  はリポジトリルートの config.yaml が `bios.image` を持つため program が 0 番地にロードされず失敗する
  (既知・今回と無関係。`git stash` でも同様に失敗することを確認)。
  デバッグ GUI / 実機検証の手順: ゲストを直接動かすには BIOS 節の無い config.yaml を持つディレクトリで
  起動する。**フォント未指定だと PCG テキストが描画されず画面が空白になる**ので、その config には
  `pcg.font` (例 `assets/cga.f08`) を必ず入れること (空白画面はゲスト不動作と誤認しやすい)。

## sol LSP の宣言ナビゲーション (2026-10-17)
- 対象: `tools/solc/sol_language_server.py`（正式）と `tools/vscode-sol-syntax/server/sol_language_server.py`
  （同梱）。**両者はバイト単位で一致させる**（cp して cmp）。同梱系列の `sol_vm.py` は古いが
  `LABEL_RE` / `QUALIFIED_RE` / `_decode_string_literal` / `RawAssemblyToken` は揃っているので、
  サーバ側の新規 import はその 4 つに留めれば両方で動く。`IncludeMarker` など新規型はサーバ内で定義する。
- 追加した capability とハンドラ: `definitionProvider` / `declarationProvider` /
  `referencesProvider` / `hoverProvider` / `documentHighlightProvider` / `workspaceSymbolProvider`。
  vscode-languageclient は capability から自動登録するので `src/extension.ts` も `package.json` も
  変更不要（DocumentSymbol のときと同じ）。
- 解決モデル: `tokenize()`（同梱系列でも位置修正済み）で include closure を**自前展開**し
  （`!include "f" as ns` は IncludeMarker で前後を挟む＝コンパイラと同じ scope 付与。相対パスは
  インクルード元ファイルのディレクトリ基準）、単一の flat ストリームを 2 パスで走査する。
  pass1=宣言（`fn` + args + 関数 body 内の `local`、`!const`/`!var`/`!define`、`!namespace`、別名）、
  pass2=参照。シンボル表はコンパイラと同じ `(namespace, name)` キーで、`!define` だけ `(None, name)`。
  解決順は **locals → args → `(現在ns,name)` → `(None,name)` → namespace 名**（locals が args に勝つのは
  sol_vm のパス順がそうなっているため）。`ns.member` は `(ns, member)` のみ。
- 参照の位置は必ず「名前そのもの」を指すこと: `>name` の store は `+1`、`ns.member` は
  qualifier と member の**2 参照に分割**して member を `len(qualifier)+1` ずらす
  （`Ref(offset=...)`）。ずれると declaration/definition が黙って None を返す。
- 宣言位置の参照は pass2 から除外する（`index.decl_positions`）。除外しないと宣言名が「参照」として
  二重に数えられ、references が 1 件多く出る。カーソルが宣言上にある場合は `find_decl()` で
  自身を返す（Go to Definition が宣言上で無反応にならない）。
- `textDocument/references` / `workspace/symbol` は `initialize` の `rootUri` / `workspaceFolders` から
  プロジェクトを走査する（`target` / `node_modules` / `__pycache__` / `.` 始まりは除外、上限 4000 ファイル・
  2000 件）。参照は「各ファイル自身の closure で解決してから (uri,line,char) で dedupe」するので、
  ライブラリ側の内部参照とそれを include するゲスト側の参照の両方が一度ずつ出る。
- パフォーマンス: ディスクの tokenize は `(st_mtime_ns, st_size)` キーでキャッシュ。実リポジトリ
  （24 ファイル）で references / workspace symbols が 0.1 秒台。closure はリクエスト毎に再走査する
  （トークンはキャッシュ済みなので軽い）。
- イン・フライトのバッファ（未終端文字列 / `!end` の無い `!asm`）は `tokenize` が例外を投げる。
  ナビゲーションは `tokens = []` に縮退して null を返す。`semanticTokens/full` も同じ入力で
  空データを返すようにした（以前は例外がハンドラを貫き、`main()` が id 無しのエラーを返していた）。
  加えて `main()` の例外ハンドラは**リクエストの id を付けて**返すようにした（`{"id":None,...}` だと
  クライアントは自分のリクエストの応答を永久に待ち、テスト用 Session もハングする。実際にこれで
  テストが FAILED ではなく**ハング**して気づいた）。
- 落とし穴:
  (1) `Server.path()` は `urlparse(uri).path` をそのまま返すため Windows では `/C:/...` になる。
      ナビゲーション用に `fs_path()`（先頭 `/` を除去）と `uri_for_path()` を追加した。既存の
      `path()` は `compile_program` の診断に使い続けているため**触っていない**（そのため
      `!include` 失敗時の診断メッセージは今も `C:\C:\...` と二重ドライブに見える。既知・未修正）。
  (2) テストの `Session.__exit__` が `return True` だと `with` 内の AssertionError を**握り潰して
      PASSED になる**。実際にこれで 4 テストが偽陽性 PASS した。`return None` にして
      「同梱コピー未同期なら落ちる」ことを確認してから cp する、が正しい手順（偽陽性の検出手段）。
  (3) `find_references` でループ変数の index を使い回すと、後続ファイルで root の index を
      上書きして誤った closure を検索する。
- 検証の型: `python3 -m pytest tools/solc -q`（156 件全通過。`tools/solc/test_sol_language_server.py`
  に 4 テスト追加: cross-file / args・locals・highlight / namespace シャドウ＋壊れたバッファ /
  workspace symbols。**同梱サーバも同じテストで回す**）。実リポジトリ検証: tinybasic.sol の
  `pcg.pcg_cur_x` で references が 18 件（`grep -ro pcg_cur_x` の内訳 dos3 / pcg9 / test3 /
  tinybasic3 と完全一致）、`pcg.putcscr` の hover が `fn putcscr (char)` + `sol/cpt32/pcg.sol:102:4`、
  workspace/symbol "prnscr" が containerName `pcg`。プローブ（node_modules 直下に置いて実行後に削除）で
  `vscode-languageserver-protocol` の `Location.is()` / `Hover.is()` に定義・宣言・参照・ホバーを
  通し、highlight / SymbolInformation は型ガードが無い版なので shape を手動検証。`npm run compile` と
  `npx vsce package --no-dependencies`（15 ファイル）も通過。
## sol 名前空間の入れ子（2026-10-17）

- `!namespace` ブロックは入れ子にできる（実装: 名前空間名のスタック）。完全名は `outer.inner` のように `.` で連結し、
  `!end` は直近に開始したブロックを 1 つだけ閉じる。同じ完全名は複数ブロックに分けて再宣言でき、1 つの名前空間として扱う。
- 非修飾名の解決順: 現在の名前空間 → 外側の名前空間（内→外）→ グローバル。修飾名 `ns1.ns2.name` は
  その完全名が指す名前空間のみを検索し、外側へフォールバックしない（`inner.target` は `outer.target` を拾わない）。
  名前空間ブロックの内側で `!include "f" as ns` すると、その名前空間の内側に入れ子（`outer.ns.*`）になる。
- 実装場所: `tools/solc/sol_vm.py` と同梱 `tools/vscode-sol-syntax/server/sol_vm.py` の両方に
  `split_qualified_name()` / `namespace_chain()` を追加し、`resolve_function_name` / `resolve_symbol` を
  チェーン検索へ、`QUALIFIED_RE` を複数ドット可（`(?:\.[A-Za-z_]\w*)*`）へ、`internal_function_name` を
  コンポーネント毎の `len_` 連結（`__solc_ns_1_a_1_b_1_f_f`）へ変更。単一階層では従来と同一文字列に
  なるようにしてあるので、既存ゲストの `.a` はバイト一致（tinybasic.sol / dos.sol で HEAD と cmp 済み）。
- LSP（canonical `tools/solc/sol_language_server.py` と同梱コピーは **byte-identical を維持**）も
  `scan` の pass1/pass2 と `tokens()`、`symbols()` を名前空間スタック化。`Index.resolve()` は
  ドット付き名を「namespace 完全名 → 修飾シンボル」の順に引き、非修飾名は `namespace_chain` で検索する。
  修飾名の参照は**各プレフィックスごとに Ref を出す**（`a.b.f` → `a`, `a.b`, `f`）。`find_ref` は挿入順の
  最初の一致を返すので短いプレフィックスから追加すると、各コンポーネントのクリックがその名前空間の宣言に飛ぶ。
  namespace の Decl は name=局所名・key/containerName=完全名。
- 検証: `python -m pytest tools/solc -q` 全通過（vm に 7 件、LSP に 2 件追加）。入れ子ゲストを
  `solc.py compile` → `asm.py` → `src32_testbench.exe`（bios 無し config の一時ディレクトリ）で
  `-O size|speed` × `--cpu scalar|pipeline` の 4 通り PASS（R1=45、入れ子 `!var` の ST バイトも確認）。
- 落とし穴: `!end` の閉じ忘れエラーは残っているスタックの完全名を出す（`!namespace a - !namespace b -` に
  `!end` 1 個 → `unterminated namespace: a`）。テストで「a.b」を期待しないこと。

## DOS 表示幅切替とプロンプト表記 (2026-10-17)
- `dos.sol` の `MODE 40` / `MODE 80` は `pcg.col40` / `pcg.col80` を呼び、画面を消去して選択幅を表示する。コマンドは `shell.dos_dispatch`、ヘルプ／起動画面も更新する。
- REPL prompt はドライブ番号の後に `> `（スペース1個）。`tests/dos_smoke.rs` は両モード切替後の確認表示と `0> ` をPCG text planeで検査する。

## Debug GUI Reset counters and IRQ state (2026-10-18)
- `Debug Controls` displays `cpu.cycles()` and `cpu.instructions_executed()` as total cycles and total executed instructions. `Reset` invokes `AnyCpu::reset(0)`, pauses execution, and drops queued steps.
- Scalar/cached and pipeline reset `cycles`, `running_cycles`, and `instructions_executed` to zero. Bus system time and guest RAM remain unchanged; IRQ controller pending/enable/source levels are cleared after resetting expansion cards so cards cannot reassert stale IRQ state.
- Regression tests: `tests/cpu_select.rs::cpu_reset_clears_cycle_and_instruction_totals_for_all_modes` covers Scalar/Cached/Pipeline, and `src/main.rs::debug_reset_restarts_cpu_paused_without_clearing_ram` covers the Debug Controls reset path.
- Verification: `cargo test --test cpu_select` passed (11); `cargo test --bin cpt32 debug_reset_restarts_cpu_paused_without_clearing_ram` passed; focused `rustfmt --check` passed. Native Debug Controls window was not opened for visual inspection.

## 低速 DOS の PeC/IRQC 手動プローブ (2026-10-17)
- `--debug-gui` の `Debug Controls > Cycles / frame` を 800 にすると、画面上で設定値 800 を確認できる。DOS の `HELP` 実行中は Performance 表示が `CPU utilization: 100.0% (800 / 800 running cycles per VSync)`、`Instructions this VSync: 693` となった。
- その処理中に `FREE` を入力すると PeC trace に FIFO push が記録され、HELP 完了後にイベントが pop され、DOS は `ERROR: DRIVE NOT MOUNTED - FORMAT IT FIRST` の後に次の `0>` prompt を表示した。この `HELP` + 未接続ドライブ `FREE` の組合せではハングを再現していない。別の重い DOS 操作が必要な可能性を残し、一般的な再現不能と結論しない。
- `tests/irq_vsync_probe/probe.sol` は基準キー待ち → 10,000 回の busy loop → 2 回目のキー待ちを実行し、`irq3` が IRQC source 3 を acknowledge する手動ゲスト。`config.yaml` は BIOS を省き、PCG font を相対指定する。`CPT32_PEC_TRACE=1` と emulator の stderr redirect で GUI key / FIFO / IRQC 遷移を採取する。
- プローブ開始時に PeC FIFO を空にしてから source 3 pending を clear・enable すること。起動時に PadConnected 等の非キーイベントが FIFO に届くため、事前排出しないと source 3 が high のままになり、次の key が新しい rising edge にならない。最初の試験でこの落とし穴を確認した。
- Sol は 0=true。複数の比較条件を論理 AND する場合は bitwise `or` を使う。プローブ初版は `and` を連結していたため、A key-up（down 比較だけ false）も A key-down と判定し、post-stress 待ちを早期通過する偽陽性があった。`irq-vsync-release.log` は基準 A down/up の FIFO pop 後、後続の key event は pop されず FIFO high-water/overflow まで進むため、この後半はテスト終了後の入力であり CPU の IRQ wake 失敗を示す証拠ではない。プローブは `or` に修正してから再実行する。
- ユーザー提供の `C:\Users\y2k34\Downloads\floppy1.44.img` は 1.44 MiB の有効な DSFS。`TEST.TXT` は 14 bytes。`dsfs.py ls` の `created=51` は RTC の既定値 0 を基準にしたゲスト時刻であり、画像破損・ホスト時刻ではない（この画像は read-only 検査のみ）。
- `dos_cmd_hex` はファイルを `DOS_FILE_BUF` に読み込み、16行を表示した後、シェルと共通の `kbd.dos_read_line` で入力を待つ。既存 smoke は HEX→Q→TYPE を検査。read-only の14-byte `TEST.TXT` で scalar / pipeline 両 CPU、800 / `CYCLES_PER_FRAME` のテストを追加し、Q 入力前に最終行 `00F0:` の描画完了を待つ。
- 検証: solc/asm の DOS build 成功。DOS smoke 全7件、pipeline CPU 全16件 PASS。`cargo check --bin cpt32` 成功。ユーザー提供 image は有効な DSFS 1.44 MiB、`TEST.TXT` は14 bytes。`created=51` は RTC 既定値0のゲスト時刻であり、画像破損・ホスト時刻ではない。
- 初版テストは HEX header の直後に Q を送り、表示の途中で timeout した。これは報告されたハングではなかった。修正テストは `00F0:` 表示後に Q を送る。
- 先の `irq-vsync.log` は probe binary（262218 bytes）、後続添付は DOS binary（264764 bytes）。最新の再現は BIOS なし。
- 実 trace: HEX 描画中の Enter key-up で source3 pending=0x0009。CPU は cause3、EPC/PC=0x00006488（`con.dos_puts`）で IRQ を受理するが、対応する IRET が記録されない。Q 入力時には CPU が HALT、`cpu_irq_enabled=false`、IRQC source3 は pending+enabled のまま。後続キーは FIFO に積まれるが消費されない。
- 根本原因: active PipelineCpu の IRQ acceptance は pipeline を flush して `fetch_pc` を vector に設定する一方、`retire_pc` を中断位置に残していた。次 tick の pipeline-empty `sync_from_register_block()` が stale PC で `fetch_pc` を上書きし、IRQ dispatcher / IRET を迂回していた。修正では割り込み受理時に `retire_pc` / `retire_mode` も vector に更新し、元 PC は EPC に保持する。active-pipeline IRQ regression は修正前 `irq_enabled=false` で失敗し、修正後 PASS。
- `CPT32_PEC_TRACE=1` の `[PEC-DBG] GUI key` 行は `total_cycles` / `running_cycles` / `instructions` / `cpu_running` / `cpu_irq_enabled` / `pc` / `epc` / `irq_cause` / `debug_gui_running` を記録する。`total_cycles` は HALT 中の clock advance でも増えるので、実命令進行は `running_cycles` と `instructions` の差分で判定する。scalar/pipeline とも halted tick は `cycles` のみ増やし、`running_cycles` は増やさない。debug GUI pause 中は `cpu.run` 自体が呼ばれず、`debug_gui_running=false`。Q 入力時に IRQ enable が false なら CPU マスク／IRET 経路、true なら IRQ 入力サンプリング側を調べる。`cargo check --bin cpt32` 成功。
- `CPT32_PEC_TRACE=1` では scalar/pipeline CPU の IRQ acceptance と IRET commit も `CPU IRQ accept` / `CPU IRET` として記録し、cause・PC/EPC・active cycles・retired instructions を出す。これにより Enter key-up 後に IRQ3 handler が入ったのか、IRET まで戻ったのかを一回の実 DOS run で判別できる。`cargo check --bin cpt32` 成功。
- CPU `status` MMIO write と scalar/pipeline の `sync_from_register_block` にも IRQ-enable 差分ログを追加。次の trace で `CPU status MMIO write` / `CPU status resync` が出ればソフトウェア／register-block sync が mask を落とした経路。出ず、`CPU IRQ accept` の後に IRET が無ければ割り込み復帰経路。`cargo check --bin cpt32` 成功。
- Release-profile verification: `cargo test --release --test cpu_pipeline active_pipeline_irq_returns_to_the_interrupted_program -- --exact`, plus `dos_pipeline_hex_returns_to_shell_at_default_cycles_per_frame` and `_at_800_cycles_per_frame`, all passed. The CPU-only regression reproduces the active-pipeline IRQ case independently of the GUI. The GUI run itself has not yet been repeated after the fix.

- IDC transfer timing (`doc/spec_IDC.md` §6) must remain cycle-exact, but calculating its `u128` ceiling-division deadline on every 48 MHz device tick made the synchronous GUI frame loop repeatedly pay for expensive arithmetic during disk I/O. `WindowEvent::RedrawRequested` runs the CPU frame budget on the GUI thread. Cache the next absolute sector deadline at command start and recompute only at sector completion; this preserves latency/rate semantics while removing work proportional to simulated cycles. Read regression `idc::tests::read_dma_uses_exact_deadline_for_each_sector_without_early_next_sector` checks both sector boundaries and no early DRQ; write regression `idc::tests::write_dma_commits_each_sector_only_at_its_deadline` checks commit boundaries. Verified with `cargo test --lib idc::` and the TinyBASIC disk-transfer guest smoke.

- GUI audio synthesis currently remains on the event-loop thread after CPU/frame execution; CPAL's callback is a separate thread that only drains the output ring. `IdcAudio::render_into` previously cloned the selected `SoundProfile` every frame, deep-copying its WAV sample vectors (including FDD seek variants); it now borrows profiles and reuses mix scratch buffers. GUI retains stereo buffers, `S3w2Sound::clock_mixed_into` adds chip output into them, and `AudioHost` interleaves/converts directly into the ring without a temporary `Vec`. These reduce copies/allocations but do not move synthesis off-thread. A worker cannot safely take the current SGU/IDC objects as-is: `SguDevice`/`S3w2Sound` share `Rc<RefCell<...>>` state and PCM RAM with CPU MMIO; offloading requires a message/event handoff or a deliberate thread-safe ownership refactor. Verified: `cargo check --bin cpt32`, `cargo test --bin cpt32` (14 passed), `cargo test --lib idc::audio::tests` (6 passed), `cargo test --lib test_audio_clock` (1 passed), and targeted `rustfmt --check`.

## Cached Interpreter CPU mode (2026-10-18)
- Select with `--cpu cached` or `cpu.microarchitecture: cached`; the pipeline remains the default. `CpuKind::Cached` shares the scalar instruction semantics but takes a dedicated interpreter path: simple instructions commit in one cycle, MUL/MULH in 3, DIV/DIVU/MOD in 16. CPUID remains `0x3F`; there are no pipeline hazard or branch-flush cycles.
- The 1024-entry direct-mapped decode cache is keyed by PC and instruction mode. RAM page-generation tracking is allocated/enabled only for Cached mode, uses 4 KiB pages, and checks generations for both the first and last instruction byte (including unaligned page crossings). Changed RAM writes invalidate that page; CPU, debugger, and DMA writes all route through `Ram::write`. Cache hits skip instruction-fetch bus accesses, so BMC/DMA arbitration can differ from Scalar/Pipeline. Devices without generation tracking always fetch/decode normally.
- Regression coverage: mode parsing/config selection, architectural execution and CPUID, self-modifying code, cross-page invalidation, Short-mode return, one-cycle simple instruction, and 3/16-cycle arithmetic latency. `cargo test --test cpu_select --test cpu_pipeline` passed (26); monitor cached-mode test passed; `cargo check --bins` passed.
- Release benchmark `cargo run --release --bin cpu_bench -- 10000 5`: 90,006 guest instructions/sample; Scalar 54.338 ms, Pipeline 35.133 ms, Cached 14.942 ms median. Cached measured 3.637x Scalar and 2.351x Pipeline, with 89,991 hits / 15 misses. This is a synthetic repeated ALU/MUL/memory loop on this host, not a universal GUI workload prediction.

## Debug GUI realtime runtime ratio (2026-10-18)
- `PerformanceStats::runtime_percent` measures CPU-step wall time divided by simulated time: `execute_cpu_elapsed * CPU_CLOCK * 100 / delta(cpu.cycles())`. `execute_cpu` is timed directly, excluding time between VSync samples and rendering. Use total `cpu.cycles()`, not `running_cycles()`, so HALT intervals still measure device-clock progress. A paused sample or zero execution duration has no ratio (`None`) and does not add a runtime-history point.
- The displayed multiplier is `100 / runtime_percent`: 100% = 1.00x realtime, 50% = 2.00x, 200% = 0.50x. The runtime bar fills to 100%, then stays full and turns red above 100%; the graph uses a 0–200% scale with 100% at its midpoint, while the number remains unclamped.
- Native GUI smoke used an infinite branch guest and a temporary 50,000-cycle frame budget to keep the UI responsive. It showed `Runtime: 20098.1% of realtime (0.005x emulation; bar max 100%)`, a red full bar, and both history graphs. Performance visibility and cycle budget were temporary smoke settings and restored to their defaults; the guest and app process were removed.
- Verification: `cargo test --bin cpt32` (16 passed) and `cargo build --bin cpt32` passed. Regression tests cover 100% = 1x, 200% = 0.5x/red, faster-than-realtime = 1.5x, zero-cycle paused samples, and bar clamping.

## WAX guest loader and DOS launch (2026-10-18)
- `sol/cpt32/wax.sol` validates WAX v2 `.wax` images in guest RAM: exact file length, header/reserved fields, table bounds, block address ordering/non-overlap, payload/debug span non-overlap, reserved-memory exclusion, and an entry inside a loaded block. File spans use an 8 KiB one-bit-per-byte map in the staging window, so validation stays linear in file size. Input is limited to 64 KiB and must not overlap `0x00FE0000..0x01000000`.
- `wax_load(buffer, size)` stages the file and transfers through a copied, self-contained assembly trampoline at `0x00FF0000`; this permits blocks to replace DOS code at address 0 and the original file buffer. DOS `RUN NAME` loads from DSFS and starts the declared WAX entry. Sparse holes remain untouched. `.wal` is not executable.
- The reserved range lies in 16 MiB main RAM (`src/devices/ram.rs`). Keep the spec, Japanese translation, DOS command help, and `doc/Software.md` index synchronized when changing the guest loader contract.
- Verified: solc/asm DOS build passed; `cargo test --test dos_smoke dos_runs_sparse_wax_and_rejects_invalid_images -- --nocapture` and full `cargo test --test dos_smoke` passed (8 tests). The real-CPU WAX case rejects invalid headers, reserved destinations, overlapping load blocks and payload spans; it launches a sparse image that overwrites DOS at address 0 and halts at its declared nonzero entry.

## DOS DIR and HEX column alignment (2026-10-18)
- DIR headings place `SIZE` at column 23 and `BLOCKS` at column 31. The row must pad the displayed filename to 17 columns, print the byte size in an 8-column field, and derive the remaining padding from its decimal digit count. Width 8 supports DOS volumes up to 16 MiB. `con.dos_num_text` does not append NUL; terminate its scratch string before `con.dos_puts`.
- HEX offsets render as four hex digits plus `": "` (six columns), so the byte labels need six leading spaces, not eight. The first label/data pair starts at column 6; the ASCII separator follows 16 three-character byte fields at column 54.
- PCG text rows are packed at the active `pcg_width` (40 or 80); integration-test screen sampling must use that width as the VRAM row stride in 40-column mode.
- Verification: solc/asm DOS build and full `cargo test --test dos_smoke` passed (9 tests). `dos_dir_columns_align_in_40_and_80_column_modes` checks 16-byte and 110,228-byte files; the existing scalar/pipeline HEX smoke paths assert label/data alignment.

## Opt-in CPU and frame-stage profiler (2026-10-18)
- `--profile-cpu` implies `--debug-gui`, opens `Windows > Performance`, and is the only option enabling the new timing profile. Plain `--debug-gui` does not create frame-profile history, call `AnyCpu::enable_profiling(true)`, or invoke added timers. `run()` selects a const-generic profiled/unprofiled loop once per batch, so the default loop eliminates timer code rather than checking the profile flag on every cycle. CPU profile is an `Option<CpuProfile>`; `take_profile()` returns `None` when disabled and resets only its sample counters when enabled.
- CPU timing buckets: Device/IRQ, Fetch, Decode, Execute, Memory, Commit, Other. Scalar mode maps these to FSM states; Pipeline maps them to IF/ID/EX/MEM/WB work plus device/IRQ and control; Cached combines fetch, cache lookup, and decode in Fetch. Compare stage shares, not raw enabled-profile time: timed buckets and calls perturb execution.
- The same explicit option enables the frame-level VDP/VSync, CPU, CPU-side render submission, audio synthesis, and audio queue timing. GPU execution, VSync wait, and event-loop idle are outside these timings. `Performance` shows latest and rolling 120-frame averages.
- Implementation: `src/cpu.rs`, `src/cpu_pipeline.rs`, `src/cpu_select.rs`, and `src/main.rs`; CLI and measurement limitations are in `README.md` and `doc/spec.md`.
- Verification: `cargo test --test cpu_select` passed (10), `cargo test --bin cpt32 profile` passed (2), and focused `rustfmt --check` passed. Native smoke `cargo run --bin cpt32 -- test.wax --profile-cpu` loaded guest+BIOS and auto-opened Performance with CPU internals visible. One debug-host smoke showed 120-frame average Fetch 5.636 ms (56.1%), Device/IRQ 3.284 ms (32.7%), total measured CPU stages 10.043 ms; latest sample had 147 tick calls. Workload/host/debug-build-specific; profile overhead distorts absolute time, so compare buckets rather than raw values. Smoke process was stopped after capture.

## Bus IRQ and empty-expansion hot-path reductions (2026-10-18)
- `Bus::advance_system_cycles` used to aggregate device IRQs before every positive advance, then again after each clock boundary. It now refreshes at entry only for zero/saturated advances or when a due expansion event ran; it still refreshes after every advanced boundary. This keeps current-cycle expansion events latched before advancing and handles IDC changes in `tick_devices` after the advance. Zero-cycle calls still refresh.
- `Bus::has_expansion_cards` is set when a card is installed. Without cards, `next_expansion_event_cycle` and `run_due_expansion_events` return without scanning eight slots, and `refresh_device_irqs` avoids per-card `irq_asserted()` probes while still writing false levels for expansion IRQ sources to preserve source clearing behavior.
- Reviewed affected paths: `refresh_device_irqs` still checks DMAC, IDC, PeC Mutex state, and PeC timers every refresh; `service_dma_request(false)` still polls BMC/DMAC every device tick. These remain possible bottlenecks. Do not attribute the whole Device/IRQ profiler bucket to expansion cards or declare the hot path fixed without a controlled before/after profile.
- Verification: `cargo test --test wasm_expansion` passed (18), `cargo test --test cpu_select --test cpu_pipeline` passed (27), and `rustfmt --check --edition 2024 src/bus.rs` passed. `--profile-cpu` GUI smoke loaded `test.wax` and displayed CPU counters after the optimization; this was not a controlled performance benchmark.

## Emulator performance investigation (2026-10-18)
- `src32_testbench --profile-cpu` now reports aggregate CPU stage times after a fixed frame run. Profiling uses per-stage `Instant` timing and materially changes wall time; use it to rank stages only, and run an unprofiled pass for throughput. With `--allow-running --profile-cpu`, generic guest workloads skip the VPU-render smoke assertions so profiling can target non-VPU guests.
- On this Windows host, release `cpu_bench 10000 7` measured 90,006 instructions/sample: Scalar 41.780 ms, Pipeline 30.786 ms, Cached 11.213 ms. `dos.wax`, headless 5 frames, 7 samples measured medians Scalar 183.7 ms, Pipeline 123.2 ms, Cached 58.0 ms. These timings include process startup for the guest harness and are not GUI/GPU measurements.
- Pipeline profile of `dos.wax` reported Instruction fetch 40.6%, Device/IRQ 21.6%; Cached reported Device/IRQ 42.0%, Instruction fetch 17.6%, Stall/bookkeeping 21.1%. Cached's Fetch bucket includes cache lookup and decode, and CPU modes have different cycle/bus contracts, so these are stage rankings within each mode rather than a direct stage-by-stage speed comparison.
- First follow-up targets: profile pipeline fetch internals; break Device/IRQ into system-cycle advance, IDC/DMAC polling/arbitration, device IRQ aggregation, and IRQ-controller reads. `Bus::tick_devices` performs these operations for each emulated cycle. Preserve DMA arbitration, event timing, self-modifying code invalidation, and the known differences between Cached and Pipeline. Use GUI frame profiling and GPU timestamps separately before claiming a frontend bottleneck.
- `cargo run --release --bin emulator_component_bench` measures non-CPU components independently: layered Bus device sets, VDP/PCG composition, SGC fill/sprites, VPU triangles, and SGU inactive/noise mixing. Workloads use one warm-up operation and seven median samples. The compositor excludes WGPU upload/presentation; SGU is one chip and does not include IDC disk audio. Use this to rank CPU-side subsystem costs and retain GUI frame profiling for actual app costs.
- Latest release median (7 samples, rerun): empty Bus 5.920 ms/400k ticks; RAM+BMC/DMAC 7.479 ms/400k; core devices 7.534 ms/400k; core+PeC 14.284 ms/400k; core+empty IDC 8.740 ms/400k; standard idle set 12.732 ms/400k (2.15x empty). VDP tick 3.223 ms/1M; Graphics composition 1.161 ms/frame; PCG mode 1.548 ms/frame; Graphics+PCG overlay 1.789 ms/frame; SGC composition 8/32 sprites 1.285/1.694 ms/frame; VPU flat triangles 1/8 per frame 0.372/2.959 ms; SGU mix (800 stereo sample frames per mix) no-active/8-noise channels 0.098/0.143 ms per emulated frame. PeC and 32-sprite medians varied materially between runs and need more samples. Bus tests have no IRQ controller, active disk/DMA, or expansion card, so they do not measure the production upper bound. GPU and IDC audio costs remain open.
- Actual guest follow-up: release-built `dos.sol` on Pipeline CPU listed all 256 entries in an 80-column DSFS directory (zero-byte files), with a key sent at each `-- MORE --`. Headless elapsed without profiling was 5.107 s; profiled wall time was 11.756 s for 18.9M ticks, with stages summing to 8.201 s (Fetch 38.2%, Device/IRQ 24.7%, Execute 9.2%, Memory 9.0%). Profiling inflated wall time about 2.30x. This includes frame-sized runs, simulated page input, and screen polling; it is not pure guest execution time.
- Release-built `3d_cube.sol` ran 10 headless frames / 8M cycles in 2.114 s without profiling; profiled run was 3.300 s (Fetch 36.8%, Device/IRQ 26.0%, Execute 9.4%, Memory 8.9%). One-frame shares were similar. CPU profiling inflated wall time by about 1.56x. Headless harness excludes WGPU, presenter, and real GUI; it also combines initial noise setup and cube render, so later measurements should split startup from steady state and pair with GUI frame timing.

## Pipeline hazard-check allocation reduction (2026-10-18)
- `PipelineCpu::id_sources` previously allocated a Vec and re-decoded IF/ID even when EX was not a load. Gate it with `id_ex.is_load` and use `[usize; 2]`; R0 padding is safe only because `load_use_hazard` excludes destination R0. Preserve that exclusion when extending hazard logic.
- The regression compares source membership to `decode_id` for all 65,536 short encodings, all normal opcodes with rotating register fields, and all 32 load destinations. Source order may differ for short stores; hazard membership must match.
- Release synthetic benchmark (100,000 loop iterations, 15 samples, 900,006 retired instructions/sample): Pipeline 338.094 -> 298.639 ms and sequential rerun 340.437 -> 304.716 ms (10.5–11.7% less time). Cycles/stalls/flushes unchanged. Scalar/Cached are controls and host load varied; do not extrapolate to GUI workloads. Details: `doc/cpu-hazard-performance.md`.
- Debug checks: hazard regression 1 passed; cpu_pipeline 17 and cpu_select 11 passed; focused rustfmt passed. A release-test attempt collided with the running benchmark executable (Windows file lock); rerun after benchmark exit.
- Release rerun after benchmark exit: cpu_pipeline 17 and cpu_select 11 passed.

## Cranelift CPU JIT design (2026-10-18)
- Design proposal: doc/cpu-jit-design.md; no JIT engine or CLI has been implemented yet. Use Cached as the initial semantic/timing reference, retain the Pipeline default, and keep Wasmtime expansion-card execution separate from CPU translation.
- Native ALU blocks must preserve per-cycle device/IRQ/register-block synchronization and exact run budgets. Entry-only page checks are insufficient: DMA can modify later instructions during a block. Validate all covered pages after device progress before executing stale code, and exit on stores initially.
- Existing locked Cranelift is 0.117.2 via Wasmtime. Verify compatible direct JIT dependencies before adding them. Benchmark cold compilation separately from warm execution against Cached; no speedup was measured in this design-only task.

## Cranelift CPU JIT initial implementation (2026-10-18)
- Optional feature `jit` enables `--cpu jit` / YAML `cpu.microarchitecture: jit`; Pipeline remains default. Native Normal-mode ALU/half-immediate/CPUID/branches/calls are compiled after 16 dispatch visits, up to 16 instructions/block. Memory, MUL/DIV, Short mode, HALT/IRET, and non-previewable code retain Cached execution and timing. Direct Cranelift dependencies match existing Wasmtime's 0.117.2 series; Cargo.lock remains repository-ignored.
- `src/cpu_jit.rs` is a private child of Cpu. Generated code sees only the explicit register array and C ABI helpers, never Rust Cpu field offsets. Helpers capture panics and return through native code before Rust resumes unwinding. Do not let a panic unwind directly through Cranelift code, or free/reset a module while its native function is executing.
- Every instruction boundary shares Cached's device/IRQ/MMIO synchronization. After device/DMA progress, native preparation calls the existing generation-validated Cached fetch and compares the decoded instruction to the compiled identity. A mismatch issues the actual instruction for that already-consumed cycle and exits/invalidate the block; do not tick devices a second time. This covers DMA modifying a later instruction in the same block, page crossings, debugger/load writes, and preserves Cached cold-fetch bus effects.
- Commit of one native instruction is fused with preparation of the next. Budget exit and block exit must commit the final pending native instruction exactly once. IRQ PC changes after commit stop the next native instruction. Profiling and debugger stepping deliberately execute Cached; profiling is not a measurement of native JIT internals.
- Bound dispatch entries and cumulative native definitions at 1024 each. Prune uncompiled cold entries before resetting the module: resetting on every cold-map overflow caused 3121 compiles/94 resets on the cube guest. Final run retained hot blocks with 186 compiles/0 resets. `jit_stats` counters are cumulative across CPU reset; code bytes count live instructions including invalidated retained code, excluding allocator padding/metadata. fallback_cycles counts actual interpreter ticks, not bulk HALT advancement.
- Verified release CPU/JIT/selection/pipeline/DOS tests: 69 passed; feature-disabled debug CPU/selection/pipeline: 49 passed. JIT tests explicitly assert native execution, compare Cached state/cycles/cache counts, and cover mid-block DMA code writes + completion IRQ, MMIO PC changes, HALT wake, panic transport, non-previewable devices, and hot-block retention under cold cache pressure.
- Initial unprofiled release median (7 samples, 400k cycles/phase): warm ALU Cached 34.284 ms vs JIT 33.730 ms (1.016x, within run-to-run variation); warm mixed ALU/MUL/memory 39.729 vs 41.524 ms (0.957x). Headless process-inclusive DOS boot 5 frames: 58.325 vs 75.512 ms; cube startup 10 frames: 968.814 vs 1065.546 ms; cube 60 frames: 1667.552 vs 1835.725 ms. No robust speedup over Cached yet; do not promote JIT as a performance win or change the default. Full conditions are in doc/cpu-jit-design.md.
- Cube 10-frame graphics assertions fail during startup in both Cached and JIT. At 60 frames, both pass GP noise/VPU cube/SGC overlay assertions and are halted; this is not a steady animation benchmark. Headless measurements do not include GUI/GPU costs.
## Device/IRQ batching investigation (2026-10-18)
- Inspection only; no runtime behavior or benchmark result changed. `Bus::tick_devices` normally refreshes IRQs once inside `advance_system_cycles(1)` and again after IDC tick, with another refresh possible after DMA. Each refresh reads PeC input under a Mutex; these refreshes can observe different device states, so deleting the second call without tracking changes is not automatically equivalent.
- JIT `prepare` still invokes `begin_cached_cycle` and generation-validated Cached fetch per instruction. Reducing native arithmetic dispatch alone therefore leaves device polling, synchronization, and helper overhead.
- For a future opt-in relaxed timing mode, consider bounded CPU batches (e.g. trial limits 16/32/64 cycles), flushing at MMIO, HALT/IRET, run-budget exit, and device deadlines. `advance_system_cycles(N)` advances PeC timing/expansion events, but does not replace N IDC ticks or DMA arbitration opportunities. Active IDC/DMA/expansion paths need bulk-advance semantics or strict fallback first.
- IRQC sources and CPU input both use edge detection. Delayed CPU acceptance must preserve pending requests and low/high transitions; sampling only the final source level can lose events. Keep VSync/IRQ0 driven by the host frame loop. Batch sizes above are design candidates, not validated performance settings.
## Opt-in cached-relaxed idle device batching (2026-10-18)
- `--cpu cached-relaxed` / YAML `cpu.microarchitecture: cached-relaxed` uses Cached instruction timing/CPUID with at most 32 deferred device cycles during `run`. JIT also enables the same batching automatically while retaining per-native-instruction IRQ sampling, code generation checks, and CPU budget exits. Ordinary Cached/Pipeline and profile/direct tick/debug step retain strict device progression. Defaults are unchanged.
- Flush on every bus byte access (including RAM/cold fetch; since 2026-10-20 only timing-sensitive devices, see the timing-insensitive MMIO entry), rejected IDC word accesses, timer deadlines, HALT/IRET, and run-budget exit. IDC owns a separate u128 cycle counter: advance it by the entire idle batch, not one tick. Active IDC, any DMA BUSY channel (including globally disabled channels needing abort handling), or installed expansion cards disable batching.
- Idle BMC arbitration history and host-input IRQ observation timing may differ. Do not replace active IDC/DMA ticks with `advance_system_cycles(N)`, which only handles system-clock/PeC/expansion progression. Pending IRQ/ACK/source-level behavior must remain intact.
- A post-HALT IRQ acceptance must call `sync_register_block()` after switching PC/status to the vector. Otherwise next Fetch restores stale CPU MMIO state and skips the handler. `tests/cpu_relaxed.rs::relaxed_halt_boundary_irq_syncs_the_handler_pc_before_next_cycle` covers this specifically. Reset flushes pending time and clears the temporary relaxed-run flag.
- `connect_pec_input` already attaches timer/RTC; do not separately connect the timing block in the benchmark (overlapping device map). Bus-only IRQC tests must explicitly call `attach_irq_controller`; mapping IRQC MMIO alone does not wire Bus source updates.
- Unprofiled release medians: idle-device synthetic ALU 4.875x (80.902/16.595 ms), ALU+RAM 1.691x (128.049/75.737 ms), 800k cycles, 9 samples. Process-inclusive guest 7-sample medians: DOS boot 5 frames 1.629x (54.624/33.532 ms), cube startup 10 frames 2.689x (982.097/365.282 ms), cube 60 frames 2.417x (1661.848/687.675 ms). Full methodology/limitations: doc/emulator-performance-plan.md. Cube host TRNG makes R1/noise nondeterministic; both engines pass their own 60-frame GP/VPU/SGC assertions. No GUI/GPU or steady-animation speed claim.
- Validation before JIT integration: release with JIT feature CPU-relaxed 5, JIT 10, selection 11, pipeline 17, DOS 12 passed; Bus-specific unit tests 2 passed. Expansion suite 18 existing cases passed, plus relaxed Cached active/HALT card-deadline regression. Then enable batching in `Cpu::new_jit` as well; update the JIT test/measurement record after rerun. Windows release cpt32/testbench/benchmark builds succeeded.

## JIT uses relaxed idle device batching (2026-10-18)
- `Cpu::new_jit` opts into the same 32-cycle idle-device batching as `CachedRelaxed`. No additional mode name is required. Native `prepare` still calls the shared begin-cycle helper each instruction and compares the generation-validated cached fetch with compiled identity after device progression. IRQ acceptance/commit and budget exit remain at instruction boundaries.
- JIT timer-deadline IRQ test passes through an active native ALU block and IRET. Expansion scheduled-IRQ test covers both the looping and HALT entry with CachedRelaxed/JIT. DMA code invalidation/IRQ and the native differential suite also pass. JIT profiler and direct step use strict per-cycle Cached execution because they bypass `run_jit_block` or keep `relaxed_run` false.
- With `--features jit` release build, guest seven-sample process-inclusive medians: DOS 5 frames Cached 59.950 ms / relaxed 37.092 / JIT 61.639; cube 10 frames 978.480 / 367.928 / 449.574; cube 60 frames 1671.373 / 712.581 / 867.408. JIT speedup vs strict Cached was 0.973x, 2.176x, 1.927x; relaxed was faster than JIT in all three. In the JIT synthetic benchmark, warm ALU+branch 87.088/22.456 ms (Cached/JIT, 3.878x), mixed ALU+MUL+memory 82.822/40.355 ms (2.052x). Full methodology in doc/emulator-performance-plan.md; no GUI/GPU result.
- Final release verification with JIT feature: cpu_jit 11, cpu_relaxed 5, cpu_select 11, cpu_pipeline 17, dos_smoke 12, wasm_expansion 19 passed. Bus batching lib tests 2 passed in debug; release build of cpt32, src32_testbench, cpu_relaxed_bench and cpu_jit_bench succeeded. `git diff --check` passed before JIT integration; repeat it after doc/test updates.

## JIT native idle-window batching and tiering (2026-10-18)
- JIT's remaining per-instruction Rust helper, Cached fetch and 32-register MMIO copies can dominate native arithmetic. Add a second pure native function only after 128 dispatch visits; retain conservative compilation at 16. Keep one-instruction blocks interpreted and gate the outer JIT call to running Normal/Fetch states to avoid calls throughout Short mode and MUL/DIV stalls.
- A pure batch must stay STRICTLY BEFORE the next deferred-device flush/deadline: `Bus::native_idle_budget()` leaves the final cycle to ordinary execution. All prefix instructions must already be decode-cache hits with matching identity and endpoint generations. No cold bus reads, stores, active DMA/IDC, installed expansion cards, or enabled pending CPU IRQ may enter this path. With no device progress or store, code/CPU MMIO cannot change inside the batch; sync registers and counters once at exit. Preserve exact run prefix budgets and cache hit counts.
- Do not validate changed/cold instructions speculatively using Bus reads: those flush time and can change IRQ/device state before the corresponding instruction. Reject the whole candidate prefix to the conservative helper instead, so late self-modification still invalidates the original native entry and preserves cold-fetch effects. DMA-capable execution must keep the per-cycle path.
- Eagerly generating both variants with Cranelift speed optimization increased DOS compilation to ~27 ms and regressed startup. Retain the existing Cranelift defaults and tier the pure variant. Preserve each block's original unique serial for delayed variant names; using the latest global serial can collide across blocks. Limits remain 1024 block definitions, up to two functions per block. `batch_compilations` is separate from first-tier `compilations`; compile_time and code bytes include both.
- Timer tests must loop ADDI/JMP with offset -8, not -4 (which only loops JMP). CPU IRQ is edge-triggered: mask/sample a latched timer source low before driving a fresh external IRQ edge. Comparing two CPUs alone is insufficient if both suppress the intended test IRQ; assert handler counts and causes too.
- Final release 800k-cycle/phase, 9-sample medians: previous/new JIT warm ALU 19.099/11.877 ms (1.608x), mixed 37.705/32.318 ms (1.167x). With idle platform devices new warm ALU Cached/relaxed/JIT 83.977/19.201/12.017 ms; mixed 99.950/39.343/38.758 ms. Mixed improvement over relaxed is small; do not attribute device batching's whole gain to JIT.
- Seven-sample, alternating process-inclusive headless guest medians previous/new JIT: DOS boot5 53.791/54.442 ms (flat), cube startup10 458.432/434.584 ms, cube complete60 926.754/867.300 ms (~5–6% less time, host variation present). Relaxed remains faster (34.085/372.873/764.259 ms). Cube smoke reports native 1,795,618 instructions versus 11,136,733 fallback cycles; Short/memory/arithmetic fallback remains a large target. No GUI/GPU/steady-animation claim. Both guests retain PC/cycles/running outcomes and cube60 passes all rendering checks; exclude host-TRNG R1 equality.
- Final release focused suites: CPU 21, JIT 13, relaxed 5, selection 11, pipeline 17, DOS 12, expansion 19 passed (98 total). Bus batching unit tests 2 and no-feature CPU/selection/pipeline 49 passed. New JIT differential tests prove batching occurred and cover every 1–65-cycle prefix/deadline, unaligned page crossing, register MMIO, late block code replacement and external IRQ. Details: doc/cpu-jit-design.md; preserve earlier unrelated working-tree edits.
- Additional verification: debug `cargo test --features jit --test cpu_jit` also passed all 13, exercising idle-budget debug assertions. Focused rustfmt and final diff whitespace checks passed. Cargo reported incremental-cache hard-link warnings and copied files instead; tests completed successfully.

## Python CPT32 GUI launcher (2026-10-18)
- Rust startup reads a fixed `config.yaml` relative to the process working directory. The Python/Tk launcher stages an isolated run folder with that filename and runs the absolute ROM/config file references there; never point only the GUI binary at a user YAML because Rust would keep loading the repo's default config.
- Rebase known config-relative keys before staging: `bios.image`, `pcg.font`, `disk[].image`, `expansion_card[].package`, and `disk_sound.asset_root`. Disk read/write permissions are intentional; read/write media are used in place, so launch does not copy or modify their contents itself. Sound root fallback and other relative runtime files need deliberate path handling.
- Preserve unknown config keys while editing exposed CPU, logging, BIOS, PCG, disk, and card controls. Save the source YAML only through the explicit Save button; comment formatting may be lost. Named profiles live under LocalAppData on Windows and `~/.config` elsewhere. Keep profiles free of secrets.
- Python 3.15/Tk is available in this workspace but PyYAML is absent and network access cannot install it. Keep the launcher usable without dependencies using the standard-library YAML subset parser/emitter, fail clearly on aliases/tags/block strings, and use PyYAML automatically if present. Current repository config is ordinary nested YAML and uses quoted Windows paths.
- Rust CLI accepts program plus `--cpu KIND`; monitor mode is `--monitor [program]`. JIT needs `--features jit`. Cargo can run from a temporary working directory with `--manifest-path <repo>/Cargo.toml`, which allows per-run config staging. Child stdout/stderr can be streamed into a Tk text area from a reader thread and drained on the Tk loop via `queue`.
- `python -m py_compile tools/cpt32-launcher/launcher.py` is syntax checking only. Current developer instructions prohibited adding/running tests unless specifically requested; don't claim runtime GUI/YAML roundtrip tests were run.


## Furnace S3W2 SGUB ROM exporter and pitch interoperability (2026-10-18)
- The sibling Furnace checkout (`../furnace`) has a stub S3W2 implementation, not a normative CPT32 SGU core. Existing uncommitted platform/GUI/core edits were preserved. Added ROM target `CPT32 SGUB (S3W2)` for one or two S3W2 chips only, GUI and CLI `.sgub` output, 48 kHz tick accumulation into 60 Hz frames, ordered writes, bounded size/count, cancellation and engine cleanup. Explicit `.sgub` CLI output must not silently fall back to another viable ROM format.
- Furnace uses modulation 3/4 for Slew Rate/Step, while CPT32 already uses those IDs for Hard Sync/Window. Preserve the latter; SGUB maps Furnace 3/4 to added CPT32 5/6 and unknown Furnace 5..31 to None with a warning (stub no-op behavior). Never label every reserved value as unsupported Slew. Slew uses a per-channel unsigned Q8.8 accumulator and MOD_PARAM1 at 192 kHz, reset to zero on reset/phase-reset write. Rate settles; Step deliberately takes a negative step at equality and may oscillate. Wave/PCM/noise paths are covered, preserving SQV4 behavior.
- SGUB initialization is applied only once, so whole-song repeat resets both chips in frame zero. PCM_CONTROL bit 0 gates wave/noise too: clearing it during reset without re-enabling it produces a perfectly parseable but silent wavetable stream. Frame zero re-enables active channels at volume zero. `regPool` is not a reset snapshot (pan and wave defaults differ).
- Furnace signed PCM becomes unsigned via +128 modulo 256. Compatible bank layouts share offsets; conflicting bank 1 is packed after bank 0 with a silent guard/alignment, and every start/end/loop address is rewritten with byte carries. Combined banks plus guards must fit 1 MiB; individual common samples in conflicting banks are not deduplicated. Rejecting every overlap unnecessarily blocks valid two-chip songs.
- Pitch fixes are in Furnace: use SharedChannel::calcFreq, fold its five-octave frequency-table scaling (/32) into table initialization so non-linear pitch offsets are in Hz, and scale PCM separately by sample centerRate/(standard C4 Hz * 32). Standard C4 uses 440 Hz tuning as reference so song tuning still affects PCM. Raw values stay register units; clamp final values; clear key transition flags; keep note state and notifyPitchTable current. A4 absolute macro [0,128,-128,0] gives [440,466,415,440] in linear mode; non-linear gives [440,568,312,440].
- Specs: doc/spec_SGU.md and doc/spec_SGUB.md, indexed in doc/Software.md; Furnace doc/2-interface/export.md documents the stub and format limits. `tests/fixtures/furnace_pitch.sgub` is actual Furnace CLI frame output with only its unused 1 MiB PCM initialization removed (header counts/size repaired).
- Verified: Furnace Release build; standalone CTest SGUB encoder (size/count/address limits, strobes, fractional frame timing and modulation translation); real-engine CLI smoke including pitch modes, PCM/tuning, 50/125 Hz, Slew/Step/reserved modes, incompatible systems, shared-RAM relocation and capacity rejection. CPT32 tests: furnace_sgub 2, SGU core 9, music::stream 7 passed; CPT32 Release build passed. Full Rust suite and native GUI audition were not run. Furnace had pre-existing whitespace warnings and unrelated core/link warnings.
- Final rerun: Furnace real-engine smoke passed all 18 cases, including PCM song tuning at 432 Hz. CPT32 Release build also passed with `--features jit` to retain the existing optional JIT capability. Focused rustfmt and SRC32 diff checks passed; Furnace whitespace checks excluding pre-existing edited platform/core files passed.
- GUI export follow-up: the full 1 MiB PCM initialization is 1,048,576 SGUB records. Emit this bulk region in chunks with direct packed writes rather than per-record initializer-list vector inserts, and keep the ROM progress indicator advancing during both initialization and song capture; a constant 0% bar made the GUI exporter appear frozen. Re-run standalone writer CTest and the real-engine CLI smoke after changing the record encoding path.

## Launcher sol source ROM support (2026-10-18)
- ROM selection accepts `.sol` (case-insensitive detection). Each launch runs the selected workspace's `tools/solc/solc.py compile` then `tools/asm/asm.py` using `sys.executable` and compiler defaults, producing `program.asm` / `program.bin` inside the isolated run folder. Pass the original absolute source path so relative sol includes resolve against its directory. Keep the source path in the UI/profiles; only the emulator command receives the generated binary path. Existing executable and monitor modes use the same build path.
- Build and emulator stages run serially on the reader worker with stdout/stderr forwarded to the Tk queue. A failed build must never launch later stages. Track the whole pipeline with `running`, rather than checking only a current process: between stages (and before the first spawn) there may be no live process. Serialize spawning with the stop event/lock so stopping cannot accidentally launch the next stage. Close cleanup waits for the whole worker before removing its files.
- Verified `python -m unittest discover -s tools/cpt32-launcher -p 'test_*.py' -v`: 9 passed, including real compilation/assembly with relative includes and Unicode/space paths, compile/assembly failure gates, cancellation boundaries, missing tools, and GUI/monitor command construction for Cargo/existing executables. The earlier launcher note about a prohibition on tests described a previous session, not a current restriction.
- Real pipeline smoke with the existing release `src32_testbench.exe`: `1 2 add` halted with R1=3; repository `3d_cube.sol` compiled/assembled and executed 2 frames / 1,600,000 Pipeline cycles with render checks skipped. This confirms guest execution only; interactive GUI and rendered cube appearance were not checked.

## SGUB v2 format design draft (2026-10-18)
- `doc/spec_SGUB_v2.md` proposes a new version while keeping v1 byte-compatible. The main size reduction is pool payloads referenced by ID plus FILL for deterministic silent RAM; COPY_POOL can place a shared wave/PCM blob at multiple destinations. Keep SGU wave SRAM (`0x000..0x7FF`) distinct from channel registers (`0x800..0x8FF`), and retain the 14 MiB loaded-stream bound.
- The v2 Rust `StreamPlayer` validates and plays WRITE/FILL/COPY_POOL, and `compile_song_v2` pools repeated 256-byte waves and PCM. Furnace ROM export now writes v2 pools and fill/copy operations; its writer and release build passed checks.
- The guest Sol v2 path now passes actual Furnace output and synthetic PCM subrange/repeat regression tests. The failure was missing STREAM_BASE rebasing, not opcode recognition: header/directory offsets are file-relative, so every directory access, operation scan cursor/limit, frame cursor and pool payload must add STREAM_BASE exactly once. A pool-less empty frame did not expose these wrong addresses; tests must execute real writes and copies.
- `tests/fixtures/furnace_minimal_v2.sgub` is the full user-provided 4,995-byte Furnace export (384 frames, two wave pools, full 1 MiB PCM silence FILL, one 131 Hz note). Test both Rust playback/audio and compiled guest execution on PipelineCpu. Also retain the v1 Pitch macro test, initial/frame PCM copies, odd source/destination ranges, FILL word/tail handling, repeat without initialization, and invalid offsets/overflow rejection before uploading PCM.
- FILL/COPY_POOL destinations exclude side-effectful control registers, so bulk 32-bit stores/loads are safe; ordinary WRITE must stay ordered. Avoid calling a 3-argument Sol helper for every PCM byte. Bound range lengths using subtraction to avoid signed u32 addition overflow. Check directory size before reading entries.
- Build playable drivers with `-O speed`. Default size mode uses expensive short-mode helper sequences; a dense reset frame can span several host CPU frame budgets even when playback is correct. In guest tests wait for IRQC enable after boot and for the IRQ handler to return before inspecting completed frame writes. Keep VSync generation in the host; do not generate it from guest cycle counts. Music Editor v2 export remains deferred.
- Validate every directory entry, pool range, initial op, frame op, and destination before mutating device memory. Never replay initialization on whole-song repeat; frame zero must reset mutable SGU state.
- Verified the playback repair with `cargo test --release --test furnace_sgub` (7 passed), compilation/assembly of the speed-optimized guest, and the real user sample through the Release `src32_testbench` (two VSync frames, note/volume and final PCM silence byte checks). Rebuilt `cpt32` and `src32_testbench` in Release with the existing `jit` feature, so Music Editor's embedded driver source is updated too. Interactive GUI audio audition was not performed.

## DSFS ImHex pattern (2026-10-18)
- `tools/imhex/dsfs.hexpat` describes DSFS v1 raw images using big-endian fields: boot sector at 0, superblock at byte 512, bitmap and directory at their superblock block offsets, and 256 fixed 32-byte entries. It places each used entry's declared file bytes at `start_block * 512`, guarded against empty/out-of-volume ranges.
- This is a visualizer, not a mount validator: it does not enforce name/padding rules, bitmap consistency, duplicate names, overlap, or reserved-byte constraints. Use `tools/dsfs/dsfs.py verify IMAGE` for the full §7 contract.
- Verified generated 64-sector images: `dsfs.py verify` reported zero leaks for the one-file and two-file fixtures; ImHex `--pl format` parsed the expected superblock, all 256 entries, exact 21-byte and 600-byte file payloads (including the two-block extent), and omitted the empty file's payload view.

## SGUB ImHex patterns (2026-10-18)
- `tools/imhex/sgub_v1.hexpat` parses the 20-byte v1 header, five-byte records with a 24-bit address, initial record array, and each frame's u16-counted records. `tools/imhex/sgub_v2.hexpat` parses the 40-byte header, 16-byte pool directory entries with payload placement at file offsets, and opcode-sized WRITE (6-byte), FILL (10-byte), and COPY_POOL (15-byte) operations in initialization and frames. Both are visualizers, not semantic validators; `StreamPlayer` is the validation authority.
- Verified with ImHex `--pl format` on `tests/fixtures/furnace_pitch.sgub` (v1: 8 frames, 24,641 bytes) and `tests/fixtures/furnace_minimal_v2.sgub` (v2: 384 frames, two 256-byte wave pools, one initial FILL). The v2 fixture exercised all three opcodes across its frames. Both outputs had exact declared frame/pool/operation counts and parsed without errors.

## GUI CPU execution policies (2026-10-18)
- GUI CPU scheduling is selected with `--cpu-execution strict|best-effort|cycle-independent` or Debug Controls. Default is `best-effort`: run up to the configured `Cycles / frame` cap or time slice, then drop the remainder rather than catching up later. `strict` preserves exact per-frame cycle budgets and can delay GUI input/rendering. `cycle-independent` ignores the cycle cap and runs for the time slice; host performance determines guest cycles.
- `--cpu-time-slice-ms` and the Debug Controls field accept 1–16 ms (default 8 ms). Timed modes check between 256-cycle batches; one batch can exceed the requested slice, so this is a soft limit, not hard real-time isolation. CPU, rendering, and audio remain on the GUI thread. Performance cycle utilization is unavailable in cycle-independent mode because it has no fixed cycle budget.
- Verified `cargo test --bin cpt32 cpu_`, `cargo test --bin cpt32 cycle_independent_performance`, `cargo build --bin cpt32`, and `cargo fmt -- --check`. Native GUI smoke showed Cycle-independent selected at 8 ms, then remained visible/responsive while the running CPU counter advanced. Full `cargo test --bin cpt32` also exposed two package guest-builder assertions with PC `0xFF000000` instead of `0`; cause remains unverified.

## Launcher CPU execution settings (2026-10-18)
- The CPT32 launcher exposes `strict`, `best-effort` (default), and `cycle-independent` plus a 1–16 ms time slice (default 8 ms) in its launch settings. GUI launch passes `--cpu-execution` and `--cpu-time-slice-ms`; monitor launch omits both because the execution policy controls GUI CPU frame scheduling only.
- These launch settings belong in saved launcher profiles, not the emulator YAML config schema. Older profiles without the fields load using the defaults. Validate mode and time slice before building or launching; malformed or out-of-range time slices must fail with `ConfigError`.
- Verify command construction for Cargo and existing executables, profile round trips/legacy defaults, and monitor omission with `python -m unittest discover -s tools/cpt32-launcher -p 'test_*.py' -v`. GUI controls are in the launcher’s 起動設定 tab.
- Main RAM and VDP VRAM sizes are configured in `config.yaml` as `memory.ram_mib` (1–256 MiB, default 16) and `memory.vram_mib` (4–128 MiB, default 4). VRAM's `0x10000000` base leaves exactly 128 MiB before PCMRAM at `0x18000000`; cap VRAM at 128 MiB to prevent overlap. Launcher controls are in 詳細設定, and the YAML config example plus `doc/spec.md` describe defaults/ranges.

- Launcher UI initialization must declare every tkinter `StringVar` before `_build_ui()`; missing `hdd_profile` causes `AttributeError` during construction, which launcher unit tests do not catch because they avoid creating a real `Tk` root. Smoke-launch the window after editing controls.

## CPU Bus Error behavior (2026-10-18)
- Unmapped CPU reads/writes set the Bus Error latch; they must not panic the host. Scalar/Cached execution consumes it at instruction commit, records the faulting instruction PC in EPC, sets CAUSE=2, masks IRQ, and jumps through `0xFFFF0008`. Pipeline detects data-access faults in MEM and flushes younger stages. An unchanged default self-vector halts at that vector instead of repeatedly decoding it.
- Regression: `tests/cpu_select.rs::unmapped_store_enters_bus_error_handler_for_each_cpu_kind` exercises a 1 MiB RAM fault and custom handler for Scalar, Cached, CachedRelaxed, and Pipeline CPUs.

- Debugger-originated reads/writes use `BusAccessSource::Debugger`; unmapped addresses and faulted devices must not latch a CPU Bus Error or redirect guest execution. Open-bus debug byte reads return `None`; equivalent CPU-originated accesses still latch Bus Error. Regression: `tests/cpu_select.rs::debugger_memory_reads_do_not_raise_cpu_bus_error_for_any_cpu_kind`.
- CPU performance stage 1 (2026-10-18): BMC `grant` can bypass the priority/round-robin search only when the original request mask has exactly one bit and eligibility has already been checked. Reset all eight wait counters, update pending/active/next_master exactly as the general path. Testing only `eligible.is_power_of_two()` is unsafe: disabled arbitration can leave multiple original requests whose waiting/aging state must still advance. The differential regression covers 32,768 states including all masks, enable states, round-robin positions, aging thresholds and saturation; keep active state unchanged on no grant.
- `cpu_bench --platform` adds idle standard devices and IRQ controller; RAM-only measurements never exercise BMC. Effective MHz is running cycles / measured CPU seconds / 1e6, not MIPS. The mixed loop retains exact architecture/instruction/stall/flush checks. Stage 1 reduced Pipeline time by 7.3–7.8% in platform synthesis, 6.3% for DOS boot and 7.0% for cube startup on this host. These are not GUI/steady-state/active DMA performance claims; see `doc/emulator-performance-plan.md` for conditions and the 48 MHz roadmap. At an 8 ms GUI time slice, 800,000 cycles/frame requires 100 MHz CPU-only throughput; best-effort dropped cycles cannot establish 48 MHz realtime operation.
- A 64 KiB direct Bus page-table experiment was rejected: DOS boot improved about 3%, but RAM-heavy synthesis regressed. Preserve byte arbitration, MMIO effects, IRQ sampling and instruction/cache coherence when investigating fetch; do not silently substitute Cached/JIT semantics for Pipeline. Saved comparison executables and guest sample data are local under `target/cpu-performance-baseline/` and are not versioned.

## Launcher sol compiler arguments (2026-10-18)
- The launcher’s 起動設定 tab accepts sol compiler arguments as shell-like whitespace-separated tokens; quoted whitespace stays within one argument. `prepare_program` passes the parsed argv directly to `solc.py compile` without invoking a shell, and malformed quoting raises `ConfigError` before launch.
- Save the setting in launcher profiles; older profiles default it to empty (solc defaults). Verify argument boundaries and malformed quotes with `python -m unittest discover -s tools/cpt32-launcher -p 'test_*.py' -v`.

## Independent freestanding C / LLVM SRC32 toolchain (2026-10-18)
- `tools/llvm-src32/` is separate from `tools/csc/`. `driver.py` compiles C with Clang's big-endian MIPS32/o32 data model, then uses llvmlite's official parser, verifier and module linker before custom SRC32 lowering and existing ASM/WAX packing. Do not silently retarget host x86 LLVM IR: byte order and pointer/aggregate layout must already match.
- The private stack ABI is not sol/csc/MIPS binary-compatible: SP=R28, FP=R29, LR=R31, 8-byte call alignment, stack arguments at callee FP+8, recursive frame-local SSA spills; i64 memory/arguments are BE word pairs, return low=R1/high=R2. Whole-program `inreg` arguments are consistently remapped. Phi edge copies read all inputs before writing destinations; branch operand ValueRefs must be canonicalized to real block objects before accessing `.instructions`.
- `ModuleRef.struct_types` may include literal anonymous structs after linking; only named types form standalone LLVM type declarations. LLVM22 can upgrade `icmp` to include `samesign`; parse predicate after that optional flag, never interpret it as a comparison predicate.
- ROCm Clang20 ran SLPVectorizerPass at O2 despite frontend `-fno-slp-vectorize`; pass tracing identified it. Driver explicitly sets LLVM `-vectorize-slp=false` and `-vectorize-loops=false`. Unsupported vector IR still fails, rather than pretending the CPU has SIMD/FPU.
- `cmake/SRC32.cmake` uses `project(... LANGUAGES NONE)` plus `src32_add_library` (source collection, not archives) and `src32_add_executable` (WAX custom target). Driver `--depfile` aggregates actual Clang header dependencies, with Make escaping including Windows paths/spaces; CMake passes DEPFILE to Ninja. This is not an unchanged-project standard add_executable toolchain. Read README for supported target properties/arguments and limitations.
- Verified 12 unittest cases: four-TU C conformance at O0/O1/O2 on Scalar/Cached/CachedRelaxed/Pipeline; narrow/wide arithmetic including 64-bit edge division/shifts; recursive/indirect calls; struct value ABI; VLA; memory runtime; cyclic phi; long jumps/nonzero origin; malformed/unresolved inputs; ASM roundtrip; CMake header-change rebuild R1=42→43. Regression cases cover i1 true/false switch cases, user identifiers named `acc`/`byref`/`atomic`, and incompatible module data layouts rejected before linking. Manual CMake O1 conformance WAX passed Pipeline R1=42. Use native `src32_testbench.exe` on Windows; extensionless binary can be an old Linux build. Run from isolated cwd without BIOS config; `--skip-render-checks` also skips R1 validation, so do not use it for result proof.
- This is integer freestanding C, not all C11: floating point/SIMD/atomics/varargs and hosted OS/libc are explicit errors or absent APIs. Stack exhaustion is not checked at runtime; provision RAM/stack margin. Runtime string routines link only unresolved names, preserving user definitions.

## CPT32 C guest library port
- `sdk/cpt32` ports implemented `sol/cpt32` APIs to the independent LLVM/SRC32 C toolchain: display/PCG/print/RNG/colors, IRQ, IDC/DSFS, WAX, SGUB v1/v2. C predicates use conventional nonzero-true; error codes retain source meanings. IDC identify returns status separately from its `uint32_t` sector capacity, avoiding the sol API's `-1` sentinel collision with high-bit capacities. The source colors helper is additive `b + (g << 2) + (r << 4)`, not RGB332; source `hsv2color` was empty and is not advertised.
- Link `cpt32_irqc` before enabling interrupts. Backend detects `void irqc_dispatch(void)`, installs all16 vector instructions as JR R30 before main, reserves R30, saves all other live registers on a128-byte/8-byte-aligned IRQ frame, calls the C dispatcher and restores through IRET. Source0 VSync is host-driven. Dispatcher drains all currently enabled pending sources in priority order, invokes the callback before ack so device conditions can be cleared, then acknowledges the bit. Do not enable nested interrupts in callbacks.
- WAX load uses the original 16 MiB RAM reservation `0x00FE0000..0x00FFFFFF`, with a raw SRC32 trampoline at `0x00FF0000`. It reads four private-ABI arguments at incoming SP+0..12 before overwriting code or stack. Never replace it with an ordinary C copy loop returning into overwritten memory. Real smoke overwrites both address0 code and the current stack page and boots a guest returning R1=42.
- Use O0 for these library sources: Clang O1 may generate unsupported fastcc on internal helpers. Verified Clang20.1.8 + llvmlite0.50; LLVM23 support is not claimed by this port.
- `python sdk/cpt32/smoke/run.py --clang PATH` builds 5 guests and checks R1=42 on Scalar/Pipeline, with disposable DSFS media recreated per run and host validator reporting zero leaks. Coverage includes PCG ring cursor, IRQ callbacks preserving interrupted calculations, SGUB v1/v2 and rejection before initial MMIO, DSFS write/grow/shrink/delete, WAX validation and destructive transfer. Existing compiler suite12 tests passes.
- `examples/3d-cube-c` now links display/RNG/IRQ libraries. Its Q10-to-binary32 exponent uses scale+136, matching source sol, not+127. Isolated no-BIOS testbench runs120 frames on Scalar/Pipeline passed cube/SGC/GP-background assertions with running=true and VPUstatus0; a BIOS-containing cwd can falsely test the BIOS instead of the guest.
- The same5 guest smokes also pass Cached/CachedRelaxed, including destructive WAX code/cache replacement and zero-leak DSFS validation. The default runner keeps Scalar/Pipeline; select additional modes with repeated `--cpu` flags.
## DOS and TinyBASIC C guest ports (2026-10-18)

- Freestanding C ports of root `dos.sol` and `tinybasic.sol` live in `examples/dos-c` and `examples/tinybasic-c`. Build their WAX artifacts through CMake with `tools/llvm-src32`; they do not use `tools/csc`. `tools/llvm-src32/README.md` now marks `tools/csc` deprecated and states it will not receive further updates; the CSC implementation itself was not changed.
- `<cpt32/cpu.h>` declares `cpt32_halt()`, a reserved LLVM/SRC32 intrinsic lowered directly to `HALT`. It accepts no arguments, returns void, and resumes after an enabled IRQ. It is not inline assembly or a host sleep API. Toolchain regression `test_cpt32_halt_waits_for_irq_then_resumes` verifies wakeup on Scalar and Pipeline.
- PeC can contain queued keyboard events before the foreground reader reaches its first wait. Readers must inspect/drain the FIFO before HALT and drain again after wake; otherwise an earlier IRQ acknowledgement can leave the level asserted without a new rising edge. C guest keyboard smokes exercise this prequeued-input case. TinyBASIC string token values are offsets into `STR_BUF`; token decoding must add that base, and lexing must store the first character at the reported offset.
- Rust smoke harnesses can select raw C images via `DOS_GUEST_IMAGE` and `TINYBASIC_GUEST_IMAGE`, avoiding overwrite of the default sol guest binaries. DOS harness execution has a five-minute per-harness wall-clock watchdog.
- Verified CMake WAX builds and testbench execution for both guests (`pipeline`, 120 frames, 96,000,000 cycles, `running=false`; VPU status 0). With raw images built using `tools/llvm-src32/driver.py`, `cargo test --test dos_smoke` passed 11 tests; `cargo test --test tinybasic --test tinybasic_load_disk` passed 6 tests. The complete LLVM/SRC32 toolchain suite passed 13 tests, including HALT wakeup; `cargo fmt -- --check` passed.

## Configurable JIT block size (2026-10-19)
- JIT preview maximum is controlled by `cpu.jit_block_size`, valid range 2..=64 instructions, default 16. Launcher presets map Small=8, Medium=16, Big=32 and a numeric control accepts custom values. The YAML value is carried through staged launcher configs and profiles, then read by GUI, monitor, and testbench CPU construction.
- Larger previews stop early at control-flow instructions or unsupported instructions; per-instruction code-generation validation and cycle checks remain in effect. Larger settings can increase generated code and reduce stability, so no performance gain is guaranteed.
- Regressions: `tests/cpu_select.rs::jit_block_size_defaults_and_validates_configured_values` checks defaults/range/type; launcher document round-trip checks custom 32. Focused verification passed: JIT-enabled `cpu_jit`/`cpu_select`, no-default-feature `cpu_select`, and launcher unittest suite. On Windows, set `TMPDIR` to a workspace-writable path when launcher tests cannot create files under Python's sandboxed default Temp.

## Launcher console log mirroring (2026-10-19)
- `tools/cpt32-launcher/launcher.py::_spawn_and_read` owns child stdout capture and forwards lines through `result_queue`; `_poll_process` renders them in the Tk log. Mirror lines with `print(..., flush=True)` at production time so the Python console receives them immediately while GUI queue behavior remains intact. Also mirror launch/build failure messages.
- The launcher unittest can invoke `_spawn_and_read` synchronously and capture its console output with `redirect_stdout`; child output still passes through the pipe first.
- SGUB guest driver のC版は `sol/cpt32/sgu_music_driver.c`。SDKの `sgub.c` と `irqc.c`、PCG表示用 `pcg.c` をリンクし、Music Editor は `sgu_music_driver.c` をストリームと同じ場所に書き出す。driver はRAM `0x00200000` のstreamを読み、v1はheader+16、v2はheader+32からtotal sizeを読む。全stream検証とinit後にIRQ0を有効化し、IRQごとに1 frame進める。
- SGU master attenuation belongs in the shared sample-output stage, after per-channel register volume, so hardware guest playback, Music Editor preview, and host mixing all receive the same gain without rewriting SGUB channel volumes.
- 再生中ビジュアライザーはPCG画面へ16chのvolume (`base+3`) とfrequency (`base+0..1`) をhexで表示し、VSyncごとに更新する。FILL/COPY_POOLは非制御領域に限定される仕様なのでC側でも32-bit MMIO bulk転送とbyte tailを使う。通常WRITEの並び順は維持する。
- `tools/llvm-src32/driver.py` では現状C guestを `-O0` でビルドする。`-O1/-O2` は内部関数呼び出しのfastcc lowering errorが起き、一時的にstatic helperをglobal化して通した実験でも実stream再生の初期化失敗を確認した。最適化レベルを変える場合は compiler loweringとguest再生の両方を再検証する。
- 検証: `python sdk/cpt32/smoke/run.py --clang <clang> --cpu pipeline` 全件通過。Music Editor export focused test通過。実C driver + `music.sgub` のtestbench 4-frame実行はR1=0でhalt待ちまで到達。

## LLVM/SRC32 optimized internal calls (2026-10-19)
- `tools/llvm-src32/backend.py` accepts LLVM `fastcc` on both definitions and direct/indirect calls, remapping it consistently to the private SRC32 stack ABI, like `ccc` and the existing `inreg` hint. Clang 20 emits `fastcc` for surviving static helpers at `-O1`/`-O2`; accepting it only on one side still breaks compilation. Other calling conventions and `musttail` remain errors; ordinary `tail` uses a normal call.
- This supersedes the earlier O0-only SDK/SGUB workaround. Do not make static helpers global or silently lower the requested optimization level. The old conformance project alone did not expose this failure because helpers could be inlined. The new regression forces a noinline helper and checks that linked IR actually contains `fastcc`, then executes its 64-bit result. Another IR regression covers indirect fastcc, private byval copies and 64-bit returns.
- `python sdk/cpt32/smoke/run.py --optimization 1` (or `2`) rebuilds all five SDK guests and library sources at that level. Both levels passed Scalar/Pipeline execution with R1=42 and DSFS zero-leak verification. The LLVM toolchain suite also checks the actual C SGUB driver with the complete Furnace v2 fixture at O0/O1/O2 on Pipeline: IRQ0 enabled, last PCM byte=128, channel frequency=131 and volume=255. This checks initialization/playback, not just a successful build or a HALT.
- The testbench music-stream pre-run is a fixed 50 million cycles. Scalar can still be initializing at O0 or handling the first dense IRQ at O1/O2 when the first frame is checked; do not assume Pipeline frame deadlines for Scalar. SDK functional smokes still cover both CPUs without that deadline assumption.
- In this Windows sandbox, Python's default temporary directory may be inaccessible. Set TEMP and TMP to an existing writable directory under target before running compiler/tests. Release `src32_testbench` substantially reduces the real-stream regression runtime; set SRC32_TESTBENCH to its absolute path after rebuilding it.
## 3WS88PWN4 WASM audio card (2026-10-19)

- ABI v1 optionally exports `cpt32_audio_render(frames, sample_rate) -> pointer` plus `memory`. The host checks types/ranges and consumes little-endian interleaved i16 stereo before the next guest call. Old five-export cards remain valid and silent. GUI mixes SGU/IDC then cards in slot order; audio stays on the emulator thread and never advances virtual cycles. Frame-end MMIO state is used, so this is not sample-timestamped playback.
- Keep the 10,000,000 fuel ceiling. A 256-frame call exhausted fuel during 64-operator decay. The host now calls roughly 1 ms blocks capped at 64 frames, allowing 8k/44.1k/48k/96k resampling without increasing guest execution limits. Audio traps/bad pointers fault only that card; prior mixed sources remain intact.
- New implementation: `examples/wasm-3ws88pwn4`, contract `doc/spec_3WS88PWN4.md`. Oscillator phases and WT/PCM cursors are integer Q32.32, increments derived directly from the 192 kHz period and Q4.12 ratios. Amplitude/envelope/filter math remains floating point. Negative modulation table indices must truncate toward zero like the original C++ core; floor/wrap alone changes direct-phase synthesis audibly.
- Register sheet Gate is HS `+0x1E` (write 1 always retriggers, 0 releases); WT sync is PCM/WT `+8` (any write, reads 0). ResetGate restarts time/phase but preserves the old envelope level for the first sample and preserves feedback. Sheet mode 1/4 labels disagree with source: retain source 1=interpolated WT, 4=nearest. Comp RateU/L are unused. FIFO mode 5 has new data/count MMIO at `0x400300+16*channel`, explicitly a card extension in reserved space.
- Legacy PCM looping uses `start + phase % (end-loop_start)`, not `loop_start + ...`; this surprising behavior is intentionally documented/preserved. Preserve pan 0=both full, mute 0=audible, PCM gain 1.3, master /4, and the original EQ coefficient default rate of 44.1 kHz.
- Verified: 22 WASM expansion tests plus the separately invoked built-package integration test; chip tests (integer phase carry/year skip/wrap, Gate/Release/WTSync, FIFO/zero-frequency, and 49 old-C++ output scenarios), debug and release. At exactly representable 750 Hz and integer operator ratios with neutral feedback, all dry reference samples match; EQ/compressor differ by at most 1 LSB. This does not establish bit equality for arbitrary old floating phases or feedback. SDK package tests: 3 passed; starter/timer/new card WASM release builds passed. Physical host audio audition and the full Rust suite were not run.
- Excel can deny ordinary reads while the supplied sheet is open; closing it resolved this session's access error. Python package tests needed TMP/TEMP pointing inside workspace on this sandbox. Use the bundled Python when extracting the register workbook; do not save it while reading unsupported sparkline extensions.
- Regular Release build could not overwrite `target/release/cpt32.exe` (Windows access denied). `cargo rustc --release --bin cpt32 --offline -- -C extra-filename=-3ws88pwn4` succeeded; copy `target/release/deps/cpt32-3ws88pwn4.exe` to `target/release/cpt32-3ws88pwn4.exe` for a separately verified host. Its monitor successfully loaded the real sound-card package and read its MMIO. Do not report the locked regular executable as rebuilt.
## Build identity in the emulator title (2026-10-19)
- `build.rs` embeds the reachable HEAD commit count (`git rev-list --count HEAD`) and UTC build-script execution time. Git-free source builds use `unknown`; shallow clones count only available history. Unchanged Cargo builds reuse the timestamp.
- Watch source/Cargo inputs and Git-resolved HEAD, symbolic branch ref and packed-refs paths so commits, branch switches and linked worktrees invalidate metadata. Keep `BUILD_TITLE` in startup, cartridge replacement and load-error titles; the package-drop regression checks the shared prefix. UTC calendar boundaries are tested by compiling `build.rs` with `rustc --test`.

## C PCG CLS (2026-10-19)
- `sdk/cpt32/src/pcg.c::pcg_cls` must clear the complete 4 KiB text plane at `pcg_vram_base` and fill both 4 KiB attribute planes (+0x1000 FG / +0x2000 BG) with the requested color. Updating only BG leaves colored glyphs visible; clearing text alone depends on writable glyph zero being empty. Equal FG/BG guarantees a solid fill regardless of font data or color swap.
- Preserve cursor/scroll state and subsequent output colors. The display C guest smoke checks all plane bytes in 40-column, 80-column and 8x16 modes after ring scrolling, then verifies that subsequent text uses the original output colors. The old implementation fails with R1=107 (stale text).
- Verification: full C SDK guest smokes passed at O0 on Scalar/Pipeline (5 guests each, DSFS zero leaks); display also passed at O2 on both CPUs. `cargo test --test vdp_pcg_scroll` passed (1). Point TMP/TEMP inside workspace when the sandbox denies the toolchain's default temporary directory.

## SGUB v3 card-write export (2026-10-20)
- `doc/spec_SGUB_v3.md` extends the v2 pooled container with 32-bit operation addresses and eight card-slot targets (3..10). The initial implementation supports slot 0 only; other slot targets are rejected. S3HS PCM RAM uses a type 3 pool and COPY_POOL into 0..0x3FFFFF; register/FIFO/Gate writes remain ordered individual byte WRITE operations.
- Furnace's SGUB exporter handles one S3HS system, transforms signed sample RAM to unsigned card PCM, and records S3HS writes at 48 kHz into 60 Hz v3 frames. The S3HS dispatch must emit register writes when register-dump mode is enabled; reset disables that mode, so enable it after playback initialization and clear stale writes first.
- SDK C playback maps target 3 to slot 0 at 0xA0000000 and must use byte MMIO for card bulk transfers. Rust StreamPlayer validates v3 and exposes ordered slot 0 writes through `take_card_writes`; GUI playback integration still needs to route those events into an installed card.
- The Music Editor exports its guest C source from `sol/cpt32/sgu_music_driver.c` at build time via `include_str!`; update that file to read the v3 total-size field at offset 32. Leave `sol/cpt32/sgu_music_driver.sol` on v1/v2 when only the C driver is requested.

## Bus per-cycle overhead reduction (2026-10-20)
- 最大のボトルネックは `Bus::refresh_device_irqs`（約29 ns）だった。tick毎2回＋バスアクセス毎1回（32-bit fetchで4回）呼ばれ、Pipelineの1 cycle ≈ 260 nsの大半を占めた。12回の `set_source` 呼び出しを `IrqController::set_source_levels` の一括bit演算にして3 nsへ。PeC Mutexやu128除算は単体では支配的でなかった（変更しても計測差なし）——推測より先に部品単位で計測すること。
- 実装と計測値は `doc/emulator-performance-plan.md`「Bus／デバイスのサイクル毎オーバーヘッド削減」。Pipeline合成 4.7x、`test.wax` 60 frames 3.96x。
- 新しい契約: `Device::is_plain_memory()` を true にするデバイスは副作用なし・fault無し・IRQ source不変でなければならない（BusがIRQ再サンプルを省き、32-bit word を1回の調停で処理する）。MMIOデバイスでは絶対に true にしない。
- `set_source_levels` の等価性は `irqc::tests::set_source_levels_matches_individual_source_updates`、timer/RTCのu64高速経路は `timing::tests::single_cycle_advances_match_one_batched_advance`（u64桁あふれ時のu128経路も確認）。
- このホストでのプロファイル: 非管理者なのでETW/WPRサンプリング不可。`src/bin` に一時probeバイナリや `#[ignore]` テストを置き、private関数を N 回呼んで ns/call を測るのが速かった（計測後に削除）。
- 既存の不安定テスト（変更前HEADでも失敗）: `cow::tests::rejects_crc_sequence_range_and_base_mismatch`、`tb_load_live` 6件、`strek_smoke::probe_strek_tron`、`strek_micro`（0–3件、ホスト入力スレッドとのタイミング依存）。
- `tinybasic.sol` の `tb_read_line` は「FIFO空を確認→`halt`」の間にキーが届くと、irq3がIRQC pendingを消してHALTへIRETし、PeCレベルがhighのまま新しいedgeが来ず永久HALTする lost-wakeup 競合がある。エミュレータが速くなると発生頻度が変わるので、`strek_micro` 等の失敗をCPU/Bus変更の回帰と即断しないこと。

## JIT dispatch bottlenecks, MUL/DIV and memory support (2026-10-20)
- JITがinterpreterより遅かった主因は生成コードではなくRust側の固定費だった。batch経路をrdtscで分解すると、命令ごとのdecode cache検証52%（世代lookupがdevice線形探索×2/命令）、SipHash dispatch 31%、register block同期12%、native code 6%。生成コードを速くする前に往復コストを測ること。詳細と数値は `doc/cpu-jit-design.md`「dispatchボトルネック、乗除算・memory対応」。
- 実ゲストではさらにcompile時間（test.wax 60 framesで183 ms）が支配的だった。strict関数は命令ごとに`prepare`するので解釈実行とほぼ同速なのにcompile時間の約63%を占めていた。batch関数は128 visitsで、strict関数はbatchを32回辞退した後に遅延生成する。release buildはCranelift IR verifierを無効化（debugでは有効）。合成ベンチだけでなく `src32_testbench test.wax --frames 60 --cpu jit` の実ゲストで必ず比較すること。
- 新しい契約: batch stampは `Cpu::instruction_cache_fills`（decode cache slot書込みで増加）と覆うpageの世代で有効性を判定する。cache slotを書き換える経路を追加したら必ず`instruction_cache_fills`を増やすこと（増やさないとstale命令をnative実行する）。
- batch内のload/storeは`batch_memory` helperが`defer_native_idle_cycles`で経過cycleを渡してから `execute_normal` を呼ぶ。plain memory以外はアクセス前に停止、IRQC出力変化か自block code pageの世代変化でアクセス直後に停止。連続batchはregister blockを同期せず連結するので、batch内からCPU register MMIOに触れる経路を作ってはならない。
- `RELAXED_DEVICE_WINDOW`（bus.rs）は256 cycle（以前32）。JIT block上限は`MAX_JIT_BLOCK_SIZE`=256（launcher/config/specも同期）。block sizeを大きくしても実ゲストでは遅くなった（重複blockのcompile増）ので既定16のまま。
- JITテストは `eager_tiers()`（`AnyCpu::set_jit_tier_thresholds(16, 0)`、doc hidden）で旧来の早期・両段階compileに戻してstrict経路を検証している。既定の遅延tierは `lazy_tiers_compile_batch_when_hot_and_strict_only_after_declines` が検証。
- テストの落とし穴: 毎iterationでcodeを書き換えるとdecode cacheが常にmissしてbatchが一度も走らず、自己書換え検出のテストにならない（mutationが通った）。書換えを稀にしてvalidated batchの最中に起きるようにする。またrun(7)のような小予算では16-cycle DIVがbatchに入らない。mutationでテストが実際に経路を通るか確認すること。
- Cranelift: `finalize_definitions`は約5 µsで支配的でない。`opt_level=speed`は効果なし。single-pass regallocはcompile約40%減だが実績が少ないため不採用。

## I/O-bound guests: waiting devices and spin-loop fast-forward (2026-10-20)
- IDCドライバ（sol／C）は転送完了をMMIOポーリングで待つため、DMA／IDC active中はrelaxed batchingもHALT skipも効かず、全CPU方式が10〜17 MHzだった。ボトルネックは毎回4 byteに分かれるMMIO状態読み出しと、activeなIDC/DMAの毎サイクルtick。数値は `doc/emulator-performance-plan.md`「I/Oバウンド処理」、ベンチは `cargo run --release --features jit --bin idc_bench`。
- 新しい契約: `Device::peek_u32_be` は「その4 byteの読み出しに副作用がなく、faultせず、IRQ sourceも変えない」場合だけSomeを返す（DMAC全レジスター、IDCはDATA以外）。これはBusの1回調停読み出しとスピン検出の「純粋な読み出し」判定の根拠になる。読み出しで状態が変わるレジスター（PeC timerのlatch、FIFO pop等）には絶対に実装しない。
- `Idc::cycles_without_event` は次のtickが何も起こさない残りcycle数。`advance_idle_cycles` はactive中でもこの範囲内なら使える（範囲外はassert）。
- スピンループ早送り（spec_CPU §1.4）は全CPU方式で既定ON。正しさの前提は「probeから現在までの変化がCPU実行だけで生じた」こと。ホストがレジスター／PC／IRQ入力を書き換える公開APIは必ず `forget_spin()` を呼ぶこと（`write_reg`だけでなく`set_pc`・`set_instruction_mode`・`set_irq_input`・`set_irq_source`）。Cpuの内部writebackは`set_gpr`、公開`write_reg`はprobe破棄付き。これを忘れると `run(1)` を繰り返すテスト（JALR R31）でJALRが飛ばされた。
- Pipelineのループ先頭は「後方へのtaken redirect」に限定し、probeは先頭PCごとに8個。sol関数呼び出しのpoll loopは1反復で約9回taken branchするため、redirect先ごと1個・後方フィルタなしでは一度も一致しなかった（実ゲストでPipelineだけ改善ゼロ）。合成ベンチの単純ループだけで判断しないこと。
- 検証は `tests/spin_fast_forward.rs`（早送りON/OFFのlockstep差分、timer IRQ併用、関数呼び出し型poll、ホスト書き換え）。`AnyCpu::set_spin_fast_forward` と `spin_skipped_cycles` はdoc hiddenのテスト用hook。timer期限・IDC期限を無視するmutationはどちらも検出される。
- sol小ネタ: `while` 本体は末尾で0（継続）／1（終了）を積む。ループ内の分岐に `ret` を入れると「stack underflow at jz」になることがある。`$TMPDIR/..` は `AppData/Local` でありscratchpadのTempではない（Writeツールの出力先と混同した）。


## Launcher JIT presets (2026-10-20)
- Launcher presets are Small=16, Medium=32, Big=64 instructions. Keep preset selection, numeric-to-preset detection, initial state and config/profile loading aligned. The emulator default remains 16 (now Small), and custom values remain 2..256.
- Launcher help must reflect the 256-instruction limit. Larger blocks can increase generated code and compilation time and may run slower; preset changes do not change JIT safety contracts.

## Timing-insensitive MMIO devices (2026-10-20)
- `Device::is_timing_insensitive()`（spec_CPU §1.5）: IRQ sourceでない・faultしない・値がシステムクロックやtickされるデバイスに依存しない・アクセスがそれら／IRQ source／CPUを変えない、の約束。RAM/ROM（plain memoryは暗黙に該当）、VDP（VRAM/regs/SGC）、VPU、SGU（regs/PCM RAM）、RNG、UARTが宣言。DMAC・IDC・BMC・IRQC・PeC・CPU regs/vectors・拡張カードは宣言しない。宣言は登録時に`DeviceMap`へキャッシュされるので、デバイス生存中に変えてはならない。
- 用途: relaxed／JITでflushしない（RAMも含む＝relaxed区間がRAMアクセスで切れなくなった）、全方式でIRQ再確認を省く、DMA要求がなければ32-bitアクセスを調停1回、JIT batch内でnative実行。
- 検証: `tests/timing_insensitive_mmio.rs` がフラグON/OFF（`set_timing_insensitive_fast_paths`、doc hidden）を全CPU方式でlockstep比較（VRAM/PCM RAM/VDP/timer IRQ/timer counter読み）。PeC timerを誤って宣言するmutationはCachedRelaxedで検出される。新しいデバイスにフラグを付けるときはこのテストにアクセスを追加すること。
- 落とし穴: RAMでflushしなくなるとJIT batchが区間を使い切って辞退→strict tierの遅延compileが増え、cubeでJITが遅くなった。非連結batch開始時に区間切れならflushして新区間を開き、連結中の区間切れはchain終了だけにする（辞退扱いしない）。`lazy_tiers…`テストは「別entryがbatch tierを得ることもある」前提に緩めた。
- 計測: `mmio_bench`でJIT 61→16 ms、relaxed 50→34、Cached 61→49、Pipeline 97→86。`cpu_bench`の2〜5%差はbuild間揺れと同程度で、Bus primitiveは単体で速い（layout差と判断、要追跡）。untrackedファイルは`git stash`で退避されない（`-u`が必要）ので、base比較で新規benchを消さないこと。


## JIT startup: Windows GOT/PLT distance panic (2026-10-20)
- `cranelift-jit 0.117.2` PIC mode allocates GOT and PLT independently and unwraps an i32 displacement in `write_plt_entry_bytes`. Windows address placement can exceed the signed 32-bit range, causing intermittent `TryFromIntError` during helper declaration, even in ALU-only batches.
- CPU JIT now constructs the native ISA explicitly with `is_pic=false` and `use_colocated_libcalls=false`, then uses `JITBuilder::with_isa`. On x64, non-colocated helper calls use full-width addresses without GOT/PLT. Passing false to `with_flags` does not work: that constructor overrides is_pic to true internally. Preserve this when upgrading Cranelift or changing module setup.
- Both startup and cache reset use `create_module`; keep their settings identical. Keep debug IR verification and Cached fallback intact.
- `repeated_startup_and_reset_execute_both_native_tiers_with_helpers` covers ALU-only and memory-helper blocks over repeated creation/reset and compares state with Cached. It exercises both tiers, but does not force the failing ASLR placement. Focused release tests: cpu_jit/cpu/cpu_select/cpu_pipeline/dos_smoke, 83 passed.
- 実ゲスト検証: config.yaml のない `target/jit-startup-smoke` を cwd にし、release `src32_testbench test.wax --cpu jit --frames 5 --allow-running --skip-render-checks` を別プロセスで10回実行。全回4,000,000 cycles、92 batch compilations、compile_failures=0。描画／GUI検証ではない。ログにあった一時 program.wax は既に消えていた。

## VPU terrain flight C port (2026-10-20)
- `examples/vpu-terrain-flight-c` ports the sol demo to LLVM/SRC32 C (-O2). Terrain is periodic world-space smooth lattice noise; never derive vertex heights from frame phase or reset movement after 84 frames. Camera-relative widening rows cover the perspective frustum, and a one-cell height halo supplies cached slope shading. Generate vertices before clearing the previous image.
- Q10-to-binary32 normalization must include the -10 scale: exponent starts at 126 when normalized magnitude is 512..1023. Check that Q10 1024 becomes 0x3F800000. For a +Z camera looking down by positive pitch, view Y uses +sin*pZ and view Z uses -sin*pY; the opposite signs look upward and hide most ground below the screen.
- Read the actual VPU command interpreter, not just the full spec: SET_STATE currently accepts only IDs 0/1. CLEAR currently ignores its supplied depth and clears to 1.0. Send the spec-correct 1.0 anyway. Sky depth must be below 1 and behind terrain (the demo uses 0x3F7FFF00), without unsupported Z-state switches.
- SGC FIFO has 256 words and glyphs consume four words. Poll FIFO_LEVEL at 0x80010024 (low 16 bits free words) before each complete glyph. Status bit 3 is a sticky overflow error; it is not a live full predicate. Long help lines otherwise silently lose trailing text.
- Real guest regression: build the documented WAX, then `cargo test --release --test vpu_terrain_flight -- --ignored --nocapture`. Cached/Pipeline both validate visible green terrain (22377 pixels), sky/full coverage, VPU/SGC status, pause, held turn/release, flight and reset. This checks a completed frame, not 60 fps: a bounded CPU drain without another host IRQ finishes the current draw. GUI keyboard operation was not manually exercised.
- PeC input runs on a separate thread. Tests use a following MousePosition event as an ordering barrier before running the guest, so a batch such as R-down/up then Space-down/up cannot straddle guest updates nondeterministically. `Vdp::framebuffer()` renders/copies the SGC plane: call it once per snapshot, never once per pixel.
- Clang 20 -O2 can turn unsigned edge-neighbor clamping into unsupported llvm.usub.sat.i32. The height halo avoids this pattern without expanding the compiler scope. CLI and CMake/Ninja builds both passed. On this Windows sandbox set TMP/TEMP to an existing workspace directory for Python temporary output.
