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
- 既知の制限：別ファイル（`!include ... as ns` の中身）に定義された `ns.member` は member 側を
  function/constant/variable に解決しない（include 展開の再実装はコンパイラとずれるため意図的に
  やらない）。prefix だけ namespace 色になる。
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
