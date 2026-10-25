---
lang: ja
translation_of: ../spec_WAX_v3.md
---
# WAX/WAL v3 — CPU 保護とライブラリシンボル

Revision 3.1、2026-10-25。状態: **仕様策定済み・Foundation カーネルが読み込み可能**。
v3 のバイナリ形式と基本 OS ローダー契約を定義します。
「必須」「禁止」は規範要件です。[英語版](../spec_WAX_v3.md)。

[v2 仕様](spec_WAX.md)と既存 parser、assembler、C toolchain、DOS loader の
動作は維持します。v3 を読み込むのは Foundation カーネル（`foundation/kernel/wax.c`、
選択と配置の方針は [Foundation §9.5](../spec_Foundation.md)）です。`tools/llvm-src32`
は C から call ABI 1 の v3 WAX と WAL を作ります（`--wax3`、`--wal`、`--import-wal`。
同ツールの README を参照）。`tools/wax/wax3.py` は用意済みの block とシンボル表から
v3 ファイルを組み立てるもので、コンパイラとローダーのテストが使います。
magic は共通ですが version=3 であり、v2 reader は拒否します。
v2 の version だけを書き換えても変換にはなりません。

## 1. 役割と識別

- WAX は固定アドレスの**ユーザー実行ファイル**です。スタック、block、直接依存する
  WAL を宣言します。静的リンク済みコードは WAX の一部であり、実行時依存ではありません。
- WAL は**ユーザーモード用 PIC ライブラリ**です。独自スタックやプロセス entry はなく、
  単独起動できません。export と必要なら別 WAL への直接依存を持ちます。
  ユーザープログラムの代わりに S モードで実行することはありません。
- 両 type とも拡張 P と flags bit1 `PROTECTED` が必須です。
  BIOS／カーネル用 S モード image は引き続き v2/raw を使います。
  v3 は supervisor module を定義しません。
- ライブラリの識別は `(image_id, abi_major)` です。ファイル名・path とは別です。
  provider の `abi_minor >= min_minor` なら要求を満たします。
  minor 更新では旧 ABI と意味を維持し、破壊的変更では major を変えます。
- コンテナー版、呼出 ABI `call_abi`、ライブラリ ABI major/minor、
  OS syscall ABI `os_id/os_major/os_min_minor` は別の識別軸です。
  OS は未対応の組合せを拒否し、名前から互換性を推測してはいけません。

基本仕様は起動時の一括解決、関数／object export、プロセスごとの WAL instance、
非循環の依存 graph を対象とします。遅延解決、weak／大域シンボル／差し替え、TLS、
再配置、暗黙の constructor/finalizer、実行中の load/unload、プロセス間共有は
定義しません。初期化・解放が必要なら通常の関数として明示的に公開します。

## 2. 表現と header

整数はすべて unsigned big-endian、単位は byte、範囲は半開区間です。
`count * entry_size` を含むすべての演算は、u32 に狭める前に overflow を検査します。
実ファイル長は u32 内でなければなりません。

header は固定 160 byte。空 table でも entry_size は規定値です。
空 table は offset=count=0。image-relative と記載したもの以外の offset は
ファイル先頭基準です。reserved はすべて 0 必須です。

| Offset | Size | Field / 規定値 |
|---:|---:|---|
| 0 | 4 | magic `57 41 58 00` (`WAX\0`) |
| 4 | 2 | version = 3 |
| 6 | 2 | type: 1 WAX、2 WAL |
| 8 | 2 | arch = 1 (SRC32) |
| 10 | 2 | flags: bit0 DEBUG_PRESENT、bit1 PROTECTED、bit2 RELOCATABLE（§3.1）、他は 0 |
| 12 | 4 | header_size = 160 |
| 16 | 4 | file_size = 実ファイル長 |
| 20 | 4 | call_abi = 1（§6） |
| 24 | 4 | base_addr: WAX 最小 block address、WAL は 0 |
| 28 | 4 | entry_offset: WAX の image-relative Normal-mode entry、WAL は 0 |
| 32 | 4 | user_stack_top: WAX の絶対終端アドレス、WAL は 0 |
| 36 | 4 | user_stack_size: WAX は非ゼロ、WAL は 0 |
| 40 | 4 | block_table_offset |
| 44 | 4 | block_count: 非ゼロ |
| 48 | 4 | block_entry_size = 24 |
| 52 | 4 | dependency_table_offset |
| 56 | 4 | dependency_count |
| 60 | 4 | dependency_entry_size = 16 |
| 64 | 4 | export_table_offset |
| 68 | 4 | export_count: WAX は 0、WAL は非ゼロ |
| 72 | 4 | export_entry_size = 24 |
| 76 | 4 | import_table_offset |
| 80 | 4 | import_count |
| 84 | 4 | import_entry_size = 24 |
| 88 | 4 | string_table_offset |
| 92 | 4 | string_table_size: 非ゼロ |
| 96 | 4 | image_id: string offset |
| 100 | 4 | abi_major: 非ゼロ。WAX ではアプリ契約の版 |
| 104 | 4 | abi_minor |
| 108 | 4 | os_id: syscall ABI の string offset |
| 112 | 4 | os_major: 非ゼロ |
| 116 | 4 | os_min_minor |
| 120 | 4 | debug_offset: 不在なら 0 |
| 124 | 4 | debug_size: 不在なら 0 |
| 128 | 4 | reloc_table_offset: ファイル先頭からの位置。RELOCATABLE のときだけ非ゼロ |
| 132 | 4 | reloc_count: RELOCATABLE のときだけ非ゼロ |
| 136 | 4 | heap_hint: プログラムが求めるヒープの byte 数（8 の倍数）。無ければ 0。RELOCATABLE のときだけ指定可 |
| 140 | 20 | reserved |

header、各 table、string table、非空 payload、debug のファイル範囲は
相互に重ならず、file_size 内に収まる必要があります。開始 offset はすべて 4 の倍数。
header 後の配置順は任意、padding は 0。debug は不透明な NOLOAD データで、
DEBUG_PRESENT は debug_size > 0 の場合に限り設定します。
未知の version/type/arch/flag/ABI ID は拒否します。checksum・真正性保証はありません。

### 2.1 文字列

string table の先頭 byte は NUL。string offset は table 内の非空 NUL 終端文字列の
先頭を指し、直前 byte は NUL、終端も table 内でなければなりません。
大文字小文字を区別する ASCII で、正規化はしません。offset=0 は、後述する
「文字列なし」を許可するフィールドでのみ使用します。
未使用部分も完全な NUL 終端文字列またはゼロ padding にします。

image/dependency/OS ID は `[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*`、
symbol 名は `[A-Za-z_][A-Za-z0-9_]*`、いずれも NUL を除いて最大 127 byte。
ID は path ではありません。OS の検索規則で ID/major からファイルを決め、
image に検索 path を指定させません。

## 3. Block とアドレス

block entry は 24 byte、次の u32 × 6 です。

| Offset | Field | 意味 |
|---:|---|---|
| 0 | address | WAX は絶対アドレス、WAL は確保基底 B からの offset |
| 4 | file_offset | payload 位置。file_size=0 の場合に限り 0 |
| 8 | file_size | 初期化データの byte 数 |
| 12 | memory_size | 確保 byte 数。非ゼロ、file_size 以上 |
| 16 | perm | R=1、RW=3、RX=5 のいずれか |
| 20 | alignment | 4 以上の 2 のべき乗。address はその倍数 |

address 昇順で、memory range の重複は禁止です。WAX の base_addr は先頭 address。
WAL の先頭 address は 0 必須、B は全 block の最大 alignment に揃えます。
WAL は `[B, B + max(address + memory_size))`、WAX は最小から最大 block 終端までを
hole 込みで予約します。別 image の予約範囲およびスタックとの重複は禁止です。

WAX の `origin=base_addr`、WAL の `origin=B` とします。
entry・symbol・import slot の **image-relative offset** は `origin + offset`。
WAX block の address に origin を二重加算してはいけません。
WAL block のロード先は `B + address` です。

file_size byte をコピーし、memory_size までの末尾をゼロ化します。
hole は未マップで、暗黙の消去・アクセス許可はありません。
metadata も暗黙にマップしません。全配置先は実在する main RAM 内であり、
OS・他プロセス・staging・その他予約領域の外でなければなりません。
VRAM/device/ROM/CPU 予約領域を要求することは禁止、W+X も禁止です。

WAX entry は 4-byte 整列、先頭命令 4 byte 全体が初期化済み RX に収まる必要があります。
stack top/size は非ゼロの 8 の倍数、減算で underflow せず、image 予約範囲と重複不可。
OS はスタック全体をゼロ化します。WAL の stack/entry フィールドはすべて 0 です。

### 3.1 再配置可能な実行形式

flags の RELOCATABLE を立てた WAX は、nommu システムの FLAT 形式のように、
どのアドレスにでも読み込めます。`base_addr`、block の address、entry、スタックは
**リンク時**の配置で、ローダーは最大 alignment の倍数で、すべての範囲が RAM に収まる
`delta` を選び、image 全体を `base_addr + delta` に置いて relocation table を適用します。
再配置可能にできるのは WAX だけです（WAL は構造上すでに位置独立）。

relocation table は `reloc_table_offset` にある u32 を `reloc_count` 個並べたもので、
address の厳密な昇順です。各エントリは上位 30 bit がリンク時アドレス（4 byte 整列）、
下位 2 bit が種類です。

| 種類 | 意味 |
|---:|---|
| 0 | R/RW block にある 32 bit big-endian のデータ語。`delta` を足す |
| 1 | RX block にある `LDIH rd, hi` とそれに続く `LDIL rd, lo`（rd は同じ）。`hi:lo` の 32 bit 値はリンク時アドレスで、`delta` を足して両方の即値を書き直す |

それ以外の種類は予約で、拒否します。各エントリは、対応する権限の block の
初期化済み領域に完全に収まらなければなりません。entry や image-relative offset、
import slot は配置後の origin からの相対なので、エントリは要りません。
再配置はプロセスの実行前、スタックのゼロ化より前に行います。

再配置可能な WAX のスタックは最下位の block の直下にあり（`user_stack_top == base_addr`）、
予約範囲 `[base_addr - user_stack_size, 最上位 block の終端)` は 1 つの連続領域として
まとめて動きます。ローダーは初期の `R30` に配置後の origin（`base_addr + delta`）を入れ、
位置独立な開始コードが image を見つけられるようにします。ローダーは最上位 block の上に
ヒープ用のゼロ化メモリを足してよい（`heap_hint` を参照）。それは image には含まれません。

## 4. 依存とシンボル

### 4.1 Dependency entry（16 byte）

u32 順に `library_id, abi_major, min_minor, reserved=0`。
library_id は string offset、major は非ゼロです。
配列の 0 始まり index を import から参照します。
同じ ID の重複、自分自身への依存は禁止。各依存は少なくとも 1 import で使用します。
WAX/WAL の両方が import でき、export は WAL のみです。

ユーザー命令を実行する前に graph 全体を解決します。
プロセス内で同じ ID は 1 instance、異なる major の同時要求は conflict。
minor 要求は最大値を取り、OS は全要求を満たす provider を選びます。
選択規則は文書化された決定的な規則とし、選択結果と版を記録します。
選択したファイル自身の image_id/abi_major/abi_minor が要求を満たすことを検証し、
ファイル名や catalog の一致だけで受理してはいけません。
全 image の call ABI と OS ID/major は一致し、OS の対応 minor は各 os_min_minor 以上。
循環・依存不足では起動全体が失敗します。
import の検索対象は指定依存自身の export だけで、推移的依存の export は検索しません。

### 4.2 Export entry（24 byte、WAL のみ）

u32 順に `name, kind, value_offset, size, signature, flags=0`。
name は string offset。ASCII 名の昇順で、kind が違っても同名重複は禁止。
kind は FUNCTION=1、OBJECT=2。

- FUNCTION: signature 必須（§6）。size は非ゼロの 4 の倍数、value_offset は 4-byte 整列。
  extent 全体が 1 つの初期化済み RX block 内に収まる必要があります。
  公開 entry/return は Normal mode。Short routine は extent 内に Normal wrapper を持ちます。
- OBJECT: signature=0、size は非ゼロ。全体が 1 つの R または RW block 内に収まります。
  BSS も公開でき、byte alignment を許可します。layout/constness は major 契約の一部です。

実アドレスは `origin + value_offset`。export は領域・権限を追加しません。
関数 extent の重複（alias）は許可します。

### 4.3 Import entry（24 byte、WAX/WAL）

u32 順に `dependency_index, name, kind, slot_offset, size, signature`。
index は依存配列の範囲内。指定依存の export と name/kind を完全一致で照合し、
不足・不一致なら拒否します。FUNCTION は size=0、signature 文字列の byte 列が一致必須。
OBJECT は signature=0、size は非ゼロで export size と一致必須。
object layout の互換性は提供者の major 契約で保証します。

slot_offset は image-relative、4-byte 整列。8 byte 全体が 1 つの RW block 内に
収まる必要があります。import は slot_offset 昇順で slot 重複は禁止。
同じ symbol を別 slot に import することは許可します。
slot の初期値は payload/BSS を問わず両 word とも 0 必須。
OS は big-endian の descriptor を書き込みます。

```
slot + 0: 解決した symbol の実アドレス (u32)
slot + 4: provider の origin B         (u32)
```

関数の第2 word は module context（§6）、object では所有 module を表します。
OS が修正するのは宣言済み slot だけで、命令は修正しません。
解決後も slot は user writable。結合は同一プロセス内の完全性境界ではなく、
WAL とアプリは同じアドレス空間・信頼境界に属します。

## 5. CPU 保護と 8 領域制限

[拡張 P](spec_CPU_priv.md) の **8 領域は CPU context 全体**の制限です。
image ごとに 8 領域あるわけではありません。領域 0 は WAX の RW スタック。
WAX と全 WAL の block を実アドレス順に並べ、同一 perm で完全に隣接するものだけを
併合します。hole を跨ぐ併合や権限 bit の追加は禁止。
結果を領域 1..N にアドレス順で設定し、N <= 7 必須、残りの register はすべて 0。
領域重複は禁止です。

配置を工夫しても収まらなければ REGION_LIMIT。S モード・RWX・重複 grant・
全 RAM 許可への fallback は禁止です。WAL 間の保護分離はありません。
予約範囲が許せば、別 image の同一 perm block を隣接配置して併合できます。
OS は CPU 予約領域の検査に加えて実際のメモリ所有権を検査します。
拡張 P は bus-master DMA を検査しないため、基本仕様では device mapping を与えません。

## 6. 公開呼出 ABI 1: SRC32 module stack ABI

これは**新しい公開 ABI**です。既存 sol/csc/`tools/llvm-src32` バイナリとの互換性を
意味しません。compiler の明示的 v3/PIC 対応または adapter が必要です。
callback も同じ規約に従います。

- R0=0、R28=SP、下向き、呼出境界で 8-byte 整列。
  引数は呼出時 SP から連続する 4-byte big-endian word で渡します。
  狭い整数は 32 bit に拡張、u64/i64 は上位 word、下位 word の順で 4-byte 整列。
  引数領域全体を 8 byte に切り上げ、caller が解放します。
  register 引数・暗黙の引数はありません。
- R29=FP。callee は R28/R29 を復元します。R30 は**現在の module origin**で、
  callee が呼出時の値を保存します。R1..R27/R31 は caller-saved。
  入れ子呼出をする側は戻り先 R31 を退避します。
- module 間呼出では caller の R30 を退避し、R30 を変える前に descriptor の両 word を
  scratch register に読みます。第2 word を R30 に設定して第1 word に JALR します。
  復帰後は結果を壊さず caller の R30 を復元します。
  callee は渡された R30 を保持し、内部呼出では R30 を変えません。
  PIC は R30+offset で local data/import にアクセスし、相対制御転送または
  R30 から計算した target を使います。コードのロード時再配置はありません。
- 32-bit/data pointer 戻り値は R1、64-bit は low=R1/high=R2、
  fnptr は address=R1/origin=R2、void は結果を無視。
  Normal mode の `JR R31` で戻ります。module 境界を渡る関数ポインターは
  生の命令アドレスではなく、**8-byte descriptor の値**です。
  callback でも address と context の両方を保持します。
- signature は `fn(` + 引数 type のカンマ区切り + `)->` + 結果 type。
  引数なしは `fn()->...`。type は `u32/i32/u64/i64/ptr/fnptr`、
  結果のみ `void` も許可。空白は禁止です。
  ptr は 32-bit data pointer、fnptr は `(address, origin)` の 2 word。
  float、値渡し aggregate、varargs は非対応。
  狭い整数 API は u32/i32、aggregate は明示 pointer と文書化した layout を使います。
  signature は機械的な呼出型であり、pointer の所有権・意味的安全性は保証しません。

R30 が module base なのはこの ABI の U mode 内だけです。
S bank はカーネルの GP/IRQ context のままです。
trap/context switch では CPU 仕様に従い、user context 全体と領域 register を保持します。
カーネルサービスは宣言した OS ABI の SYSCALL を使い、S mode address を import しません。
この形式自体は exit を含む syscall 番号を割り当てません。

WAX entry は `fn()->void`。Normal mode、USP=stack_top、UFP=0、UGP=base_addr、
R1..R27=0、R31=0 で開始します。entry から return してはいけません。
runtime が宣言した OS ABI で終了を要求します。
WAL に暗黙の entry はなく、必要な初期化関数は結合後に WAX runtime が呼びます。
起動引数・環境・exit・fault 処理は OS ABI が別途定義します。

## 7. OS の起動契約

1. 全 image の構造・文字列・symbol extent・slot 初期値・ABI を検証し、
   検証後のファイル変更がロードを変えないよう byte 列を snapshot します。
   file bytes/count/graph depth/総 resident memory 等の上限を文書化し、確保前に検査します。
2. graph/provider を解決し、固定 WAX・全 WAL span・stack の private RAM を予約します。
   全体の領域計画を検証します。
3. S mode で block/stack をコピー・ゼロ化し、全 import descriptor を結合します。
   staging と転送先の重複は禁止。この段階でライブラリコードを実行しません。
4. syscall/fault handler と有効な S stack/context を用意します。
   割り込みを mask した切替中に旧 grant を無効化し、新しい全 grant と user register を
   設定します。START/END を書いてから領域を有効にします。
   EPC=entry、STATUS.PU=1、PSM=0、PIE は OS 方針で設定し IRET。
   中間状態で別 context を実行してはいけません。
5. 事前のどの段階で失敗しても全予約を解放し、不完全なプロセスを起動しません。
   exit/fault では実行を停止し、grant を除去して graph 全体を解放します。
   実行可能な間に WAL を解放せず、別プロセスへの再利用前に領域をゼロ化します。

論理 error 分類（OS の数値 code とは別）:
FORMAT、UNSUPPORTED、DEPENDENCY_MISSING、ABI_MISMATCH、DEPENDENCY_CONFLICT、
DEPENDENCY_CYCLE、SYMBOL_MISSING、SYMBOL_MISMATCH、ADDRESS_CONFLICT、
REGION_LIMIT、RESOURCE_LIMIT。
診断には可能なら要求元 image・依存・symbol を含めます。
構造検証は署名検証・信頼性の認定ではありません。

## 8. 結合例と適合検証

APP.WAX: origin=`0x100000`、RX `[0x100000,0x100100)`、RW
`[0x100100,0x100180)`、entry_offset=0、stack `[0x1FF000,0x200000)`。
`cpt32.console` major=1/min_minor=2 を要求し、`write` を
FUNCTION、`fn(ptr,u32)->u32`、slot_offset=`0x100`、size=0 で import します。
CONSOLE.WAL は同 ID の 1.3、RX offset=0/size=`0x80`、RW offset=`0x80`/size=`0x40`、
write は offset=`0x20`/size=`0x10`、signature は同一。例の block はすべて初期化済みです。

B=`0x120000` なら `0x100100` に `00 12 00 20 00 12 00 00` を書きます。
領域は stack RW、APP RX、APP RW、CONSOLE RX、CONSOLE RW、残り 3 領域は無効。
write 呼出時は PC=`0x120020`、R30=`0x120000`、復帰後は R30=`0x100000` に戻します。

将来の実装で必須となる適合検証:

- 依存なし WAX、WAL から WAL への import、別 slot への同一 symbol import、
  file_size=0 の BSS、BSS object の export を受理する。
- 同 WAL を異なる整列済み B に配置し、local data・入れ子 import・callback が
  それぞれ正しい instance を参照する。
- BSS/stack のゼロ化、Normal entry、R30 復元、64-bit 順序、終了時解放、
  SYSCALL 前後の module context 保持を検証する。
- metadata/payload 重複、offset/count/address overflow、不正／文字列途中の offset、
  export 名重複、signature 構文不正、entry_size 不正、非ゼロ reserved/slot、entry 非整列を拒否。
- ID/symbol 不足、kind/signature/object size 不一致、call/OS/library ABI 不一致、
  major conflict、循環、自己依存・重複依存・未使用依存を拒否。
- RW 外／block を跨ぐ slot、不正 export extent、BSS entry、WX、stack/image 重複、
  WAL 整列違反、main RAM 外、stack 込み 9 領域を必要とする graph を拒否。
- stack 込みちょうど 8 領域は受理。同 perm の隣接のみ併合し、hole や異なる perm は併合しない。
- user の RX 書込・RW/stack 実行が拒否され、device/kernel grant がなく、
  拒否 image が S mode に fallback しないことを検証する。
- v2 reader は v3 を拒否。将来両版を扱う loader でも v2 に v3 symbol/ABI を暗黙付与しない。

これらのケースは Foundation のローダーテストで確認しています。形式・依存グラフ・
シンボル・領域・配置はホスト上（`foundation/tests/wax/test_wax_loader.py`）で、
実行、R30 の扱い、コールバック、64-bit の順序、SYSCALL、保護違反、2 つの基底での
個別インスタンス、終了時の後始末は src32_testbench 上（`foundation/tests/wax/main.c`）で
確認します。
