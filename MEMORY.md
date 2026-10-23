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
## Windows shutdown crash investigation (2026-10-20)
- `0xC0000409` は名称だけで stack buffer overrun と断定しない。今回の WER / minidump は `atio6axx.dll` 32.0.31041.1004 の `+0x12AC465`, parameter 7 (`FAST_FAIL_FATAL_APP_EXIT`)。別ビルドでも同位置。OBS graphics-hook64.dll もロードされていたが関与は未確認。詳細と Report ID は `doc/shutdown-crash-investigation.md`。
- `GuiApp` は window → presenter → debug_gui の宣言順で自動 Drop される。unsafe 作成した wgpu Surface より先に Window を破棄するため lifetime 契約違反。修正候補だが AMD fail-fast の直接原因と実証したわけではない。描画資源 → Surface → Window の順を保証し、同じ条件で終了を比較すること。
- `InstanceDescriptor::new_with_display_handle` は WGPU_BACKEND を読まない（from_env / with_env が必要）。全 backend 初期化で GL ドライバーがロードされても、表示 backend が GL と断定できない。今回ランタイムコード変更・GUI再現・完全なstack unwindは未実施。
- 続く修正で `GuiApp::release_graphics` を `exiting` / `Drop` から呼び、debug GUI → presenter/Surface → Window の順に解放するようにした。`Option::take` により再呼出しは安全。元バイナリ1回、修正版3回（Alt+F4 / Escape / close button）は全て終了コード0で、元障害は比較時に再現せず、完全解消の証明ではない。通常cwd/config/TinyBASIC/JITを使用し、修正版のみRUST_LOG=info。adapterはAMD Radeon RX 7600/Vulkan。ビルド・対象2ファイルrustfmt・diff check通過。詳細は調査doc末尾。
## Launcher log flooding (2026-10-21)
- 原因: `_poll_process` が空になるまで無制限にキューを消費し、各行でTk Textのconfigure/insert/seeを呼んでいた。生成が速いとイベントループに戻れず、キューとTextも無制限に増大した。
- `ProcessEventQueue` は2048件。lineだけ満杯時にdropし、省略数をmutex保護で回収してGUIに表示する。phase/started/exited/errorは通常のblocking putで必ず届ける。GUI pollは最大256件/約64k文字を1回のText更新にまとめ、backlog時も10msタイマーで必ずTkに戻る。Textは末尾200k文字。child stdoutはreadline(8192)で改行なし巨大出力も分割。コンソールはGUI省略とは独立して出力する（遅いコンソールはreader workerを詰まらせ得るがGUIスレッドでは書かない）。
- 回帰は `python -m unittest discover -s tools/cpt32-launcher -p 'test_*.py' -v`。100万文字の改行なし出力＋10万行の実subprocess、満杯queueのdrop/終了通知保持、pollのyield、実Tk Textとtimer heartbeat/履歴上限を確認する。WindowsでTMP/TEMPをworkspace内に設定すること。
## sol PCG CLS parity (2026-10-21)
- sol版 `pcg.cls` は固定BG領域だけを書き換えていた。C SDKの`pcg_cls`は既に全3plane対応済み。sol版もmutableな`pcg_vram_base`からtext/FG/BG各4KiBを0/color/colorで埋める。40/80/80x16に共通で、表示外ringセルを含めて消し、glyph0が非空でも単色にする。cursor/scroll/output色とfont領域は保持。
- `test_sol_vm.py` の6ケース（3モード×標準/変更base）は修正前にstale textで失敗し、修正後通過。`tests/pcg_cls.sol`を`solc.py compile -O speed`→`asm.py`でWAX化し、空cwdのrelease src32_testbenchでScalar/Pipelineとも64frames、R1=42、running=falseを確認。`--skip-render-checks`のbyte/state検証で、GUI描画検証ではない。VM/compiler/LSPは165件通過。ゲストWAXはライブラリ変更後に再ビルドが必要。
- WindowsのPython3.15では親のstderr handleが無効だとLSP子プロセス起動がWinError6になることがある。テストを別Pythonの`subprocess.run(stdout=ログfile, stderr=STDOUT)`で起動すると有効なhandleを継承できる。TMP/TEMPもworkspace内を指定する。

## SGUB unoptimized frame completion regression (2026-10-21)
- The previously recorded SGUB -O0 frame-0 failure was an observation-time bug in the regression: 50M startup cycles completed on Pipeline and the guest was HALTed awaiting IRQ, but the dense first IRQ exceeded the 800K frame budget. At the first frame it was still executing; the next CPU interval returned to the waiting HALT. Do not treat this as a wrong frequency calculation or fix it by sending synthetic CPU-cycle-driven VSyncs.
- `src32_testbench --irq-drain-cycles N` optionally runs up to N extra CPU cycles after each normal frame until `irq_enabled()` is restored. It adds no VDP tick or host IRQ; if the handler never returns (or IRQs remain disabled), it fails with the frame, budget and PC. Default is disabled, preserving real-time-budget tests. This verifies completed guest state, not playback throughput; SGUB uses 40M as a bound and still checks memory every frame.
- Furnace v2 SGUB now passes -O0/-O1/-O2 on Pipeline; LLVM toolchain full suite passes 22/22 with the rebuilt release testbench. The new stuck-IRQ test fails as intended on Scalar/Pipeline. Expanding the real-stream test to Scalar exposed a separate startup limitation: its -O0 full PCM init does not finish within the harness's fixed 50M startup budget, so the stream regression intentionally remains Pipeline-only.
- Rebuild `src32_testbench` before running the updated Python suite; an older binary rejects the new CLI flag. `TMP`/`TEMP` must point at an existing writable workspace directory. The SDK/guest player and production emulator timing were unchanged.

## SoftFloat C demo and LLVM saturating subtraction (2026-10-21)
- `examples/softfloat-c` vendors the unmodified binary32/int32 subset from InfiniBrains/softfloat parent `5f544d63f2b9859c7105c49935c800e5c0e717b4`'s `berkeley-sw` submodule `b64af41c3276f97f0e181920400ee056b9c88037` (actual Berkeley Release 3e). The parent's root sources retain mixed 3c/3e headers and float8 extensions: do not substitute those and label them vanilla 3e. Keep COPYING and UPSTREAM provenance.
- SRC32 port must leave LITTLEENDIAN undefined (not defined to 0), use INLINE_LEVEL=0 with portable out-of-line primitives, and avoid host intrinsics/TLS. The 8086 specialization is a NaN/exception policy, not an x86 instruction dependency; default NaN is 0xFFC00000. The vendored headers declare APIs beyond the linked subset.
- Clang 20 -O1/-O2 emits `llvm.usub.sat.i16` for upstream `softfloat_subMagsF32`. Backend now lowers unsigned saturating subtraction for i1..i32/i64: clamp underflow to zero; for i64 compare both high words and low-word borrow before masking both result words. Direct IR tests cover 75 boundary cases, including non-C widths, unsigned sign boundaries and cross-word borrow, on Scalar/Pipeline.
- SoftFloat guest checks 14 arithmetic/conversion/rounding/exception cases plus PCG text VRAM, returns 42, and passes -O0/-O1/-O2 on Scalar/Pipeline. CMake uses -O2 and also passes both CPUs. These are guest byte/state checks, not GUI pixel comparison. Flags are sticky: clear them between cases. All 31 copied upstream files were byte-identical when verified.
- Full LLVM suite: 20/21 pass. SGUB -O0 still fails frame-0 memory 0x80020801 (0 instead of 131); a fresh build using HEAD's unchanged backend reproduced the same failure, independently of the new intrinsic path. Keep that failure distinct from SoftFloat support. Testbench expected R1 checks require omitting --skip-render-checks and running in an empty cwd without config.yaml.

## C++17 LLVM/SRC32 sample (2026-10-21)
- `examples/hello-cpp` is a multi-TU freestanding C++17 guest, built by the existing driver/CMake module together with C SDK sources. `.cpp/.cc/.cxx` select C++17; `.c` and the bundled string runtime retain C11. Clang 20 needs `-Xclang -mno-constructor-aliases`: ordinary out-of-line constructors otherwise emit unsupported LLVM aliases. Keep exception/RTTI/threadsafe-static/cxa-atexit flags scoped to C++.
- Freestanding Clang mangles even `main`; declare `extern "C" int main()` and wrap C SDK includes in `extern "C" { ... }`. Define methods out of line to avoid weak/COMDAT. C++ standard library, allocation runtime, dynamic global initialization and shutdown destructors are not provided. The dynamic-global regression deliberately checks rejection of `llvm.global_ctors` appending linkage; never silently skip constructors.
- The sample checks its class result and PCG text VRAM, then returns 42. C++ extension/optimization regression covers -O0/-O1/-O2 on Scalar/Pipeline; the documented CMake build also returns R1=42 on both. This is byte/state validation, not GUI pixel comparison. `src32_testbench` has no UART device at the C SDK's 0x80040000, so calling `print_prnln` there causes a bus error; use PCG for the sample. `--skip-render-checks` also skips the expected-R1 assertion, so omit it when validating this returning guest.
- Full LLVM suite: 18/19 passed with Clang 20.1.8 and the existing release testbench. SGUB -O0 failed frame-0 memory 0x80020801 (0 instead of 131). HEAD's unmodified driver reproduced it and emitted a byte-identical WAX (SHA256 22ce28a1e361bfd729701b97e36e3c1f6298fc5b8e645225aa1d08abeeedca8f); keep that pre-existing failure distinct from C++ support.

## C SDK function summaries (2026-10-21)
- `sdk/cpt32/include/cpt32/*.h` の公開93関数には宣言直前に日本語の短い説明がある。詳細なバッファ・戻り値・IRQ等の既存契約コメントも維持する。新規APIや挙動変更では説明も実装と同期する。
- VPU配列はbinary32値そのものではなくuint32_tのビット列。DSFS bitmapのset/clearはメモリ上の操作で保存は別、write_fileはsrcのセクタ末尾をゼロ埋めする。SGUB初期化はv1/v2/v3を受理する。
- 説明追加のみの場合は、コメント除去後のヘッダーとHEADが一致すること、全宣言の説明網羅、ClangのC11構文検査でコード不変を確認できる。

## External-crate PSG WASM card example (2026-10-21)
- `examples/wasm-psg` uses the registry crate `psg = "=1.0.1"` (thedjinn/psg-rs), not a copied synthesizer. Adapter is `#![no_std]`, but upstream links std and supplies the panic implementation: adding a custom panic handler causes duplicate definitions. `wasm32-unknown-unknown` release builds load under the existing ABI import allowlist without WASI. Always validate the final module in the real host; a successful Rust build alone does not prove ABI compliance.
- MMIO R0..15 are byte latches, address/data ports are 0x10/0x11, model 0x12 is 0=YM / 1=AY, ID at 0xF0 is `PSG-RS01`. Debug reads only inspect latches. R14/15 have no external GPIO. Reset clears all synthesis/filter/resampling state and uses R7=0x3F with zero amplitudes. Repeated R13 writes must reach upstream to retrigger the envelope.
- Upstream's byte period setters clamp zero to 1 after each write. Reassemble tone and envelope periods from the adapter's two raw latches before calling the full-period setter, so writing a zero low byte then a nonzero high byte does not accidentally produce period+1. Native tests compare actual samples with upstream at the intended full period.
- Fixed 1,789,772.5 Hz clock / 48 kHz synthesis, centered mono duplicated to stereo, scaled by 16384 to i16. Integer-phase zero-order hold preserves block continuity and rescales phase on host-rate changes. Rates below 8 kHz return silence without advancing state to bound work; downsampling has no extra anti-alias filter. Host audio rendering does not advance Bus cycles.
- Validation: 4 native tests; built-package `CPT32_PSG_CARD_PACKAGE=... cargo test --release --test wasm_psg -- --ignored --nocapture` passed both models with 3 tones+noise+envelope at 48/44.1/8/96 kHz under normal fuel/memory limits, readback and reset. Existing WASM suite: 22 passed / 1 unrelated built-card test ignored. Package builder: 3 passed after setting `tempfile.tempdir` to a writable workspace directory (default Windows sandbox temp denied access). Physical audio playback was not tested. Build/package/config commands are in the example README.

## MIT scope and third-party notices (2026-10-21)
- Root LICENSE covers original CPT32 code; external fonts/code keep their own terms. Unifont 18.0.01 attribution is now verified against the official release archive (see follow-up below); its data and generated ROM retain OFL-1.1. Owner confirmed BIOS and sound.cpp/sound.hpp are original, and subsequently confirmed 3WS88PWN4 is their own project and may be included under MIT.
- scripts/generate_license_notices.py uses offline locked root Cargo all-features/all-target resolution and sol-extension production npm dependencies, retaining full texts including nested Dear ImGui and inline stb licenses. Reviewed expressions are explicit; unknown expressions or missing notices fail. licenses/overrides.json preserves revision-pinned upstream texts omitted from published crates, or explicitly labeled standard-text supplementation. r-efi stores its MIT terms in AUTHORS. Cargo.lock remains ignored: regenerate notices after resolution changes. MPL option-ext requires covered source availability; keep its source URL in distributed notices.
- GUI Help > About / Third-party licenses embeds the notices, with ListClipper and cached lines to avoid laying out the whole long document every frame. VSIX folders/SDK/starter carry LICENSE; sol extension also carries notices. This is a text inventory/check, not legal certification.
- IBM CGA.F08 was removed at owner request. UNSCII.F08 derives from pinned upstream UNSCII-8 using tools/build_unscii_font.py, preserving CP437 slots and 2048-byte bank layout. NUL blank; missing 0x0F/0x7F/0xA9/0xF9 use ?. Do not substitute GPL unscii-16-full. tests/unscii_font.rs verifies ASCII/block, both 8x8 banks, and no overwrite of separate 8x16 storage. vdp_pcg_scroll and unscii_font each passed with all-features; cargo check and both generator --check passed. GUI visual verification was not performed.

## 3WS88PWN4 MIT ownership confirmation (2026-10-21)
- Owner explicitly confirmed 3WS88PWN4 is their project. Treat examples/wasm-3ws88pwn4 as covered by the project MIT grant; its Cargo manifest and standalone LICENSE now agree. Remove the earlier provenance-pending warning from generated notices as well as the compliance document. The separate Unifont release-notice issue was subsequently resolved against the official archive below.

## Unifont 18.0.01 release attribution verified (2026-10-21)
- The release-specific guessed LICENSE URL returning 404 did not mean attribution was unavailable. Follow the official unifont/index.html -> unifont-utilities.html -> versioned tar.gz. Read specific members with tarfile.extractfile (no filesystem extraction). The release BDF COPYRIGHT property lists the actual 1998-2026 font holders; common LICENSE placeholders are not font-specific rights metadata.
- The bundled gzip decompresses byte-identically to release font/precompiled/unifont-18.0.01.hex (SHA256 2ab7801c809b76541a10b25858c1a962bdb650eee56b4c7df1f325b56e3955ee). licenses/UNIFONT-18.0.01-NOTICE.txt preserves the release archive hash, provenance and BDF header. licenses/UNIFONT-OFL-1.1.txt preserves its official OFL text, normalizing upstream CR-CR-LF and trailing spaces only. Include both in generated distribution notices.
- Unifont attribution is resolved; remove stale UNVERIFIED warnings. Keep font data and converted ROM under OFL-1.1, original CPT32 code under MIT. Verification: tools/unifont_hex_to_chr.py --check assets/unifont-18.0.01.hex.gz assets/unifont-bmp.chr, notice generator --check, and cargo check --bin cpt32 --all-features --offline.
- WASM カードのデバッグ表示 (`--debug-gui` のメニュー `Expansion Cards`): fuel 統計は `ExpansionCard::call_guest(kind, ..)` が呼び出し毎に `GUEST_FUEL - get_fuel()` を entry point 別 (`ExpansionCallKind`) に累計する。trap/fuel 枯渇でも計上し、枯渇は上限全体になる。start function と `cpt32_abi_version` は load 時に `Start` へ合算。新しい guest 呼び出し経路を足すときは必ず `call_guest` を通し kind を割り当てること (直接 `.call` すると統計から漏れる)。GUI は `Bus::expansion_card_debug_info()` (Device trait の既定 None、カードだけ Some) を毎フレーム取り、前フレームとの差分で per-frame 値を出す。debug_read (Memory Monitor) も計上される。`ExpansionCard.label` は config 経由だと解決済み path になる点に注意 (テストは ends_with で比較)。検証: `cargo test --test wasm_expansion`。GUI 表示自体は自動テストなし。

## Extension P: privilege levels and SYSCALL (2026-10-21)
- Spec: doc/spec_CPU_priv.md (ja: doc/ja/spec_CPU_priv.md). Shared logic lives in src/cpu_privilege.rs (`SystemState` holds IE/PIE/U/PU/PSM, EPC, CAUSE, the non-current R28..R30 bank and 8 regions). Every CPU kind keeps its trap state only in `sys`; do not reintroduce separate irq_enable/epc/cause/irq_return_mode fields. CAUSE is u32 (`irq_cause()` returns u32).
- Opcodes: 0x23 SYSCALL imm16 (bits 7..0 number, 15..8 context, copied into CAUSE 0x8003_xxxx; EPC = next instruction; vector word 0xFFFF000C), 0x24 MFSR rd,sr, 0x25 MTSR sr,rs1. Exceptions set CAUSE bit31 with code in bits 23..16; IRQ CAUSE stays 0..15 so the sol dispatcher still works. Bus error CAUSE changed from 2 to 0x8002_0000. Illegal/privileged share vector 0x04, bus error/protection share 0x08. Unchanged self-referencing exception vectors halt the CPU (no host panic). Illegal instructions no longer panic anywhere.
- Banking is implemented by swapping reg[28..31] with `other_bank` at trap entry/IRET, so `reg` always holds the current mode's bank and engines/JIT need no bank awareness. USP/UFP/UGP always mean the user copy.
- User-mode checks are done by the CPU before any bus access (`SystemState::allows`), every byte must be granted, 0xFFFF0000.. is always denied, regions reset to zero (default deny). Scalar/Cached check fetch in the Fetch state even on decode-cache hits.
- Pipeline: SYSCALL, MTSR, IRET and every exception are handled at commit and flush the whole pipeline (`redirect_empty`), so mode/bank/regions never change under in-flight instructions. IRET no longer redirects at EX. After a flushing commit, call accept_pending_irq before returning so IRQ timing matches the scalar CPU. MEM-stage protection faults trap before the access. Denied fetches are carried as `IfId.fetch_fault` and are not counted as executed instructions.
- JIT runs native blocks only in S mode (`run_jit_block` breaks on `sys.user`); native code does not check protection. The fallback-panic JIT test was replaced by an illegal-instruction trap test.
- IRET restores IE from PIE (was always 1). Exceptions keep the pending IRQ latch (bus error used to clear it).
- Boot policy (owner decision): reset is always S; user mode is opt-in through a PROTECTED .wax and the BIOS WAX loader does the S->U hand-off. Host direct loads (main/testbench/monitor) call `wax::reject_protected_direct_load` and refuse protected images rather than running them unprotected. A BIOS image itself must not be PROTECTED. The owner wants the BIOS rewritten in C with a WAX loader; that is not done yet.
- WAX PROTECTED (flags bit1): 16-byte block entries with perm R/W/X at +12, user_stack_top/size at header 48..56, max 7 blocks, entry block needs X, stack must not overlap blocks or reach 0xFFFF0000. `WaxImage::user_regions()` / `user_regions(image)` define the hand-off: region 0 stack RW, region 1+i block i. Python `parse` adds a block "perm" key only for protected images so existing dict comparisons stay valid. sol/cpt32/wax.sol still rejects the flag.
- Tools: asm.py SYSCALL n[, ctx], MFSR rd, NAME|num, MTSR NAME|num, rs (unknown SR numbers are assembler errors); disasm.py names SRs. CPUID: 0x13F scalar/cached/JIT, 0x1BF pipeline.
- Verification: tests/cpu_privilege.rs covers all CPU kinds (JIT with --features jit); cargo test --features jit for cpu, cpu_pipeline, cpu_select, cpu_jit; src/wax.rs unit tests; pytest tools/asm/test_asm.py tools/wax/test_wax.py. Pre-existing unrelated failure: cow::tests::rejects_crc_sequence_range_and_base_mismatch fails on the clean tree too.
- Baseline check (clean HEAD worktree, same tinybasic.bin): 4 tb_load_live tests, the two main.rs package_guest_builder tests and cow crc test already fail without Extension P; strek_smoke probes need /Users/src3453/... absolute paths. Use `git worktree add` for baselines; never run `git stash` while a background build/test is using the tree.
- ISA grouping (owner): `IRET` belongs to Extension I, not the Base ISA. Base `HALT` only stops execution; IRQ wake-up and resuming via IRET are specified under Extension I (§4.6) in spec_CPU.md / ja.

## LLVM/SRC32 Extension P support (2026-10-21)
- `<cpt32/cpu.h>` intrinsics are lowered inline by tools/llvm-src32/backend.py (`privilege_intrinsic`): cpt32_syscall(number, context, a1..a4) (number/context must be integer constants 0..255; a1..a4 in R1..R4; R1 returned), cpt32_mfsr/cpt32_mtsr (constant SR from the known set), cpt32_run_user(entry, usp), cpt32_return_to_kernel(code), cpt32_halt. All names in `_INTRINSICS` are reserved (defining one is an error).
- cpt32_run_user saves kernel R28/R29/resume label/STATUS in `__llvm_src32_kernel_context` (4 words emitted after the code only when used), sets USP/EPC and STATUS = PU | (IE<<1), then IRETs. cpt32_return_to_kernel restores R28/R29, EPC = resume, STATUS = saved IE<<1 (PU=0) and IRETs with R1 = code. Not reentrant; user entry must never return (LR is the kernel's).
- `void syscall_dispatch(cpt32_trap_frame *)` makes startup store the stub address in 0xFFFF000C. The stub uses a 136-byte frame on the S stack: [SP] = frame pointer argument, frame word 0 = CAUSE, word n = Rn, word 28 = 0. Restore loads R1..R31 except R28 from the frame, so handler writes to r[1] become the SYSCALL result.
- `--protected` (CMake `PROTECTED`, `USER_STACK_SIZE`): driver splits assembled blocks at `info["data_start"]` (code R/X, data R/W), packs PROTECTED WAX with stack (stack_top, user_stack_size), startup ends with `SYSCALL 0` (toolchain exit convention) instead of HALT, and rejects supervisor intrinsics plus irqc_dispatch/syscall_dispatch. Such images cannot run until the BIOS WAX loader exists; tests only parse them.
- A user program inside a single kernel image may only execute its granted code range: keep user code free of globals, string literals and calls (memcpy/SDK), and bound it with marker functions defined in order (`user_main` ... `user_code_end`); the sample checks the order at runtime.
- examples/syscall-user-c runs on src32_testbench (R1=42) at O0/O1/O2 on scalar/cached/cached-relaxed/pipeline, via CLI and CMake. test_toolchain.py: test_user_mode_syscall_example, test_user_mode_store_to_kernel_memory_faults (direct store faults -> R1=1), test_protected_wax_output, test_extension_p_intrinsic_diagnostics. Full toolchain suite (26) and sdk/cpt32/smoke/run.py pass.

## Optional external Newlib portable profile (2026-10-21)
- `tools/llvm-src32/driver.py --newlib-source ROOT` opts into separately obtained Newlib sources (`ROOT/COPYING.NEWLIB`, `ROOT/newlib/libc`). Do not vendor, automatically download, or commit upstream headers/sources/binaries. Tested release: 4.5.0.20241231, archive SHA256 33f12605e0054965996c25c1382b3e463b0af91799001f5bb8c0630f2ec8c852. Linked guest distributions still need the applicable upstream notices; external acquisition does not remove those terms.
- `newlib.py` maps supported unresolved functions to portable string/stdlib/search source files, compiling them at O0 with PREFER_SIZE_OVER_SPEED, without application defines/includes. Headers are CPT32 newlib.h configuration -> upstream libc/include -> Clang resource headers. Default freestanding mode remains separate. This is a limited profile: no printf/stdio, strtol/locale, files, libm or full Newlib ABI support.
- Compile upstream nano-mallocr.c directly in standalone mode (! _LIBC) with DEFINE_MALLOC/FREE/CALLOC/REALLOC/MALLOC_USABLE_SIZE and a port header mapping its one-argument _sbrk_r to cpt32_newlib_sbrk. Do not link ordinary reentrant malloc wrappers/impure.c or fake a partial struct _reent. Port uses single-context errno and an aligned static bounded heap (default 64 KiB; --newlib-heap-size, multiple of 8). Heap is part of WAX data and initial stack overlap validation; allocator is not IRQ-safe. Mixing custom allocator functions with the nano allocator is diagnosed.
- calloc needs llvm.umul.with.overflow.i32: detect overflow before truncation, handle zero without DIVU, and store the aggregate i1 at its byte offset (big-endian i1 is not a low-byte 32-bit value). Nano chunk offsets require integer add/sub constant-expression relocation. LLVM can upgrade comparison idioms to llvm.scmp/ucmp.i32.i32; lower as (a>b)-(a<b).
- CMake src32_add_executable accepts NEWLIB_SOURCE and NEWLIB_HEAP_SIZE. Depfiles include external sources/headers and COPYING.NEWLIB. Compile external nano source directly rather than generating a temporary C wrapper, otherwise a deleted wrapper can force every Ninja build to rebuild. Runtime dependency glob includes nested port C/header files.
- Validation: 31 toolchain tests passed with SRC32_NEWLIB_SOURCE set, including external Newlib O0/O1/O2 x Scalar/Pipeline, followed by an additional direct scmp/ucmp regression. Guest examples/newlib-c returns R1=42 and covers supported string APIs, sorting/search, div families, zeroed/aligned allocations, realloc preservation/failure, calloc overflow, ENOMEM and free-block reuse. CMake build, no-op rebuild, and Pipeline execution passed. Tests never fetch upstream sources; external tests skip when SRC32_NEWLIB_SOURCE is unset. Use writable workspace TMP/TEMP on this Windows host and an isolated cwd without config.yaml for testbench.

## WAX/WAL v3 specification (2026-10-21)
- doc/spec_WAX_v3.md and doc/ja/spec_WAX_v3.md specify v3; parsers, assembler, C toolchain and loaders still implement v2. Do not claim v3 runtime support or convert files by changing only version. Existing v2 contracts are preserved.
- v3 WAX and WAL both require PROTECTED and run in U. BIOS/kernel supervisor images stay v2/raw. Eight Extension P regions are a process-wide budget: region 0 stack, at most seven ranges for WAX and the entire WAL graph. Merge only adjacent equal-permission blocks, never holes or permission unions; reject overflow instead of granting RWX/S mode.
- Library identity is ID plus ABI major, separate from path, container version, public call ABI and OS syscall ABI. Resolve direct dependencies and imports before execution; one provider per ID/process, maximum minor requirement, no cycles or implicit transitive symbol lookup.
- v3 ABI 1 is new, not the current LLVM/sol ABI: module origin in user R30; 8-byte function descriptors carry address plus origin, including callbacks. Read both descriptor words before replacing R30 and restore caller R30 after return. Imported slots are RW and provide no isolation between a process and its libraries.
- Validation requirements and a concrete binding/region example are in section 8. They describe future implementation tests, not completed execution tests. Specification-only work must report document checks separately from v3 runtime validation.

## C BIOS boot loader (2026-10-22)
- assets/bios.rom is now built from the C BIOS `bios/main.c` with `python bios/build.py` (driver -O1, origin 0xFF000000, `--data-origin 0x000F0000`, stack top 0x00100000, SDK pcg/idc/irqc only; ~23.6 KiB of 64 KiB). sol/bios.sol is the legacy version without boot loader; its header commands overwrite the ROM. Contract: doc/spec_BIOS.md.
- Boot order (owner decision): RAM 0x0 (first u32 != 0, host-loaded) -> ROM (reserved, owner put it on hold; always skipped) -> IDC dd0..dd3 boot sector. IDC: IDENTIFY, READ LBA0 x1 into 0x00000000, require 0x1FE=0x55 0x1FF=0xAA (same as DSFS block 0). On read error/no signature BIOS zeroes 0..0x1FF so the attempt never looks like a RAM program. Nothing bootable -> IRQs off + HALT loop.
- Hand-off via new intrinsic `cpt32_jump(entry, r1, sp)`: R1 = device (0..3 IDC drive, 0xFFFFFFFF RAM), SP = 0x000F0000, FP = 0, IRQC all disabled + acked, vectors still point at the BIOS dispatcher. BIOS work area 0x000F0000..0x000FFFFF is clobbered during POST, so host-loaded RAM programs must fit below 0x000F0000.
- `--data-origin` (CMake DATA_ORIGIN): LLVM `constant` globals stay in ROM after code; writable globals go to RAM, initialized ones first, then zero ones (detected syntactically: zeroinitializer/0/null/false, because initializers can reference unplaced symbols). Startup word-copies the load image (placed after constants, `image.load_base`) and clears to `writable_end` before vectors/main. Fixup kind "image" reads DataImage attributes after assign. Needed because ROM writes are silently ignored — SDK singletons (pcg cursor, irqc callbacks) would otherwise break.
- Testing: Python subprocess tests on this host fail with `OSError: [WinError 6]` (invalid std handle) when launched from the agent's PowerShell/Bash, even with `< NUL`, intermittently. Reliable: `Start-Process python -RedirectStandardInput file -RedirectStandardOutput ... -Wait`. Those failures are environmental, not test regressions.
- Boot verification recipe (isolated dirs, debug testbench, ~70 s each): RAM boot `ramprog.bin 42 --frames 140`; disk boot with a 4-byte zero program, dd0 unsigned + dd1 signed boot sector computing R1 = drive + (SP>>16) + 26 = 42, `--frames 170` (FDD ~108 ms/sector); no device `--skip-render-checks` shows halted pc inside 0xFF00xxxx. testbench `--expect-mem-u8` is asserted every frame, so use R1+HALT for post-boot results.

## Foundation OS implementation proposal (2026-10-22)
- doc/foundation_os_plan.md is a **proposal only** (owner: stop at the plan this time). Nothing under foundation/ exists yet; do not claim kernel/FDFS/libfnd implementation or tests.
- Owner decisions: name "Foundation"; microkernel + preemptive; DOS-like but original API, no POSIX compatibility; Unix-like single root with /bin /lib /dev (system devices) /dd/<n>/ (disk n's FS root, like /mnt, e.g. /dd/0/example.txt) /os (proc/kernel/services info); new shell built from DOS knowledge; implementation language C or C++.
- Key constraints captured: DMA bypasses Extension P so IDC/DMAC/BMC/VPU stay kernel-only (kernel blk_io validates buffers); IRQ0 is display-only, scheduler tick uses PeC timer (IRQ4, shared with RTC); llvm-src32 lacks inline asm/varargs/atomics so a context-switching trap stub (`kernel_trap` returning the frame to restore) must be added to the toolchain; existing syscall_dispatch stub cannot switch threads; WAX v2/v3 are fixed-address with no MMU, so concurrent apps need fixed slots or a future relocatable WAX; v3 entry zeroes R1..R27, so startup data goes through a syscall.
- Open items listed in §9 of the plan (address model, filename case, loader placement, tick). RAM size discovery method is unverified.

## Foundation kernel ABI v1 specification (2026-10-22)
- doc/spec_Foundation.md fixes trap/IPC/syscall/handle/boot hand-off contracts; status "specified, not implemented". Kernel and libfnd do not exist yet (the `kernel_trap` stub, solc `!abi_compat` and fnd.sol were added later; see the section below). Do not report runtime validation for any of them.
- Key contracts: SYSCALL args R1..R4, result i32 in R1, **every other register preserved** (sol !asm wrappers and cpt32_syscall rely on it); context byte must be 0. Errors -1..-63 kernel, -64..-255 API. Timeouts: 0 = poll (E_WOULD_BLOCK), 0xFFFFFFFF = forever. Handles 1..63, 0 = none. IPC is copy-only, max 4096 bytes; oversized request truncated (fnd_recv.total), oversized reply rejected with E_LIMIT and token kept.
- Fixed user slots (0x00400000..) because WAX is fixed-address without MMU; one process per slot; heap grows from the top block toward stack. Region order: 0 stack, merged blocks, heap, MMIO grants. MMIO grants only for non-bus-masters (BMC masters are CPU, VPU, DMAC; also never PeC timer/RTC/IDC or expansion cards).
- Bound IRQs: kernel masks ENABLE and keeps PENDING on notify; irq_ack W1Cs PENDING and unmasks; services must recheck device status after irq_ack (edge-latch race).
- Toolchain SYSCALL 0 exit convention collides with Foundation 0x00 sys_info; Foundation C startup must call proc_exit (0x11).
- sol: `!abi_compat "Foundation" [major]` (owner request) selects U-mode PROTECTED WAX codegen; sol language/call convention unchanged. Success 0 aligns with sol truth 0 so `status if` runs on success; handle/byte-count results need `0 lt`. Implemented 2026-10-22, see the next section.

## sol `!abi_compat` implementation (2026-10-22)
- Parsing is in the first pass of `compile_program` (sol_vm.py), before addresses are allocated, so the Foundation stack defaults (top 0x00C00000, 256 KiB) apply before `!stack_top`/`!force_stack_size` override them. Per-file placement uses `files_with_content` and skips the operands of the earlier directives (`_DIRECTIVE_OPERANDS`); `!var` takes one optional number, so `!var x` followed by a number on the next line consumes it as the initial value (pre-existing behavior, also bites tests).
- The optional major must be on the same line *and file* as the directive. After include expansion the next token can be line 1 of an included file; comparing only the line number once consumed `!abi_compat` of the library as a major.
- Mismatch rule: all declarations equal; a non-bare declaration in any include also requires the main file (`source_path`) to declare it. Undeclared libraries fit both.
- Foundation rodata/vars follow the code, but compile_program allocates them during parsing. `_compile_foundation` compiles, assembles only the code (text up to `__solc_code_end:`) to find its end, then recompiles with new bases. Code size does not depend on those addresses because addresses always load via LDIH/LDIL; keep that true or the loop may not converge. `Program.explicit_layout` records source/CLI layout settings that must not be replaced.
- asm.py `.PERM`/`.PROTECTED` produce `Assembler.block_perms`/`protection`; `blocks` stays 2-tuples for llvm-src32's driver. Vars are a zeroed RW block; their initial values are still stored by the startup code.
- The VS Code server copies were stale versions (no IRQ, no org); they are now byte-identical copies of tools/solc. Keep copying rather than hand-merging.
- No kernel exists: `tools/solc/test_sol_foundation.py` runs the PROTECTED image under a tiny S-mode harness (regions from the WAX, SYSCALL stub returning 0/len/-1, proc_exit HALTs with R1) on src32_testbench scalar/pipeline. That proves U-mode codegen and fnd.sol register use, not Foundation kernel behavior. Building the testbench on Linux needs `libasound2-dev`.
- Pre-existing bug fixed: `local r` directly followed by `!asm` crashed (`NUMBER_RE` on a RawAssemblyToken).

## FS2 filesystem specification (2026-10-22)
- `doc/spec_FS2.md` replaces the proposal's FDFS name with FS2 (File System 2). Specification only: host tools, guest driver and VFS do not exist yet. DSFS images remain a different format; changing magic/version is not conversion.
- DSFS-derived 512-byte blocks, big-endian and MSB-first bitmap. Bitmap tracks occupancy; each data block has a u32 next pointer and 508-byte payload, permitting fragmented allocation without a separate FAT. Inodes are 128 bytes; directory entries are 1040-byte logical stream records and may cross payload/sector boundaries.
- Names preserve UTF-8 spelling, max 256 Unicode scalar values / 1024 bytes. Compare using pinned Unicode 16.0.0 full case folding (C+F, no locale/T mapping); no Unicode normalization. Do not use host-version-dependent lower/casefold silently. Time is signed i64 UNIX seconds plus u16 milliseconds 0..999, not i64 epoch milliseconds; RTC seconds and scheduler ticks cannot simply be combined.
- UID-only owner/other RWX uses six compact bits, not POSIX mode numbers. VFS centralizes permissions and authenticates UID; current Foundation ABI has no UID credential path. Single-user UID 0 sessions do not establish user isolation. DSFS is read-only with synthetic ownership/modes.
- RW mount persists DIRTY before changes and clears it only after clean unmount. Multi-sector names/rename are not atomic; do not copy DSFS's leak-only crash claim. Dirty volumes require offline fsck, ambiguous corruption is reported rather than guessed, and content restoration is not guaranteed. Section 12 lists future acceptance tests, not completed runtime validation.
- VFS resolves components including `..` with mount and traversal checks; libfnd must not lexically erase them. Boot disk aliases share one volume/cache; handles/cwd retain mount identity and media generation. Open targets/descendants/cwd block unlink/rename/unmount as specified. IPC framing for up to two 4096-byte paths and authenticated UID sessions remains future file-API work.
- Foundation plan section 6.3 details DSFS compatibility under `/dd/<n>/`: validate original images without conversion, expose a flat uppercase 8.3 listing with case-insensitive lookup through common APIs, reject all updates as READ_ONLY even for UID 0, synthesize UID/mode and zero-extended seconds with ms=0, and copy files into FS2 using destination creation rules. These are planned contracts, not implemented behavior; future checks must verify rejected updates leave the image byte-for-byte unchanged.
- Host shared-directory mounts (owner decision): IDC internally converts the supplied host tree into FS2-compatible sectors, exposed through a normal IDC slot and Foundation's existing FS2 driver under `/dd/<n>/`. Plan section 6.4, IDC section 11 and FS2 section 11 record this as unimplemented. YAML keys, capacity, snapshots/refresh, attribute mapping and writable host synchronization remain to be specified; raw sector updates must not trigger host file operations from incomplete FS2 metadata. Current `image` settings still require raw image files.

## kernel_trap context-switch stub (Foundation phase 0, 2026-10-22)
- tools/llvm-src32/backend.py `emit_ctx_stub` implements doc/spec_Foundation.md §3. Defining `cpt32_ctx_frame *kernel_trap(cpt32_ctx_frame *)` requires `void kernel_panic(uint32_t, uint32_t)`, is exclusive with irqc_dispatch/syscall_dispatch/--protected, and makes startup mask IRQC, point all 16 IRQ slots (`JR R30`, S-bank R30 = stub) and the 0x04/0x08/0x0C vector words at the stub.
- Stub globals `cpt32_ctx_current` / `cpt32_trap_depth` are emitted as code-area .DWORD labels and injected into the symbol map, so C uses the extern declarations in cpu.h; a C definition of either is an error. Depth starts at **1**: any trap before `cpt32_ctx_resume` (new noreturn intrinsic) goes to kernel_panic instead of writing through a NULL frame pointer.
- Entry uses two scratch words below the current S-bank R28 (kernel stack top after a user trap, idle stack after an S trap). kernel_trap always runs from `--stack-top`, so main's stack is dead after the first resume. Restoring an S frame loads R28/R29 only; R30 stays the stub address (spec updated: S frame word 30 is informational). Restoring a U frame resets the S-bank R28 to the stack top. STATUS restored as `& 0x1A` (PIE|PU|PSM), IE only via IRET.
- examples/kernel-trap-c: cooperative switch with kernel-side mirror of live values, S-mode idle (IE=1 HALT) left by PeC timer ch0 IRQ4, timer preemption of non-yielding tasks; ends by resuming an S frame (status 0) at a rodata `HALT` word so R1 survives (a C function would clobber R1 in its prologue). A mutation (wrong mirror seed) made testbench report R1=266, so the checks are live. Tests: test_kernel_trap_context_switch_example (O0/O1/O2 x 4 CPUs), test_kernel_trap_panics_before_first_resume, test_kernel_trap_diagnostics; full test_toolchain.py 34 passed, sdk smoke run.py passed, CMake build + no-op rebuild passed.
- The diagnostic test for a wrong kernel_trap signature must not include <cpt32/cpu.h> (Clang rejects the conflicting prototype first).

## Foundation phase 1 minimal kernel (2026-10-22)
- foundation/ implements plan §8 phase 1 on the kernel_trap stub: kernel/sched.c (4 priorities, FIFO RR, PeC timer ch0 1000 Hz on IRQ4, 10-tick slice, S-mode idle HALT loop, single wake() path), kernel/proc.c (7 slots of §5.2, §5.3 region switch order, §5.4 pointer check, §9.4-style exit: threads DEAD, owned RAM zeroed), kernel/trap.c (IRQ drain, syscalls yield/sleep/thread_exit/proc_exit/time_get/debug_write, U fault -> exit code = CAUSE as i32 with fault_epc, idle exception -> kernel_panic). Build with --origin 0x10000 --stack-top 0x100000 (§5.1).
- No WAX loader yet: `fnd_proc_create_builtin` runs user code linked into the kernel image ([user_code_begin, user_code_end) R/X, top `ram_size` bytes of the slot R/W). Do not grant/zero the whole slot: zeroing 4 MiB slots at boot cost more than the testbench frame budget. User code must avoid globals, literals and calls; `<foundation/sys.h>` wrappers are macros so they stay inline at -O0.
- cpt32_mtsr needs a constant SR. A switch over region numbers works at -O0 but -O1/-O2 merge the cases into one MTSR with a computed SR (compile error). proc.c uses 24 noinline single-MTSR functions in a table.
- Real race: a tick between the timer ACK and the IRQC W1C leaves the timer source high with PENDING cleared, so no edge ever arrives again (ticks froze at -O2 cached/pipeline). handle_irqs re-checks the sticky TIMER_PENDING (0x80040188) after draining IRQC. Keep that re-check whenever the ACK -> W1C window contains work.
- src32_testbench has no UART (0x80040000 unmapped): debug_write with len > 0 or kernel_panic output bus-errors in the kernel. The test only uses len 0 and a bad pointer.
- tests/phase1 (R1=42): counters A/B advance, fault process dies with 0x80050002 and its RAM is zeroed, A's word never gets the faulting store, SERVICE probe sleeps 7..8 ticks (priority preemption), error codes, proc_exit code, then 10-tick slices alone alternate A/B. The hook snapshots the probe mailbox each tick because exit zeroes it. Mutations (slice expiry disabled; region switch disabled) fail the test. test_toolchain.py test_foundation_phase1_kernel runs O0/O1/O2 x 4 CPUs with --frames 30; JIT passed manually. With Clang 18 in the cloud container, kernel-trap and syscall-user examples fail at -O1/-O2 ("icmp constexprs") on the base commit too.

## Foundation WAX loader: PROTECTED v2 and WAX/WAL v3 (2026-10-22)
- foundation/kernel/wax.c is the loader core: plain C11 on byte arrays, no kernel state, so tests/wax/test_wax_loader.py builds it with the host CC as a shared library (host.c shim, fake RAM with a bias). Keep it free of intrinsics and MMIO. wax_plan writes nothing outside the launch record; wax_load cannot fail. proc.c `fnd_proc_spawn_wax` claims the record, plans, allocates the thread, and only then loads, which is what makes spawn atomic.
- Foundation policy that v3 left to the OS lives in spec_Foundation.md §9.5: provider = highest abi_minor among valid WALs with the same id/major (first listed on a tie), so combined min_minor is just a check; WAL placement = dependency post-order, first fit from the slot base at the WAL's max block alignment; limits 8 images / 32 blocks / 8 deps; logical category -> FND_E_* table. Where the library catalog comes from (/lib, FNDBOOT) and the proc_spawn syscall (needs PROCESS handles, must stage the image first) are still open.
- No compiler emits v3. tools/wax/wax3.py lays out images from blocks/tables and writes exactly what it is given (tests patch fields via Image.layout). tests/wax/build.py assembles *.asm (PIC code addressed through R30 with .CONST offsets; avoid far branches, the assembler's trampolines use absolute LDIH/LDIL) and writes images.h.
- images.h holds big-endian uint32_t words plus img_*_size (4-byte aligned). This started as a workaround for the 0x5C bug below.
- Fixed llvm-src32 bug: LLVM prints a backslash byte inside c"..." (and quoted names) as `\\`, every other escape as `\XX`; data.py `_quoted_bytes` only knew `\XX`, so any constant byte array or string literal with 0x5C (e.g. "C:\\dir") failed with "invalid LLVM byte escape". Regression: test_backslash_bytes_in_constants.
- src32_testbench runs all frames and then treats a non-running CPU as halted, so the idle thread's HALT (all processes exited/blocked) at the end of the budget reads as R1=0 or "did not halt". tests/wax keeps a never-yielding `spin` process. Spawning is slow with interrupts off (11 spawns ~16M cycles at -O0 Scalar), hence --frames 90.
- Mutations that the testbench test catches: descriptor origin +4, UGP = 0, regions granted RWX. Zeroing BSS is only caught by the host test (testbench RAM starts zero and exits zero it).

## GitHub Actions CI (2026-10-22)
- .github/workflows/ci.yml: `rust` builds src32_testbench (debug, uploaded as artifact) and runs the `tests/*.rs` integration tests only; lib unit tests cannot build without the external `s3w2_core` golden .inc files. Guest images dos.bin / tinybasic.bin / dsfs_guest.bin are gitignored, so CI builds them from dos.sol, tinybasic.sol and tools/dsfs/dsfs_smoke.sol (solc.py compile -> asm.py) into the repo root before `cargo test`.
- Known failures run in the `known-failures` job with step-level continue-on-error (PR check stays green, results in the step log): strek_micro, strek_smoke, tb_load_live. strek_verify needs a sibling Tiny-Basic checkout and is not run. When one of these is fixed, move it back into the blocking job.
- `toolchain` installs Clang of the same LLVM major as llvmlite (`llvmlite.binding.llvm_version_info`) via apt.llvm.org and sets CLANG; a mismatched Clang (e.g. Ubuntu's Clang 18 vs llvmlite's LLVM 20+) fails -O1/-O2 with "icmp constexprs". It also needs cmake + ninja (present on ubuntu-24.04 runners).
- Release (CD): .github/workflows/release.yml. Pushing a `v*` tag builds cpt32 / cpt32-tool / src32_testbench in release mode for linux-x86_64, windows-x86_64 and macos-arm64 (archive also carries README, licenses, config.example.yaml, assets/bios.rom) plus both VS Code .vsix files, and publishes them with softprops/action-gh-release (tags containing `-` become prereleases). PRs touching release.yml and workflow_dispatch run the builds without publishing. CI's `vscode-extensions` job also packages the .vsix on every PR.
- `toolchain` and `solc` run with `if: !cancelled()` after `rust`, so a failing integration test does not hide them; they only need the uploaded testbench artifact.
- (Resolved, see "dos_smoke deterministic input" below) dos_smoke hex tests can hang on CI runners (seen on dos_pipeline_hex_returns_to_shell_at_default_cycles_per_frame: keys for "EXIT" never reach the guest, CPU halted with IRQC pending=0x0001 enable=0x0008). connect_pec_input applies host key events on a separate `pec-input` thread, so the test races with the guest; under CPU contention an input edge can be lost. Not reproducible locally in 3 full runs. Same symptom as the known 800-cycle failure.

## dos_smoke deterministic input (KAN-25, 2026-10-22)
- 原因はテスト側。`connect_pec_input` は key event を `pec-input` スレッドで FIFO に積むが、ヘッドレステストは `frames()` でシミュレート時間を進める。CI のように負荷があるとスレッドが遅れ、`wait_text` が数千フレームを数 ms で使い切ってタイムアウトした（トレースでは最初の key だけ push され、残りは panic 後に push されていた）。ゲストの IRQ 経路は無実で、失敗時は FIFO 空・IE=true・HALT だった。
- 修正: `connect_pec_input_sync` が返す `PeCInputHandle::send` は FIFO へ同期的に apply する。フレーム単位で進めるテストはこちらを使い、次のフレームの前に必ず key が FIFO にある状態にする。スレッド版 `connect_pec_input` は GUI/testbench 用に残す。
- 連動して顕在化した 2 件のテスト不備も直した: (1) `dos_shell_manages_files_on_two_drives` は "BYE" 表示直後に PC を採ったが、その時点で HALT 未到達だった（以前は非決定的に通っていた）。5 フレーム進めて HALT を確認してから採る。(2) HEX 起動待ちが 2000 フレーム固定で、800 cycles/frame（scalar）では足りなかった。サイクル予算（`render_frames`）に比例させ、`dos_hex_returns_to_shell_at_800_cycles_per_frame` を通常ジョブへ戻した。
- 未実証の理論上の穴: `dos.sol` の読み取りループは「FIFO 空を確認 → HALT」の間に IRQ3 が入ると起床を失う（エッジは消費済みで FIFO は非空のまま）。今回の失敗の原因ではない（試験的に VSync 起床を足しても失敗は直らず、別の不具合を生んだ）。修正するなら IRQC は「複数 source が pending の間は新しい立ち上がりがない」（spec_IRQC §5）ので、ハンドラは使う source をまとめて W1C すること。
- 検証: `cargo test --test dos_smoke`（11 件）を並列 8 スレッド 55 回＋6 プロセス同時の負荷下で繰り返し、全件成功。


## Foundation minimal shell on PCG (2026-10-22)
- `python foundation/run.py` builds the shell/kernel WAX and release emulator, then runs with a dedicated `target/foundation-boot/run/config.yaml` (16 MiB RAM, absolute UNSCII font path, no BIOS/disks). Root config's `bios.image` overrides a supplied WAX entry, so launching Foundation from the usual cwd may boot BIOS instead. `--build-only` prepares the artifacts/config without opening a window; `--clang` forwards the compiler selection. Build subprocesses use workspace-local TMP/TEMP/TMPDIR to avoid Windows sandbox temporary-directory access errors. Windows release shell tests passed all 8 cases after rebuilding src32_testbench.
- foundation/shell/shell.c implements the parts of doc/spec_Foundation_shell.md that need no fs, IPC or proc_spawn: lexing/syntax §3–§4 (whole line checked before anything runs), variables §5.1, line editor §9.2, messages §11.1, builtins CLS/ECHO/SET/VER/MEM/PS/HELP/EXIT and bare CD. fs-dependent features report FND_E_UNSUPPORTED; command lookup reports `command not found`. foundation/README.md "シェル" lists what is provisional.
- (Superseded by stage 2 below) The shell no longer drives the console itself: services/console owns the PCG/PeC MMIO grants, and the shell has caps=0 and no grants.
- Line editing uses only ST_WRITE controls: console BS at column 0 goes to the end of the previous row, so redraws are "print tail, blanks, BS back". Keep that BS rule if the console is rewritten.
- `--protected` used to emit one WAX block per `.ORG` gap (alignment padding between globals), so any C program with several globals hit "protected WAX allows at most 7 blocks". driver.py protected_blocks now fills gaps between same-permission blocks with zeros (regression in test_protected_wax_output). Zero-initialized globals are stored in the image (shell ~220 KiB).
- Spawning the 220 KiB shell with interrupts off loses ticks, so tests stop at tick 3000 but need ~200+ frames; tests/shell uses 260 frames.
- src32_testbench gained `--type FRAME=TEXT` (HID usages, US layout, escapes \n \b \L \R \U \D \H \E \X \C \; sleeps 50 ms after sending because the PeC input thread applies events asynchronously) and `--dump-pcg` (visible 30 rows honoring SCREEN_MODE and SCROLL_Y). foundation/tests/shell compares whole screens: about 40 s on the release testbench, 4.5 min on debug (CI).
- The PCG font comes only from config.yaml `pcg.font`; without it the GUI shows no glyphs (VRAM text is still right).
- Owner decision (2026-10-22): built-in names are UNIX-style (cd, pwd, ls [-l], cat, cp, rm, mv, mkdir, rmdir, clear, echo, set, uname, free, ps, help, exit), still case-insensitive, no DOS aliases; error messages use the lowercase names. Interactive start prints `Foundation OS (kernel ABI 1.0)` and `Foundation shell 1.0` (same as uname). There is no separate OS version number: the kernel ABI version from sys_info stands in for it. Spec §2.1, §10, §13 record this.

## Foundation disk boot: FS2 host tool, stage1, FNDBOOT (2026-10-22)
- tools/fs2/fs2.py is the FS2 host tool (spec §12 step 1): format/info/check/ls/get/cat/put/mkdir/rm/mv/chmod/chown/install-boot. It edits a bytearray and writes the file back only after the op succeeds and §10 mount validation passes. Name comparison uses tools/fs2/CaseFolding-16.0.0-CF.txt (Unicode 16.0.0 C+F, Unicode License V3 text in its header): Python 3.13's str.casefold is Unicode 15.1 and misses e.g. U+A7CB, so never use str.casefold for FS2. The table came from the npm package @unicode/unicode-16.0.0 (unicode.org itself was blocked by the container proxy).
- Directories never shrink: rm zeroes the 1040-byte slot and create reuses free slots. File writes are in place; growth links fresh zeroed blocks at the end (first fit, so files fragment).
- foundation/boot/stage1.a must fit 510 bytes (it is 508). To fit, it does not re-enable the DMAC (BIOS §3: the BIOS just read this block through DMAC ch0, so it is still enabled) and checks only the IDC ERROR bit (IDC §5 reports DMA failures there too). Superblock checks: magic, version word = 0x00010000 (version 1 + flags 0, so DIRTY=0), total<=IDENTIFY capacity, boot_start=2, bitmap_start=2+boot_blocks without wrap, bitmap_start<=data_start<=total, 0<boot_size<=2 MiB and fits boot_blocks. It does not re-derive the bitmap/inode formulas. Failure: HALT with R1 = 1 (IDENTIFY/superblock read), 2 (superblock), 3 (FNDBOOT read/header); nothing is printed.
- llvm-src32: the freestanding entry may now take up to two uint32_t args = R1/R2 at entry (crt0 stores them at SP before anything else). The C ABI passes arguments on the stack, not in registers. foundation/boot/main.c is `int main(uint32_t drive, uint32_t fndboot)`.
- FNDBOOT (foundation/boot/fndboot.py, kernel/fndboot.c): header | module table | grant arrays (12-byte fnd_grant, §9.2) | kernel | module images, all 4-byte aligned. The direct-load WAX build (-D FOUNDATION_BOOT_EMBEDDED) links a kernel-less FNDBOOT (fndboot_image.h), so both run modes go through the same module-table code. kernel_entry = 0x10000 (crt0 is first in the raw .bin).
- (Stage 2) init restarts the shell, so the kernel no longer respawns a module; caps in the module table are passed to fnd_proc_spawn_module. /bin and /lib are on the disk, but the provisional fs serves its own embedded copies until the fs service reads the disk.
- Disk: 8 MiB, inodes 256, boot area 4096 blocks (= stage1's 2 MiB max). `python foundation/run.py --disk` boots it through assets/bios.rom from a read-only HDD in dd0 with a 4-byte zero program. FDD speed takes ~6 s to load the ~400 KiB FNDBOOT.
- foundation/tests/disk: the BIOS POST hold is 120 frames and the kernel starts ~frame 125, so --stop-ticks 4000 and typing at frame 250 (with 3000 the machine stopped before frame 330 and the typed keys were lost). The kernel's main returns FND_E_FORMAT (-12) for a bad module table, so the testbench sees R1 = 0xFFFFFFF4.
- (Superseded by stage 2 below) `--fnd-wal` and boot_libs.h are gone: the FNDBOOT modules (console, fs, init) are static WAX v2 and get no WAL list; the shell and /bin are always WAX v3 importing /lib/fnd.1.wal.

## Foundation stage 2: IPC, services, /bin (2026-10-22, PR #13)
- Kernel: kernel/ipc.c (handles, IPC, svc_register/lookup, wait, proc_kill), proc_spawn with `libs` (offset 44; image and WALs copied to staging 0x100000..0x400000 first), proc_startup, mem_grow. plan_heap rounds the heap start up to 8 bytes; an unaligned heap broke word zeroing and the libs list (FND_E_BAD_ARG).
- Boot: FNDBOOT modules console (slot 1, SVC, MMIO grants), fs (slot 3, SVC, services/bootfs: provisional read-only fs embedding /bin and /lib), init (slot 0; fnd__exec("/bin/shell.wax") with the console as roles, restarts it). init prints `init: /bin/shell.wax: <error>` on the console if the start fails.
- Shell = dispatcher (cd/set/exit/help); clear/echo/free/ps/pwd/top/uname are /bin/*.wax (foundation/bin, slot 5), WAX v3 importing WAL fnd 1.0. fnd_proc_exec reads /lib/<id>.<major>.wal by following dependency tables.
- fnd_con_pollkey = CON_READ_KEY with a0 = FND_CON_NOWAIT; the console answers FND_END when no key is queued or another request waits.
- llvm-src32 code is ~25 instructions per byte in a byte loop: kernel copies (wax_load, kmem_copy/zero) go by words, 8 per step, requiring only address alignment. Byte copies made boot ~50 frames and keys overflowed the 256-event PeC FIFO before the console ran.

## FS2 host 9P export (2026-10-22)
- `python3 tools/fs2/fs2.py serve IMAGE` runs the stdlib TCP 9P2000 adapter in `tools/fs2/ninep.py`, default localhost:5640. Linux mount: `sudo mount -t 9p -o trans=tcp,port=5640,version=9p2000,access=any,cache=none 127.0.0.1 /mnt/fs2`. This exports an offline raw image, not the running guest's filesystem. Never concurrently edit it from the emulator or another FS2 command.
- Each mutating request operates on a copy, checks the resulting image, then uses fs2._save; discard the copy on failure because Volume operations can partially modify it before raising. All connections share a locked Export. The advisory `.9p-lock` sidecar survives image replacement and prevents cooperating servers using the same canonical path; other FS2 commands/emulators do not honor it.
- Fids identify inode + incarnation, not paths, so rename preserves handles and deleted/reused inodes cannot alias old fids. Remove clunks even on failure. This adapter rejects stale handles rather than supporting POSIX open-unlink semantics. Directory read offsets count packed 9P stat bytes and must fall between records; never split records.
- Only 9P2000: cross-directory rename, links, atime writes, GID, special modes and ORCLOSE/OEXCL/OEXEC are unsupported. POSIX group displays other, and group chmod bits are ignored. No authentication: clients act as UID 0, bind localhost by default. README and spec_FS2 §12.1 cover contracts.
- `python3 -m unittest discover -s tools/fs2 -v` includes real TCP wire tests with an independent client (localhost sockets require sandbox network access). Kernel mount has not been exercised in this environment (CapEff=0). Tests cover Unicode case folding, sparse writes, directories, wstat, rollback on NO_SPACE, malformed messages, read-only mode, reconnects/multiple clients, stale fids and locking.


## CI performance and job dependencies (2026-10-22)
- Investigated CI run 39724282519 (PR #15, head e521d141): the LLVM/Foundation job spent 285.910 s on 43 toolchain tests, 306.620 s on 17 shell tests and 436.599 s on 7 disk tests, sequentially (~17.2 min). Clang setup was ~22 s. These are baseline measurements, not timings of the optimized workflow.
- CI now sets CARGO_PROFILE_DEV_OPT_LEVEL=2 and CARGO_PROFILE_TEST_OPT_LEVEL=2. The interpreter executes millions of guest instructions, so unoptimized host Rust is expensive. Keep dev/test debug assertions and overflow checks enabled; changing to release would disable them by default. Artifact paths remain target/debug/src32_testbench, and consumers set SRC32_TESTBENCH explicitly.
- `rust` only builds/uploads the testbench; `rust-tests`, `known-failures`, `toolchain` and `solc` depend on that producer. Python suites no longer wait for Rust integration tests. The toolchain matrix runs toolchain/shell/disk independently with fail-fast=false; retain every suite and its existing commands/frame budgets.
- All three Rust jobs use rust-cache shared-key=ci so consumers can reuse the producer's dependency cache. Profile environment changes invalidate the previous cache; expect a cold first build and measure cached runs separately. Local checks here: actionlint 1.7.7 on both workflows, YAML command/dependency/matrix invariants and git diff --check. Rust/Clang are unavailable on this executor, so runtime speedup and full test results must be verified by GitHub Actions.

## Foundation launcher arguments (2026-10-22)
- `foundation/run.py --arg ...` forwards all remaining argv tokens after the image path to the emulator using `argparse.REMAINDER`; put launcher options such as `--disk`, `--clang` and `--build-only` before it. Tokens are preserved without shell splitting or shell execution, including values containing spaces. `--build-only` still skips emulator launch even when extra arguments are supplied.
- Repository workflow: in remote environments, commit changes on a working branch, push that branch and open a PR targeting `main`; AGENTS.md explicitly authorizes the branch creation and push required for this workflow.

## Foundation top and CPU accounting (2026-10-22)
- Owner reports measured OS CPU overhead of about 4%; workload, CPU engine and measurement method were not specified. This is a reported observation, not a universal target or a result reproduced here.
- `sys_info` sel 2 adds a 16-byte CPU header and 8-byte PID/cycle records without changing sel 0/1 layouts. PeC timer ch1 is reserved as a free-running 48 MHz accounting clock. All counters wrap at 2^32: subtract snapshots unsigned, less than ~89 s apart. Charge user-context time at trap entry before process exit/reuse, and kernel time after sched_next (including protection changes). Reset process cycles on new PID, retain exited records.
- `top [-n count]` samples each second, sorts by CPU, and quits on q/Ctrl+C; finite counts print without clearing. `kernel~` measures C trap boundaries; assembly save/restore is charged to the surrounding user/idle context, so this is not a strict measurement of total OS overhead. Added nonblocking `fnd_con_pollkey` (FND_END when empty) shares the input decoder with blocking getkey; keep its WAL export synchronized.
- A local `-O1` pipeline testbench run of `top -n 2` with the shell otherwise idle showed kernel~ 4.2%, user 0.1–0.3%, idle 95.4–95.6%. This includes the new instrumentation and is only a sample for that build/workload.

## Foundation Jira backlog audit (2026-10-22)
- Jira project KAN (CPT32 development plan), Foundation epic KAN-4: added KAN-26 through KAN-35, all verified as `To Do (機能)` with parent KAN-4. Topics are console/shell Ctrl+C integration, offline FS2 repair, coherent RTC snapshots, trusted UID credentials, DEV_OPEN authorization, IDC host-directory media, relocatable WAX design, sol libfnd wrappers, expansion-card driver design, and implementation-plan synchronization. Each ticket records source documents, acceptance criteria and prerequisites.
- Before adding work, read all existing project issues including completed ones and check current upstream documents. This checkout was at c58cebf while upstream already had phase2 IPC, external commands, WAL integration and BMC capacity reporting. KAN-11/KAN-12/KAN-16 are completed; do not recreate them from stale local README or plan text. KAN-15 already covers guest FS2/VFS/disk/blk_io, KAN-17 file commands, KAN-18 threads/IRQ, and KAN-19 scripts/wildcards/redirection. KAN-26 covers console/shell integration after KAN-18, rather than duplicating its kernel work.
- `tools/fs2 check --allow-dirty` is inspection, not repair. FS2 repair must save the original image, reject ambiguous corruption and only clear DIRTY after complete validation. IDC host-directory conversion belongs inside IDC and is distinct from completed KAN-22's offline-image 9P export. UID authentication and DEV_OPEN caller authorization require ABI design before implementation; the current API explicitly documents their limitations.
- `doc/foundation_os_plan.md` remains non-normative and contains outdated progress and DOS command names even on upstream main. Prefer current Foundation/API/shell/FS2 contracts and actual source; KAN-35 tracks synchronizing the plan. This audit verified Jira records and document content, not guest runtime behavior.

## sol guests using C libfnd, KAN-33 (2026-10-22)
- `sol/foundation/fnd.sol` is the existing syscall interface; `libfnd.sol` adds high-level wrappers using the actual C implementation. Build with `tools/solc/build_foundation.py`, not solc alone: it supplies the private dispatcher address `FND_LIB_ENTRY`, copies arguments from sol's R26 frame into the C stack ABI, aligns SP to 8 bytes, preserves R26/R27/R30, and returns R1 as a sol value. No compiler/VM/LSP semantics or bundled copies changed. `fnd_proc_kill` already exists in fnd.sol; do not define it twice.
- Slot 5/6 only: sol image occupies the first MiB, the C bridge starts at origin + 1 MiB, stack top is the slot end. Static mode emits PROTECTED v2; `--import-wal` emits v3 using the existing signature-checked imports, descriptor slots and fnd WAL. The sample uses v3 to fit bootfs slot 3: embedding two static libfnd guests overflowed its 512 KiB. Check the complete WAX + WAL region count, not just sol's blocks.
- `fnd_exec_t` itself must point to a real initialized 28-byte structure. Only its cwd/stream fields accept 0 for inheritance; passing ex=0 faults inside C. `fnd_proc_wait` closes the PROCESS handle on success. Buffers/output words must be writable and aligned; distinguish status=0, END=1, returned handles/pointers, and sol true=0. stat seconds/size are high/low words; milliseconds are separate. Contracts are in sol/foundation/README.md.
- `examples/foundation-sol/build.py OUT` generates sol parent (slot 5), sol child (slot 6), a Foundation WAX and disk. `foundation/boot/build.py --root` now adds files to both the disk and provisional bootfs; formerly only the disk saw them. Escape host filenames as C UTF-8 byte strings when generating the table. Keep intermediate .a/.ll outside the installed --root tree.
- KAN-33 is still in progress: bootfs is read-only and DIR_OPEN is unsupported; sample asserts those real errors rather than claiming successful file writes/listing. Stream writes work on console. Directory wrappers expose raw DIR_NEXT records, not a cached one-entry fnd_dir_next. Successful writable-file/directory coverage awaits KAN-15.
- Validation: compiler/LSP/Foundation/libfnd focused pytest run passed 104 tests, including three complete real-kernel/WAL executions on each of Scalar/Pipeline. An additional static-link U-mode test passed (real C init/argc/BAD_HANDLE and nested sol frame preservation, syscall harness rather than full services). Rebuild testbench and explicitly set SRC32_TESTBENCH: the older debug executable lacked --dump-pcg. On this Python 3.15 Windows setup, pytest `-s` avoided invalid inherited stdin handles seen with capture; use workspace TEMP/TMP.
- Shell suite: 14/15 passed. test_exit_restarts_shell expects PID 5 shell BLOCK but sees RUN while child ps takes its snapshot. Reproduced with the original pre-change build.py and the same testbench, so this is independent of the new guests/root embedding; tracked as KAN-37. Do not report the full shell suite as passing or weaken its lifecycle assertions without investigating the schedule.

## Foundation shell performance and restart snapshot, KAN-36 / KAN-37 (2026-10-22)
- Bundled tools/llvm-src32/runtime/string.c was always compiled at -O0, even for optimized guests. It now uses -O2, aligned 32-bit memcpy/memset with byte prologues/tails, and a may_alias word type. Preserve byte fallback for different alignments, bounds, return pointers and memmove overlap semantics. Optional external Newlib compilation is separate. Boundary/overlap/sentinel checks pass at O0/O1/O2 on Scalar and Pipeline.
- Shell functional tests use Cached; restart/ps and a CPU-accounting snapshot still use Pipeline. FOUNDATION_TEST_CPU overrides every case. Keep independent disk copies, 600 frames, STOP_TICKS=6000, whole-screen and persisted-file assertions. All 16 cases passed both Cached and Pipeline with rebuilt guest images; seven BIOS/disk tests passed with the same generated disk.
- KAN-37: fnd__exec frees WAX/WAL/dependency buffers after spawn, before run_program calls wait. A slice expiry there lets ps run while the parent is READY, which sched_proc_runnable reports as RUN. RUN/BLOCK are both valid. KAN-15 already allowed both; preserve the assertions for the reaped old PID, new shell/init/ps parent relations and prompt return. No scheduler fix is needed.
- Matched Pipeline first prompt: 101 -> 99 frames (direct WAX + IDC FS2, no BIOS). An 8 KiB memset + memcpy repeated four times: 6,096,030 -> 976,474 guest cycles, measured with enabled PeC timer ch1. Testbench final cycles= reflects the frame budget, not operation completion, so do not use it to infer that cost. Full FS2 mount validation is unchanged and remains a boot bottleneck. See doc/kan36_performance.md for conditions and timings.
- Final mixed-CPU shell suite passed all 16 with the regular build path: 157.568 s versus the original all-Pipeline 196.973 s, each a single run (baseline partly overlapped other checks/builds). Treat this as a local observation, not an isolated statistical benchmark. Full Pipeline and full Cached runs also passed all 16 with prebuilt images; do not compare those timings directly with build-inclusive runs.

## Foundation temporary FS2 mount mitigation, KAN-38 (2026-10-22)
- foundation/boot/build.py sets FS2_FULL_MOUNT_CHECK=0 only for fs.wax. Set the constant back to 1 and rebuild to restore the full spec_FS2.md section 10 scan. Standalone fs2.c defaults to full checks; host tools and DSFS are unchanged.
- The temporary path keeps superblock/layout/DIRTY rejection, bitmap and inode field checks (including root type/parent), inode_used and free_blocks initialisation. It skips chain/directory/reference/ownership/leak scans and their scratch allocations. Clean media can still be corrupt: run tools/fs2/fs2.py check before using an image. Do not describe the temporary path as full section 10 validation or as fsck/repair.
- Host fs tests now exercise the temporary mode's persisted files, subsequent allocation, remount, skipped payload-padding validation, and retained DIRTY/layout/inode rejection without mount writes. On Windows, omit -fPIC for MSVC-target Clang, export name API wrappers from host.c, and release the ctypes DLL with _ctypes.FreeLibrary before TemporaryDirectory cleanup.
- Validation: host fs 27 tests, shell 16 tests (Cached with representative Pipeline cases), and BIOS/disk boot 7 tests passed. Guest images were rebuilt in target/kan38/quick with --stop-ticks 6000; shell/disk tests reused these artifacts and fresh per-case disk copies. Shell 116.945 s and disk 56.502 s are single runs with other checks overlapping, not isolated performance benchmarks.
- Matched first prompt: 99 -> 61 frames on Pipeline, direct WAX, read-only 8 MiB HDD, 16 MiB RAM/4 MiB VRAM, no BIOS/input, same release testbench and guest -O1. Binary-searched for banner + prompt. This is guest frame latency (about 38% less), not host wall time. Complete conditions are in doc/kan36_performance.md.

## Foundation /bin/ls, KAN-40 (2026-10-22)
- "ls: command not found" was simply a missing /bin/ls.wax (KAN-17). "bad format" for `/bin/ls` is by design: a command word with a slash gets no extension (shell spec 7.1 step 1), and only `.wax` runs. Use `/bin/ls.wax`.
- foundation/bin/ls.c lists via fnd_call(FS_DIR_OPEN / DIR_NEXT) directly (written before libfnd 1.1 added fnd_dir_*; it could move to them). Entries come unordered, so ls sorts. fnd_name_fold is still not in the fnd exports, so the sort key folds ASCII only.
- ls -l prints the wall-clock mtime, so tests must not compare its text exactly; test_shell checks only the shape and the totals line.
- Local build needs clang and llvmlite of the same LLVM major: llvmlite 0.45.1 (LLVM 20) with apt clang-20 and `--clang clang-20`.

## sol/libfnd after the real fs service, KAN-33 (2026-10-22)
- KAN-15 is complete: bootfs is gone. sol integration runs must attach a fresh FS2 HDD image in IDC slot 0; an empty config no longer boots /bin/shell.wax. examples/foundation-sol/build.py now writes run/config.yaml with that disk and the PCG font, and copies the clean foundation.img to run/disk.img. The sample uses real root FS2, not /tmp or a simulated service.
- Added C fnd_dir_open/next/rewind, dir_make/remove, file_delete and sync in lib/fnd/start.c and exposed them through sol/foundation/libfnd.sol. fnd_dir_next caches a complete 4096-byte reply per handle (1..63), validates record bounds/name lengths/count before writing any output, and returns one NUL-terminated entry. END/errors leave the entry untouched. Rewind resets both the remote cursor and cached EOF/data; fnd_close releases the cache, including partially consumed batches. First-call errors free the temporary cache. Do not bypass fnd_close with raw handle_close or mix raw dir_read and cached next without rewind.
- C fnd_dirent_t is 1100 bytes: stat at 0, u32 name_bytes at 64, u32 name_chars at 68, char name[1025] at 72. The wire record has rec_len at 0, stat at 4, u16 lengths at 68/70, and a non-NUL name at 72. Keep those two layouts distinct. FS_MKDIR puts mode in a1, unlike FS_CHMOD's a0.
- fnd WAL is now 1.1 for the added exports; the major-based path remains /lib/fnd.1.wal, and existing 1.0 export signatures are unchanged. Rebuild both WAL and guests; the new C bridge imports the added functions even when a particular sol caller does not reach every dispatch case.
- The sample writes/syncs/closes and rereads 5001 patterned bytes, checks a write through a READ-only stream fails with ACCESS and put=0, enumerates 64 ASCII names plus a 13-byte/7-scalar Japanese name across multiple batches, tests partial/EOF rewind, repeated EOF output preservation, empty directories, and four partial closes with an unchanged mem_grow(0) heap end. Exact heap-end equality catches even a single leaked allocation; a long repeated-open stress loop is unnecessary. Three complete runs per Scalar/Pipeline also exercise child argv/exit/wait and handle reuse. Make fixture bytes explicit: Path.write_text translates LF to CRLF on Windows.
- sol while is post-tested: put the continuation value just before end. A comparison at the start remains on the stack until end and runs the body once after the intended bound; early ret can then have an extra stack value. Do not write C-style pre-tested loops in samples.
- Stopping the testbench does not unmount fs: the disk remains DIRTY. Host verification uses check_image/Volume with allow_dirty=True without clearing the bit, then checks allocated metadata and fixture bytes and resolves deleted paths to NOT_FOUND. Each CPU gets its own fresh disk. The old bootfs budget was insufficient; even 24000 ticks stopped Scalar during the third run. Three full fs samples use stop-ticks 40000 and 4200 frames (not a speed measurement), with a 300-second host timeout.
- Final validation: existing compiler/LSP/Foundation tests 97 passed; libfnd tests passed across the final 7-pass run and the Scalar-only retry (1 passed, 7 deselected; 233.61 s including the build). Pipeline had already completed all three runs and host disk checks at 24000 ticks; Scalar completed them with 40000 ticks. Shell tests passed all 16. The sample's normal build regenerated foundation.wax, foundation.img and the run disk/config without a test stop. KAN-33's previously deferred writable-file/listing acceptance is now covered; remaining spec-only APIs such as seek/truncate are still explicitly outside the provided C API.

## KAN-41: API performance after PR #13 (2026-10-22)
- The kernel syscall and context-switch cost did not change across PR #13 (measured against c58cebf, within 0.3%); the extra cost is in IPC and in the fs service. Measure by calling fnd_* in a loop from a /bin command and reading sys_info CPU stats; sample the fs PC from the timer IRQ if you need a profile (symbolize with the driver's -S output through tools/asm Assembler.symbols; function names are hex in `llvm_<hex>` labels).
- `fnd_casefold`/`fnd_fold_next` must keep the ASCII fast path; the table search per ASCII char was over half of an fs lookup. Directory lookups go through a per-directory hash index (fs2.c `dir_index_get`) dropped by `begin_change`. Keep FS2_DIRIDX >= 4: with 2 slots `/bin` and `/lib` evicted each other on every exec and commands got slower than before. The index build reads through `v->scan` (read-ahead buffer only) so it does not evict the 24-line direct-mapped data cache.
- 64-bit `/` and `%` are software loops on SRC32; use 32-bit when the value fits (`read_with`).
- `foundation.wax` now carries FNDBOOT as a block at 0x200000 (like stage1), not linked into the kernel image. Linking it overran the kernel stack area once the modules passed ~900 KB; symptoms were `bad format (-12)` or `limit exceeded (-6)` from init. The fs slot (512 KiB) is nearly full too (code ~330 KB, stack 32 KB, heap mostly /tmp 64 KiB + two volumes): adding a few hundred bytes of fs state can break boot, so check `init:` messages on the PCG screen after growing fs.
- `proc_activate` writes the new regions first and disables only the surplus ones; region checks apply to U mode only, so no disable-all step is needed.
- Process start (about 13 Mcycle per `echo`) is the next thing to look at; not done.

## Extension V / vmfoundation design (2026-10-22)
- `doc/spec_CPU_vm.md` is a draft only: nothing in `src/` implements it and CPUID does not report bit 9. Do not claim emulator or Foundation support.
- Extension P can only allow/deny addresses; it cannot translate. So "emulating" virtual memory on a CPU without V can offer same-link-address processes (swap in/out through a shared window) but not sparse or shared pages. That `mm_swap` backend is design-only (`doc/foundation_vm_plan.md` §9), kept for a possible FPGA port.
- Decisions: S mode untranslated, hardware table walk, V replaces the 8 regions in U (the `0xFFFF0000+` denial stays), no ASID/demand paging/swap in 0.1, so spawn and `mem_grow` must pre-map pages and a fault just kills the process. Foundation ABI stays unchanged; only spec_Foundation §5/§9.5 internals change. Detect V with CPUID, not a probe.

## Debug GUI reset boot entry (2026-10-23)
- DebugUiState::reset_cpu uses the boot PC captured before the first guest instruction, rather than hard-coded zero. GuiApp keeps boot_pc across graphics recreation and updates both it and the current debug state after a successful cartridge drop; failed drops retain the old target.
- The existing loaders select bios.image ahead of the program: raw BIOS starts at 0xFF000000, WAX BIOS uses its declared entry. With no BIOS, reset returns to the program entry (raw=0, WAX may be nonzero). Do not re-read config or reload guest memory when pressing Reset.
- Reset still pauses, clears queued steps and CPU/IRQC state, and preserves guest RAM and bus time. Regression coverage checks raw/non-base BIOS and zero/nonzero program entries on Scalar, Cached, CachedRelaxed and Pipeline, plus cartridge replacement.
- Validation: cargo test --bin cpt32 --no-run, followed by all 25 binary tests from target/debug-reset-test with an empty config.yaml and workspace-local TEMP/TMP/TMPDIR, passed. This isolates package tests from the root BIOS/disk configuration. rustfmt check on src/main.rs and git diff --check passed. No live GUI click was tested. Jira was intentionally omitted at the user's request.

## Dependabot dependency updates, KAN-46 (2026-10-23)
- .github/dependabot.yml covers all six independent Cargo manifests (root, SDK, template and three examples), sol extension npm, llvm-src32 requirements.txt and GitHub Actions. Weekly Monday 09:00 Asia/Tokyo; version PR limit 5 per update entry. Cranelift direct crates share a group to preserve their matching versions. Major versions remain reviewable proposals; no auto-merge is configured.
- The ASM extension declares no dependencies. Inline workflow pip/npx installs and external/system sources are outside manifest updates. When adding a manifest or dependencies, revisit coverage. GitHub must receive the config on its default branch before it runs; alerts/security-update repository settings are separate.
- Review wgpu/imgui-wgpu compatibility (current Cargo.toml warns about 30.x), JIT behavior, WASM packaged artifacts, llvmlite/Clang LLVM major matching and distribution notices after actual dependency changes. See doc/dependency-updates.md. No dependency versions or lockfiles changed in this setup.
- Validation: YAML parsed and passed the downloaded SchemaStore dependabot-2.0 JSON schema; checked Cargo coverage against every tracked Cargo.toml, npm coverage against packages with declared dependencies, existing pip/Actions paths, all weekly JST schedules and the five Cranelift direct crates. git diff --check passed. GitHub job/PR creation is unverified until push; this task does not push.

## Foundation external LZMA WAL demo (2026-10-23, KAN-47)

- `foundation/lib/lzma/build.py` builds the unmodified public-domain LZMA SDK 26.05 C subset with `Z7_ST` as `liblzma.wal`, image ID `lzma`, ABI 1.0. Runtime discovery is `/lib/lzma.1.wal`; the build filename is not the library identity. `examples/lzma-wal/build.py` stages this path and a WAX v3 consumer in a root overlay for `foundation/boot/build.py --root`.
- WAL callbacks must stay inside their module: initialize the SDK allocator function pointers at runtime, then use a caller-owned 8-byte-aligned arena. No heap, libfnd, host syscall, or cross-WAL function-pointer callback is required. LZMA-Alone headers are little-endian even though guest memory and WAL tables are big-endian. XZ/liblzma API, LZMA2, 7z, streaming, and checksums are outside this API.
- A 4 KiB dictionary does NOT imply small encoder scratch space: SDK 26.05 needs more than 1 MiB even for empty/short input with this adapter. The demo uses 2 MiB and reserves a `0x240000` stack region; other inputs/settings may need more. Put scratch on the caller stack rather than static BSS to avoid megabytes of zeros in embedded WAX test fixtures. Bound guest tests generously for SDK table initialization (1200 frames), and prefer the release testbench for this workload.
- Validation: `python -m unittest discover -s foundation/tests/lzma -v` passed all 3 tests, including Python LZMA interoperability and O1/O2 x Scalar/Pipeline actual WAL calls from two differently placed processes. A Foundation disk with the overlay ran `/bin/lzma-demo.wax; echo $?` through the Cached shell and displayed `42` in PCG. License notice generation/check passed. On Windows release a ctypes DLL via `_ctypes.FreeLibrary` before removing its temporary directory.

## Extension V implementation (2026-10-22, KAN-48)
- Implemented in `src/cpu_privilege.rs` (`Vm`/TLB inside `SystemState`; `check_access` + `read_data`/`write_data`), shared by Scalar/Cached/Pipelined. JIT needs no change (U mode already runs in the Cached interpreter). `CPU_FEATURES` is now `0x33F` (pipeline `0x3BF`); tests asserting `0x13F` were updated.
- Contract: every U-mode access must go `check_access` (precise fault, may fill the TLB) and then `read_data`/`write_data` (TLB hit lookup, no side effects). Do not call `bus.read_*` with a virtual address from a CPU path. Page-crossing accesses are done bytewise on the two physical frames; a fault in either page writes nothing.
- PTEs are big-endian words (the draft said little-endian; changed to match `LD`/`ST`). `TLBINV` takes a virtual address, `PTBR` writes flush, and the TLB is architecturally visible: a changed PTE stays unseen until `TLBINV`/`TLBFLUSH` (tested in `tests/cpu_vm.rs`).
- Instruction cache (Cached kinds) is keyed by virtual PC, so translated fetches bypass it. Pipeline carries the fetch fault as `IfId.fetch_fault: Option<Exception>`; `VFAULT` is set when the exception is taken, not when it is detected.
- Bus error while walking is reported as bus error (`0x8002_0000`), not a page fault. Test pitfall: `JR` is `r(0x0B, reg, 0, 0)` (register in rd), and `addi` immediates are i16 (use `LDIL` for 0x8000).

## Foundation on Extension V, `mm` layer (2026-10-22, KAN-49)
- `foundation/kernel/mm.c`/`mm.h` hide two backends behind one interface: slot (Extension P regions, the old behavior) and vm (page table per process). `mm_init()` picks vm when `cpt32_cpu_features() & CPT32_FEATURE_V`; `-D FND_MM_SLOT_ONLY` (`FND_MM=slot` for `boot/build.py`) forces slots, which a CPU without V runs anyway. Both build from one source tree; test both when you touch `proc.c`, `ipc.c`, `blk.c` or the loader.
- Kernel code must NEVER dereference a user address. Check with `proc_user_range_ok`, then move data with `mm_copy_from_user`/`mm_copy_to_user`/`mm_copy_user` (one process to another)/`mm_read32`/`mm_write32`/`mm_read8`. In vm mode S is untranslated, so a user VA is not a pointer; the helpers walk the page table in software, one page at a time. Structs like `fnd_recv` are copied in, edited and copied back.
- vm design choices to remember: S untranslated, `VCTL.EN` set once; `PTBR` write per process switch (flushes the TLB, no ASID); frames from `0x00400000` up (first 64 KiB = `blk_io` bounce buffer, because DMA ignores page tables); frames are zeroed when handed out, not when freed; `mm_vm_map` ORs permissions on a shared page (RX code and RW data on one page become RWX); heap starts on a page boundary; `mem_grow` calls `mm_reload` (TLB flush, the TLB keeps stale PTEs); MMIO grants are Device pages rounded to whole pages.
- Pitfall: the pipelined CPU does not interlock on `CPUID`'s implicit R1/R2 writes (they retire three instructions later). `cpt32_cpu_features` emits three NOPs after `CPUID`; without them the pipeline reads a stale R2 and the kernel silently picks the slot backend. The emulator itself should probably declare R1/R2 as destinations (not done; it is KAN-48's code).
- Pitfall: tests that read physical slot memory (`tests/phase1`, `tests/wax`, `tests/wal`) are slot-only and are built with `-D FND_MM_SLOT_ONLY` in `test_toolchain.py`. `tests/shell` takes `FND_MM=slot` from the environment to pick free-memory expectations. The vm kernel test is `test_foundation_vm_kernel` (`foundation/tests/vm`), whose poke images keep code and data on different pages for that reason.
- Local setup gotcha: llvmlite (LLVM major) and clang must match. Ubuntu 24.04 has clang-20, so `pip install "llvmlite>=0.45,<0.46"` and `--clang /usr/bin/clang-20`; the latest llvmlite needs a clang that apt does not offer. `cargo build` through the proxy may need `CARGO_HTTP_MULTIPLEXING=false`.
- Not done: demand paging/COW/shared WAL pages, more than 7 processes, R-only .rodata block, sub-page MMIO grants (hardware limit), `mm_swap` for CPUs without V (design only, `doc/foundation_vm_plan.md` §9).

## MMU debug window (2026-10-22, KAN-52)
- `SystemState::mmu_snapshot` / `debug_translate` back the `MMU / TLB (Extension V)` debug window (`AnyCpu::mmu_snapshot`, `mmu_translate`). The translate path must stay side-effect free: it reads PTEs with `read_debug_u8` and never fills the TLB or checks permissions, so opening the window cannot change what the guest sees (`tests/cpu_vm.rs` checks the snapshot is unchanged). The window itself is not covered by tests (Xvfb renders black); only the data API is.

## Debug Controls のシステムレジスタ表示 (2026-10-22, KAN-53)
- `SystemState::snapshot` / `AnyCpu::system_snapshot` が STATUS、EPC、CAUSE、USP/UFP/UGP、RGN0..7、VCTL/PTBR/VFAULT、保留 IRQ を返す。USP/UFP/UGP は U モード中は生きたレジスタ（R28..R30）、S モード中は退避バンクなので、`MFSR` と同じ規則で解決する（`reg` を渡す理由）。読み取りは副作用なし（`tests/cpu_vm.rs` が連続取得の一致を確認）。
- 表示しない内部状態: TLB の中身（`MMU / TLB` ウィンドウが担当）、パイプライン段・命令キャッシュ・JIT・スピン検出などの実行機構（アーキテクチャ状態ではない）、IRQC の状態（デバイス側）、割り込みの入力線レベル（保留 IRQ のみ表示）。
- GUI 描画は目視未確認（Xvfb は黒画面）。データ API のテストのみ。


- 2026-10-22 (W^X and MMIO, KAN-49 follow-up): the owner chose the real-OS way. `llvm-src32 --page-align` starts RW data on a 4 KiB boundary and `v3.py` gives the RW block alignment 4096 (only when it already sits on a page); `foundation/boot/build.py` passes it for services, shell, commands and the fnd WAL, so a vm page is never both writable and executable. Images built without it still get the permission union of mm_vm_map. `.rodata` stays inside RW data. MMIO stays page-granular (U-mode accesses inside a mapped page cannot be intercepted); check_grants additionally requires every window the rounded page range reaches to be grantable and, if exclusive, free. The PeC page still exposes timer/RTC to a UART or input grantee. Pipeline CPUID interlock is tracked as KAN-51 (the 3-NOP workaround stays).

- **Foundation C TinyBASIC** (`foundation/bin/tinybasic.c`, `tinybasic_*.h`, `examples/tinybasic-c/main.c`): インタプリタは共用し、`FOUNDATION_GUEST` のときだけ私有ヒープと libfnd の STDIN/STDOUT/fs に切り替える。直接 MMIO・固定物理アドレス・IRQ/HALT を混ぜない。`tinybasic file.bas` はロード→RUN→proc_exit、引数なしは REPL。`LOAD/SAVE "path"` は128KiB以内のテキストファイルで、SAVEは末尾をsector paddingせず、LOADは読込・超過検査完了まで既存プログラムを消さない。入力は console LINE のエコーを使い、ゲスト側で二重エコーしない。テストは `python -m unittest discover -s foundation/tests/tinybasic -v` を既定VMと `FND_MM=slot` で実行し、WAX/WALロード・PCG表示・シェル復帰・ファイル引数と終了コード・FS2上の実ファイルを検証する。Windowsの一時ディレクトリはworkspace配下に設定する。
- Foundation 起動ランナー `foundation/run.py --root DIR` は boot/build.py の追加ルートを渡す。`examples/tinybasic-c/foundation-root/demo.bas` を入れれば、シェルで `tinybasic demo.bas` を実行できる。TinyBASIC追加時の検証では、専用テストは既定VM/slotで各6件、元のC単体版は5件通過。シェル全18件中、12MiB構成のfree期待値だけが失敗し、TinyBASICをディスクから除去しても再現した（free_kibが0 KiB freeを拒否）。本機能の成否と混同しない。

## Foundation free のバックエンド判定 (2026-10-23, KAN-55)
- `FND_MM` 未設定は VM の強制ではない。`mm_init` は実行中の CPU の CPUID bit 9 で VM/slot を選ぶ。12 MiB の slot 構成では console/disk/fs/init/shell/実行中コマンドが slot 0〜5 を占め、`free` の 0 KiB は正当。VM の空き容量とは同じ期待値にしない。
- シェルハーネスは `SRC32_TESTBENCH`、release、debug の順で実行ファイルを選ぶ。本件では古い release が CPUID=0x13F、新しい debug が 0x33F で、release の優先選択が原因だった。ソースの機能値からロード済みバイナリの機能を推測しない。VM 検証時は testbench を再ビルドするか絶対パスで明示する。
- `foundation/tests/shell/cpu_features.a` を実際の testbench/CPU で走らせて V を確認し、`FND_MM=slot` または V なしでは正確な slot 空き容量、V ありでは正の VM 空き容量を検証する。`FOUNDATION_TEST_CPU` とケース別 CPU 指定を反映し、判定を CPU ごとに保持する。VM の 0 を許すように検査を緩めてはいけない。
- 検証: 更新した release testbench でシェル全18件が既定VM/強制slotの双方で通過、共用ハーネスの TinyBASIC 6件も通過。さらに Cached の CPUID の V ビットだけを無効化した一時ビルド（CPU ソースは直後に復元・通常 release も再ビルド）で、FND_MM未設定の12MiB free・16MiB banner/free・Pipeline top の3件が通過。1つのハーネス内で Cached=slot/Pipeline=VM を切り替える経路も確認した。

- 2026-10-22 (KAN-51, CPUID interlock): `PipelineCpu::forwarded` now forwards `CPUID`'s implicit R1/R2 (CPU_ID/CPU_FEATURES) from EX/MEM and MEM/WB (`cpuid` flag), so the consumer right after `CPUID` sees them; a younger writer of R1/R2 in a newer stage still wins (checked in order EX/MEM dest, EX/MEM cpuid, MEM/WB dest, MEM/WB cpuid). Tests: `cpuid_results_forward_to_immediately_following_readers`, `younger_writer_of_r1_wins_over_cpuid_forwarding` in tests/cpu_pipeline.rs (the first fails without the fix). `cpt32_cpu_features` no longer emits the 3 NOPs (supersedes the workaround pitfall above).
- Not done: demand paging/COW/shared WAL pages, more than 7 processes, W^X for blocks sharing a page, sub-page MMIO grants, `mm_swap` for CPUs without V (design only, `doc/foundation_vm_plan.md` §9).

## Foundation file commands (2026-10-23, KAN-17)
- `/bin/{cat,cp,rm,mv,mkdir,rmdir}.wax` (`foundation/bin/*.c`, listed in `COMMANDS` of `boot/build.py`) follow `ls`: one source plus `cmd.c`, WAX v3 importing WAL fnd 1.1. Errors go through `cmd_report` (§11.1); a failed target does not stop the rest and `$?` is the first error (§10).
- `fnd_file_rename` was in API §11.4 but missing from libfnd; it is now in its own `lib/fnd/rename.c` (WAL only, in `fnd.exports`, additive in 1.1). Do NOT put it in `start.c`: static users link all of start.c, and +2.4 KB in fs.wax (383 KB of the 512 KiB slot 3) left no heap for the 64 KiB /tmp RAM volume, so /tmp silently vanished (`test_redirects_reach_the_disk` failed with `/tmp: not found`). fs has almost no heap headroom; watch fs.wax size when touching libfnd. `FS_RENAME` data is old+new path concatenated, `a0` = old length; new already existing is `FND_E_EXISTS`, another mount is `FND_E_CROSS_DEVICE`. Messages are "already exists", "not a directory", "directory not empty", "is a directory".
- `cp` copies with 4 KiB `fnd_read`/`fnd_write` blocks (IPC limit), `CREATE|EXCL` normally and `CREATE|TRUNC` for `-f`. "Same file" is detected by path text only: `fnd_stat_t` has no inode and `-f` truncates on open, so `cp -f a ./a` would empty the file. An existing-target or same-file error names the target; other errors name the source.
- Shell tests: the screen has 30 rows and scrolls, and each test has only 600 frames, so a long command list silently stops before the last prompt. Keep command lists short and compare the tail of the rows (`rows[-N:]`). Use `'\\n'` (literal backslash-n) in `--type` texts.
- Local setup: llvmlite 0.50 (LLVM 22) with clang-18 still built Foundation at -O1 here; the apt.llvm.org script is blocked by the proxy, and `apt-get update` is needed before `libasound2-dev libudev-dev`.
- fs heap is the real limit (found with KAN-17): fs is a static WAX v2 in slot 3 (512 KiB); the image is ~381 KB (code, not data: llvm-src32 code is large), the stack 32 KiB, so the heap is ~110 KiB. It holds two `fs2_vol` (~21 KiB each: 32-sector cache + read-ahead) plus the /tmp RAM image. With /tmp at 128 sectors (64 KiB) a `/bin` of 14 commands made every `fs_alloc` for a directory index fail, and every spawn answered `out of memory (-5)` (even `free`, which never touches a file). /tmp is now 96 sectors (48 KiB). The symptom is a shell where all external commands say `out of memory`; check fs headroom before blaming the new command. A durable fix is a smaller `fs2_vol` for RAM volumes or a bigger fs slot.

## /bin/hex (2026-10-23, KAN-56)
- dos の HEX（`dos.sol` と `examples/dos-c/main.c`、どちらも同じ書式）を `foundation/bin/hex.c` へ移植した。画面は 16 行×16 バイト＋ASCII 欄、コマンドは N/P/G/E/W/Q（DOS の別名 NEXT/PREV/GOTO/SET/SAVE も）、メッセージの文言は DOS のまま。DOS の `HEX NAME` は 8.3 名だったが、こちらは `hex path`。
- stdin は console の LINE モードで 1 行ずつ `fnd_read` する（RAW にしない）。画面消去は `fnd_con_clear`、保存は `FND_OPEN_WRITE|TRUNC` で開いて全体を書き `fnd_sync`。保存は同サイズで、ファイルは 64 KiB まで、空ファイルは編集不可（DOS と同じ）。Ctrl+D（EOF）も終了扱い。
- テストは `foundation/tests/shell` の `test_hex`（要 `CLANG=clang-20` と llvmlite 0.46）。エディタは毎回 `fnd_con_clear` するので、画面比較は終了後の最終画面（直前の hex 表示が残る）で見る。

## Foundation 10 ms tick and quantum (2026-10-23, KAN-57)
- `FND_TICK_HZ=100`, PeC ch0 compare/reload = `48000000 / FND_TICK_HZ` (480,000 cycles), `FND_SLICE_TICKS=1`. The old quantum was already 10 ms (10 x 1 ms); this change reduces periodic IRQ4 frequency to 100 Hz while keeping the quantum at 10 ms. Sleep and IPC timeout arguments remain ticks, now 10 ms each; real-time conversions must use `time_get(FND_TIME_TICK_HZ)` / sysinfo.tick_hz. PeC ch1 accounting stays at 48 MHz.
- Fixed internal waits also need conversion: console polling and top key polling = 1 tick, init retry = 1 tick and respawn = 10 ticks (100 ms), fs registration retries = 10 x 1 tick (100 ms), demo display = 10 Hz and ticker = 25 ticks (250 ms). The old 2/4 ms polls round up to the new minimum of 10 ms.
- Phase1 runs 100 ticks (1 second) and checks after tick 50, after the probe completes five 7-tick sleeps. It checks exactly one counter progresses per tick and independently measures the interval with PeC ch1 (hardcoded 480,000 cycles/tick, two-tick latency tolerance), so a 1 ms tick or a 100 ms quantum cannot pass just by changing shared constants. The host frame budget is 90.
- Preserve enough wall time in integration tests: shell stop = 800 ticks with 660 frames for long file-command batches; disk stop = 600 ticks (6 seconds). WAX/VM rounds still wait 40 ticks and their frame budgets are 180. Merely dividing the old shell stop count by ten cut off longer command lists midway.
- Rebuild the actual testbench after CPU changes: the pre-existing release binary still lacked the Pipeline CPUID interlock, causing the VM kernel to return 4 after the source workaround NOPs had been removed. Do not rebuild an executable while a test is running it on Windows (file removal gets access denied). Use an absolute SRC32_TESTBENCH path; debug builds can time out on the longer scheduler run.
- Validation: rebuilt release src32_testbench; phase1, WAX loader and VM kernel each passed O0/O1/O2 x Scalar/Cached/CachedRelaxed/Pipeline (12 executions each); shell 21 tests passed under default VM and forced slot; disk boot 7 tests passed. git diff --check passed. IRQ frequency reduction is configured and the guest timer period is measured; no application-throughput speedup is claimed.

## Foundation kernel /dev/fb0 (2026-10-24)

- `foundation/kernel/fb.c` is the VDP bitmap/CLUT kernel driver. `/dev/fb0` resolves through the usual fs `DEV_OPEN` forwarding path to reserved `dev.fb0`. `ipc.c` distinguishes kernel factory/stream CHANNELs from user-service CHANNELs: their NULL port is intentional, not PEER_GONE. Calls validate both message/buffer ranges, use `mm_copy_*`, and reply synchronously without blocking the thread. Last close releases the device directly instead of sending CLIENT_GONE; dup, transfer, inherited handles and proc_exit keep normal reference counting.
- One open stream at a time, shared position/rights on dup. New streams select INDEX8; WRITE open enables graphics and disables PCG overlay. The three tightly packed 320x240 planes are INDEX8 at VRAM+0, big-endian RGB555 at +0x100000, RGB888 at +0x200000. CLUT is 256 RGB888 entries at +0x12C00. Validate palette ranges by subtraction before multiplying; a write must have exactly count*3 bytes. SET_FORMAT checks actual BMC VRAM capacity before mutating format/position (zero means unknown, fail closed).
- INDEX8 overlaps the console's PCG text/attribute planes. Save/restore VRAM[0..0x3000), all 768 CLUT bytes, enable/display/format/overlay on the last WRITE-stream close, including fault/kill/proc_exit and handle-allocation failure. Bitmap data outside those saved planes survives. This is a snapshot, not console arbitration: PCG/CLUT output made by other processes during the lease is overwritten on restore. Do not print to console while drawing. SYNC is immediate, not VSync.
- Guest C helpers are header-only in `foundation/include/foundation/fb.h` over existing WAL fnd_call, so there are no new WAL exports. Read/write use existing libfnd chunking; seek is bounded to [0,size], crossing-end writes fail without mutation. READ-only open does not change the display. Example: `examples/foundation-fb`; behavior/protocol: Foundation API spec §9.1.
- When adding a kernel source, update boot/build.py, CMakeLists.txt, toolchain kernel test source lists, lzma test builder and README direct compile commands together. `foundation/tests/fb` exercises shell -> fs -> transferred kernel channel in default VM and forced slot, including all formats, CLUT, permission/range errors, exhaustion rollback, dup lifetime, leaked-handle exit, user MMIO fault and the drawing example.
- Testbench `--expect-mem-u8` checks EVERY frame, starting frame 0; it cannot assert a value written later by a booted guest. Do not use it as a final-memory assertion. Emulator config requires >=4 MiB VRAM, so a 1 MiB test is rejected before the guest runs. Run CPU feature probes in an isolated cwd with empty config.yaml, as root config may attach real host disks.
- Verified: fb integration 4/4 and shell 21/21 on default VM and forced slot; FS host 28/28 plus focused fb0 device-path check; VM and phase1 kernel regressions each O0/O1/O2 x Scalar/Cached/CachedRelaxed/Pipeline. The actual release testbench's Cached CPUID reports Extension V. Jira MCP timed out, so no task was created/updated in this session.

## Foundation original CHIP-8 and /dev/sgu (2026-10-24)

- `foundation/bin/chip8.c` is an ordinary WAX v3 command linked to fnd WAL. Build registration is in boot/build.py COMMANDS. It loads 1..3584 ROM bytes at 0x200, draws a 64x32 image at 5x scale with 40-pixel letterboxing through /dev/fb0, and uses SGU0 channel0 for a 440 Hz square wave. Original-language choices: VY shifts, I increment after FX55/65, logical VF reset, clipped sprites with modulo start coordinates, draw wait, and key release wait. COSMAC 1802 machine-code calls (0NNN), SCHIP and XO-CHIP are explicitly unsupported; this is not cycle-accurate VIP hardware emulation.
- `foundation/kernel/sgu.c` exposes both SGUs and shared PCM RAM through one exclusive /dev/sgu stream. Offsets 0..0x1fff correspond to MMIO 0x80020000..0x80021fff; offsets 0x2000..0x101fff correspond to the shared 1 MiB PCMRAM. Stream size is 0x102000. Seek/read/write/stat/sync use existing IPC and mm_copy helpers; dup shares position and lease. PHASE_RESET reads return zero without touching hardware; write intentionally resets phase. WRITE open and last close mute all 16 volumes and stop all 16 PCM controls, including process fault/exit and failed stream-handle allocation. Other registers, wavetable and PCM data persist; old playback state is not restored. Read-only leases do not silence. Raw privileged MMIO grants are outside this stream's arbitration.
- Add new kernel source dependencies to boot/build.py, CMakeLists.txt, the four toolchain kernel source lists, the LZMA test builder, and README direct commands together. IPC kernel CHANNEL kinds are fb factory/stream=1/2 and SGU factory/stream=3/4; last close must use the correct driver cleanup. Device nodes also affect FS directory enumeration expectations.
- CON_KEY_STATE (0x1008, foundation/keyboard.h) returns 96 bytes: HID page7 down/pressed/released bitmaps (8 words each). Physical state supports holding and multiple keys; repeat presses do not create edges. Edges are shared globally and cleared by a snapshot query; the normal CON_READ_KEY queue is independent. A short press+release between queries sets both edge bits. Edge ordering/repetition and events lost to PeC FIFO overflow cannot be reconstructed.
- CHIP-8 timing is based on Foundation monotonic ticks, with a fractional 60/Hz accumulator and a separate 700/Hz instruction budget; timers continue during key/draw waits. Do not derive another emulator IRQ from CPU cycles. SGU frequency registers are directly in Hz (0x01b8 = 440), not a phase-step conversion. Sound timer set to 1 is silent; values >=2 gate sound through countdown to zero.
- `examples/foundation-chip8/demo.py target/chip8-root/demo.ch8` creates a new repository-owned keypad/font/beep ROM. Build/launch via `python foundation/run.py --root target/chip8-root`, then `chip8 /demo.ch8`. Details and limitations are in doc/spec_Foundation_chip8.md.
- Testbench --key FRAME:USAGE:0|1 injects physical transitions; --expect-final-mem-u8 checks a value only after the frame loop, unlike the existing every-frame --expect-mem-u8. MMIO assertions are intentional reads: never target a register with read side effects. Active framebuffer assertions need --allow-running before the test kernel halts. --allow-running --profile-cpu retains expected-R1 independence without invoking unrelated built-in render expectations.
- Integration tests must send echo/status commands after an interactive app has exited: CHIP-8 legitimately drains CON_READ_KEY while running, so commands queued behind a ROM that fails on its first opcode can be consumed as keyboard input. Also bound each --type batch to the PeC FIFO capacity. Testing CON_KEY_STATE does not consume queued text: drain it before returning to the shell. Keep the shell test's 800-tick stop budget in mind when scheduling later commands.
- Validation: CHIP-8 host+guest suite 10/10 on default backend and 10/10 with FND_MM=slot; existing shell suite 21/21; FS host suite 28/28. Real guest assertions cover the sample V->F glyph, active SGU wave/frequency/volume, timeout silence, all 16 channel wave/control access, shared PCM boundary accesses, dup lifetime, leak/fault cleanup, and physical simultaneous holds/releases. Speaker output/perceptual quality is not verified. `foundation/run.py --build-only --root target/chip8-root` built the ready Foundation image, sample ROM disk, emulator and BIOS-free runtime config. git diff --check passed.
## /lib/lzma.1.wal in the default image (2026-10-23, KAN-58)
- `foundation/boot/build.py` now builds `foundation/lib/lzma` (via its `build()`, same `--clang` and `-O` as the rest) into `out/lzma/liblzma.wal` and copies it to `root/lib/lzma.1.wal`, so every `foundation.img` has it (mode 0x3D like fnd). The example overlay still works: same path, later tree wins.
- Choice: reuse the KAN-47 LZMA SDK adapter (public domain, no new port). xz-embedded/new code was not needed. The WAL is ~345 KB (llvm-src32 code size) and uses no heap or libfnd; the caller passes a 2 MiB arena, so fs heap is untouched. Cost: ~1 s more build time and 345 KB of the 8 MiB disk; boot time is unaffected (the file is only read when a program imports it).
- Needs clang-20 + llvmlite 0.46 here (apt-get update first). Only the host image check (fs2 read-back, mode, check_image) was run for this change; disk/shell testbench suites were not rerun.

## sol-libfnd テストの stop-ticks (2026-10-23, KAN-58)
- KAN-57 で tick が 100 Hz になったため、`tools/solc/test_sol_libfnd.py` の `--stop-ticks 40000`（旧 1 kHz で 40 秒）は 400 秒相当になり、テストが R1=0（42 でない）で失敗していた。同じ 40 秒相当の 4000 に直した（`examples/foundation-sol/README.md` も）。tick 数で書く値は、周波数を変えたら必ず秒数で換算し直す。
- 検証: release testbench で `test_libfnd_sample_in_real_foundation` の scalar/pipeline 2 件が通過（約 6 分）。

## llvm-src32 可変長引数と softfloat (2026-10-23, KAN-59)

- Variadics: Clang's MIPS o32 `va_list` is a `char *`; `va_arg` is inlined (aligning 8-byte types with `llvm.ptrmask.p0.i32`), the backend only implements `llvm.va_start` (= FP+8+size of the named arguments), `va_copy` (copy the word), `va_end` (nothing) and ptrmask. Call sites align each variadic argument to its type alignment (max 8) relative to SP, which is 8-aligned, so the callee's realignment finds it. Fixed arguments stay packed (an i64 after an i32 is not aligned), so the variadic part is the only place padding appears. The variadic position is parsed from the call's explicit function type `(i32, ...)` (`fixed_argument_count`). Structs passed to `...` arrive as `inreg` i32 words and need nothing; `byval` in the variadic part is rejected because va_arg reads in place. `<stdarg.h>` is in runtime/include (`-nostdinc` hides Clang's own). WAL/import boundary: variadic still rejected (`check_prototype`).
- Floating point: `float`/`double` are bit patterns, `_bits()` returns 32/64 so loads, stores, phis, selects, call arguments and results, bitcasts and struct layout all reuse the integer paths (double = high word first). `fp.py` is the single table of fp instruction -> compiler-rt routine (`__addsf3`, `__ltdf2`...), used by the driver (to decide what to link) and the backend (to emit calls and keep the routines alive). Comparison routines follow compiler-rt's unordered results; `fcmp` predicates are tests on them (`fp.PREDICATES`), so a change to rt.c's unordered return values breaks `ule`/`ugt`/... Frame constants: `literal_bits` parses LLVM's hex-double spelling (even for `float`) without losing NaN payloads; `data.py` serializes float/double globals.
- Runtime: `runtime/softfloat` = unmodified Berkeley SoftFloat 3e subset (the non-FAST_INT64 build: only 32/64-bit integer ops, no `__int128`) + `rt.c`. The driver compiles it at -O2 (threaded) only when the linked module has fp instructions or declares (without defining) a routine from `fp.entry_points()`. `SRC32_HAVE_<name>` drops what the program defines; a translation unit that defines any name the program already owns is skipped (programs vendoring SoftFloat, like examples/softfloat-c, still work). The backend gets `prunable` (names of runtime functions) and emits only reachable ones, text-scanning `@name` plus fp instructions' routines: a float add plus compare costs ~12 KB, a program without fp costs nothing. Berkeley's `_r_minMag` conversion takes `exact` (not a rounding mode); minMag truncation is what C's casts need.
- Gotchas: `link_in` consumes the llvmlite module (collect names before linking). `-ffp-contract=off` keeps Clang from emitting `llvm.fmuladd` (still handled for IR input as mul then add). `half`/`fp128`/vectors raise from `fp.kind`. fnptr WAL signatures and float/double in WAL signatures remain unsupported (pass bit patterns as u32/u64); plain C function pointers (also variadic or float-typed) work, also inside a WAL (foundation/tests/wal math.c).
- Tests: `fp_vectors.py` generates a ~1 MB C program of IEEE vectors from the host (checked at -O0/-O2, 400 frames; I mutated one expectation to confirm it fails), plus test_variadic_*, test_float_*; `foundation/tests/wal` now has a variadic and a float function in the PIC WAL (bit patterns: 1.5+2.25+0.5 is 0x40880000, not 0x40900000). Container: ubuntu `clang-20` (after `apt-get update`) + llvmlite 0.46; `SRC32_TESTBENCH=target/release/src32_testbench`.


## Main merge resolution (2026-10-24)

- MEMORY.md append conflict: retain both local Foundation framebuffer/CHIP-8/SGU contracts and incoming LLVM/SRC32 variadic/SoftFloat runtime guidance. These describe independent subsystems; neither side supersedes the other. The pre-existing black-screen fix working-tree changes remain separate from the merge.

## Foundation fb0 black-screen regression (2026-10-24, KAN-61)

- Common VDP DISPLAY_ENABLE at 0x80000000 is active-high: write 1 to show the display. PCG_DISABLE at 0x8000f000 has the opposite polarity: write 0 to show it. fb_open previously wrote 0 to the common register, so graphics stayed black despite correct VRAM and CLUT. WRITE open now sets the common register to 1; last close still restores its saved raw value. See src/devices/vdp/vdp.rs write_common_register/write_pcg_register and doc/spec_VDP.md.
- Prior fb/CHIP-8 memory assertions and successful console restoration did not establish visible output: they could pass with the display disabled. Test actual VdpFramebuffer::get_pixel results with src32_testbench --expect-pixel X:Y=0xRRGGBB. --dump-framebuffer PATH.ppm captures that same composed display. New assertions reproduce the old driver failure (expected white at 0,40, actual black), then pass with the one-bit fix.
- CHIP8_TEST_ROM points to an explicit user-provided external ROM for the optional integration reproduction; CHIP8_TEST_CAPTURE saves its PPM output. Do not vendor the ROM or imply repository licensing covers it. The reported octojam2title.ch8 is 1240 bytes and its 20 statically reachable instructions are original CHIP-8 operations; its blank screen was the fb enable bug, not an unsupported extension.

- Validation: pre-merge CHIP-8 suite with the reported ROM 11/11, forced-slot visible-output tests 3/3, fb integration 4/4; after the main merge, rebuilt with the updated LLVM/SRC32 toolchain and passed all 3 composed-display tests again (normal glyph, original sample, reported ROM). The reported ROM capture contains 23,175 nonblack pixels and shows the Octojam title. Rebuilt the normal Foundation image with /octojam2title.ch8 and /demo.ch8 in the disk root; the pre-rebuild disk is saved as target/foundation-boot/foundation-before-display-fix.img. No external ROM is committed. Focused diff --check passes.

## Jira tracking scope (2026-10-24)

- AGENTS.md now exempts minor changes without functional impact from Jira creation/updates: ordinary Git operations, mechanical merges/conflict resolution, commit/rebase cleanup, typos and minor development-guide edits. Do not create separate Git-operation issues alongside an implementation/bug issue. Conflict resolutions that decide or change behavior remain tracked under the relevant implementation/bug issue. This rule was explicitly requested by the user to avoid tracking overhead.

## CHIP-8 cycles-per-frame command-line option (2026-10-24, KAN-63)

- `chip8 [-c N|--cycles N] ROM.ch8` accepts decimal N=1..10000 before or after the ROM. `--` terminates option parsing. A cycle means one CHIP-8 instruction; it does not mean an SRC32/1802 hardware cycle. No option preserves the existing 700 instructions/second rate. Bad/empty/overflow/zero/duplicate option values, unknown switches and extra/missing ROMs fail with BAD_ARG before opening the ROM or devices. Error reporting uses the parsed ROM path, not argv[1].
- `foundation/bin/chip8_options.h` contains the actual shared parser and CPU-budget helper. In explicit mode, the helper accumulates 60 Hz boundaries from Foundation ticks and grants N instructions per boundary. This is 60 Hz VSync-equivalent budgeting, not a wait on host IRQ0. In default mode it retains the old 700/Hz fractional budget. CPU catch-up remains capped at 250 ms; key/draw waits discard unused credit and delay/sound continue at 60 Hz independently. Multiplying elapsed ticks by 60/700 before multiplying whole frames by N keeps u32 intermediates bounded for the validated tick-rate range.
- The SRC32 freestanding string.h provides memory functions and strlen, but not strcmp. Argument parsing uses a small local equality helper; a host build alone would incorrectly accept strcmp via the host libc. Always compile and execute the guest argument path too.
- Host tests check parsing, overflow/ranges, 60 frame budgets and 60 timer decrements across multiple scheduler rates, legacy 700 instructions/second, and bounded catch-up. Guest tests run a ROM that needs about 3,080 instructions before drawing: -c 1 stays black during the observation window, while a trailing --cycles 100 reaches the expected white glyph. They also check usage/status errors through the shell.
- Rebuilt the installed WAX with the normal WAL, origin, stack and image ID; updated only /bin/chip8.wax in the existing Foundation disk and ran fs2.check_image before/after, preserving the other files. The old disk is saved as target/foundation-boot/foundation-before-cycles.img.
- Validation: host parser/budget/timer tests 4/4; new guest CLI/slow/fast tests 3/3 in normal and forced-slot configurations. The normal suite also passed existing display, input, cleanup and sample checks. One pre-existing ROM-status test consumed its queued echo; changed it to `chip8 /bad.ch8; echo bad=$?` so the shell owns the continuation, then re-ran that test successfully (1/1). The earlier full run had 13 passes, that one failure and one optional external-ROM skip; it was not rerun in full after the test-only correction. Installed WAX build, read-back equality, FS2 checks and git diff --check passed.
