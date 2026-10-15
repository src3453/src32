## 作業で得た知識・注意点

- WAX v1 (doc/spec_WAX.md): .wax/.wal は同一 64B ヘッダのコンテナで種別のみ違う。ロード先は
  section ごとではなく base ひとつ + entry_offset、debug は NOLOAD の付録。ヘッダ複数バイト項目は
  little-endian。ローダはマジック sniff で素朴 .bin (base 0/PC 0) と両対応し、CPT32PKG manifest は不変。
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
