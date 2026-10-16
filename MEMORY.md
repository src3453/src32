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
