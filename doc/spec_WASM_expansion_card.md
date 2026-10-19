# WASM Expansion Card ABI v1 仕様

## 1. 目的と適用範囲

本書はCPT32 WASM拡張カードの規範ABI v1を定める。カードはWASM core moduleとして実行し、MMIO、仮想cycleイベント、IRQ level、既存DMACからのMMIO転送、およびmanifestで宣言した読み取り専用package assetを利用できる。WASI、ホストファイルアクセス、共有メモリ、任意のホスト関数、カード専用DMA APIは提供しない。

各カードは`.cpt32card` ZIP archiveとして配布する。archiveはrootの`manifest.yaml`、manifestが指定するWASM module、および任意のasset entryを含む。entryはZIP StoredまたはDeflate方式とし、entry単位でseek/decompressできる。起動時はmanifestとWASM moduleだけを展開し、assetはguestが初めて要求したentryだけを展開する。展開したassetはそのカードの実行期間中cacheする。1 packageは1 slot/1 WASM moduleに対応する。

manifest v1は次のYAML形式とする。manifestの展開後サイズは1 MiB以下。asset IDは0から始まる連続した整数で、guestから参照する識別子である。archive entry pathはpackage root相対の`/`区切りpathとし、absolute path、`.`/`..` component、重複path、directory entryは許可しない。manifest/module/assetはarchive内のregular fileでなければならない。assetのuncompressed size合計は64 MiB以下とする。

```yaml
abi_version: 1
module: "wasm/card.wasm"
asset:
  - id: 0
    path: "assets/font.bin"
```

実装はWasmtimeを実行engineとし、カードごとに独立したStore/Instanceを持つ。moduleのコンパイル、import/export検査、ABI version検査のいずれかに失敗した場合、そのカードを登録せず起動エラーとする。

## 2. Address mapとslot

拡張領域は`[0xA0000000, 0xB0000000)`の256 MiBで、8個の固定slotに分割する。slot `n`（`0 <= n < 8`）のbaseは`0xA0000000 + n * 0x02000000`、sizeは`0x02000000` byte（32 MiB）である。slotの窓は`[base, base + 0x02000000)`。slot 0は`0xA0000000–0xA1FFFFFF`、slot 7は`0xAE000000–0xAFFFFFFF`を占める。`0xB0000000`は拡張領域に含まれない。

空slotは未マップであり、既存の未定義アドレス動作に従う。カードは起動時に固定slotへ装着し、hotplugは行わない。

カードのMMIOは既存`Device`と同じbyte単位である。guestのMMIO offsetはslot内相対byte offsetで`0 <= offset < 0x02000000`。CPU/DMAの複数byteアクセスはバスのbyte access規則に従い、各byteが同じMMIO read/write exportを通る。

## 3. WASM module ABI

moduleは次の5つのfunction exportをすべて持たなければならない。関数名、parameter数・型、result数・型は完全一致しなければならない。

| Export | WASM function type | 契約 |
|---|---|---|
| `cpt32_abi_version` | `() -> i32` | ABI version `1`を返す。 |
| `cpt32_reset` | `() -> ()` | カード状態をresetする。 |
| `cpt32_mmio_read` | `(i32) -> i32` | slot内byte offsetを受け取り、byte値`0..255`を返す。 |
| `cpt32_mmio_write` | `(i32, i32) -> ()` | slot内byte offsetとbyte値`0..255`を受け取る。 |
| `cpt32_tick` | `() -> ()` | 予約deadline到達時に一度呼び出される。 |

moduleは次節に列挙したimport以外を持ってはならない。WASIを含む他のnamespace/import、shared memory、table/global import、未定義importは許可しない。独自DMA importはない。module start functionを含むすべてのguest実行にfuel制限を適用する。

### 3.1 許可host imports

import namespaceは`cpt32_host`に限り、次の6関数を許可する。関数は必要なものだけimportできる。名前と型は完全一致しなければならない。

| Import | WASM function type | 契約 |
|---|---|---|
| `current_cycles` | `() -> i64` | 現在の単調増加仮想cycle値を返す。戻り値の64-bit patternをunsigned `u64` cycle値として解釈する。 |
| `schedule_tick` | `(i64) -> ()` | 正値は呼出し時点の現在cycleからの相対delay。deadline到達時にhostが`cpt32_tick`を一度呼ぶ。0は既存予約を取り消す。負値はABI違反でありtrapする。カードごとに予約は最大1件で、新しい予約は以前の予約を置き換える。 |
| `irq_set` | `(i32) -> ()` | `0`でIRQ levelをdeassert、`1`でassertする。他の値はABI違反でありtrapする。 |
| `asset_count` | `() -> i32` | manifestで宣言したasset数を返す。 |
| `asset_size` | `(i32) -> i32` | asset IDのuncompressed byte sizeを返す。IDが存在しない場合はtrapする。 |
| `asset_read_byte` | `(i32, i32) -> i32` | asset IDとbyte offsetを受け取り、`0..255`のbyteを返す。IDまたはoffsetが範囲外の場合はtrapする。 |

asset importはpackageに含まれる宣言済みassetだけを公開し、guestにpathやfilesystem accessを与えない。asset ID/offsetの不正、またはasset sizeが`i32`で表現できない場合はABI違反としてtrapする。asset entryは初回read時にだけ展開する。

deadlineはカードごとに保持し、仮想時間を進める際はdeadlineを越える前に処理する。deadlineに達したとき、hostは予約を消費してから`cpt32_tick`を呼び出す。tick中の再予約は新しい予約として有効となる。reset/fault時は予約を消去する。

### 3.2 任意の音声出力export

ABI v1の後方互換拡張として `cpt32_audio_render: (i32 frames, i32 sample_rate) -> i32 pointer` を定義する。
exportしない既存カードは無音として扱う。exportする場合は同名関数の型を完全一致させ、通常のlinear memoryを `memory` としてexportする。型違い・memory欠落は起動エラー。

hostは `frames=1..256`、正の `sample_rate` (Hz) で呼び出す。現在のhostはfuelに余裕を持たせるため約1 ms、最大64 framesのblockに分ける。カードはhost rateのsigned 16-bit stereo PCMをlinear memoryに置き、その先頭offsetのu32 bit patternを返す。配置は `L0 low, L0 high, R0 low, R0 high, L1...`、計 `frames*4` bytes。CPT32 CPUのbig-endian MMIOとは別に、音声bufferはWASM標準のlittle-endianである。pointer `0` も有効。

hostはguest呼び出し後、範囲検証してからbufferを読み取り、次のguest呼び出しより前に消費する。範囲外pointer、trap、fuel枯渇は通常のcard faultにし、そのblockを加算しない。先に完了したblockの音声は残る。fault後のカードは無音。各blockに通常と同じ10,000,000 fuelの上限を適用する。

GUIの既存音声フレーム処理から、SGU/IDCの後にslot昇順で加算する。各加算をi16で飽和する。音声・MMIO・tickを直列に呼び、オーディオデバイスのcallback threadでWASMを実行しない。現在はフレーム末のレジスタ状態を使う方式で、フレーム内MMIO時刻をサンプル単位に再生する契約ではない。音声renderでBus cycleを進めず、仮想cycle tickを音声sampleから生成しない。host音声が無効なGUIやmonitor/testbenchは自動ではrenderしない。headless利用者は `Bus::mix_expansion_audio` を明示的に呼べる。

## 4. Reset、IRQ、DMA

card resetではhostが予約tickを取り消し、IRQ levelをlowにしてから`cpt32_reset`を呼び出す。Bus cycle値はresetによって巻き戻らない。各slot `n`のIRQ levelはIRQC source `5 + n`へ接続する。IRQCは立ち上がりedgeをpendingとして保持するため、カードIRQをdeassertしてもIRQC pendingは別途W1Cされるまで残る。

DMA専用host importは存在しない。guest softwareは既存DMACを設定し、装着カードのMMIO apertureを通常のDMA endpointとして利用する。beat全体が同じ装着slotの範囲内に収まる場合のみ許可する。slot境界を越えるbeat、空slot、fault状態のカードへの転送はDMA address errorとなる。BMC arbitrationおよび既存DMACの通常のRAM/VRAM/PCMRAM制約と既存VPU/IDC endpoint例外は維持する。

## 5. 実行制限とfault

各guest呼出しには10,000,000 fuelの上限を適用する。各module memoryの上限は64 MiBとする。初期memoryが上限を超えるmoduleは起動エラーとし、guestによる上限超過memory growthはtrapさせる。moduleのコンパイル・検査・startを含む初期化時にもguest実行fuelを適用する。

guest関数のtrapまたはfuel枯渇、`cpt32_mmio_read`の戻り値が`0..255`外、あるいはhost importへの規約外引数はカードfaultとする。ABI versionが1でない、必須exportの欠落/型違い、禁止import、コンパイル失敗、memory制限違反は起動時エラーであり、カードを登録しない。

fault発生時はカードをfault状態にし、以後guest関数を呼び出さない。hostは予約tickを取り消し、IRQをdeassertし、faultを一度診断出力する。faultカードへのMMIO readは`0`、writeは無視する。CPUがfaultカードへアクセスした場合はBus Errorも設定する。faultカードをDMA endpointとしてアクセスするとDMACの既存address error経路で失敗する。

## 6. 起動設定

設定ファイルの`expansion_card`配列にentryを記述する。各entryは整数`slot`と`.cpt32card` archive path文字列`package`を必須とする。未記載slotは空slot。空slot apertureへのbyte readは`0`を返し、bus errorを発生させないため、guest driverはoptional cardを安全にprobeできる。空slotへのwriteは通常のunmapped accessとしてbus errorを発生させる。`package`の相対pathは設定ファイルのparent directoryを基準に解決する。設定ファイルが存在しない場合、またはentryがない場合は拡張カードなしで起動する。

例:

```yaml
expansion_card:
  - slot: 0
    package: "plugins/timer.cpt32card"
  - slot: 7
    package: "plugins/storage.cpt32card"
```

slot範囲外、重複slot、型不正、package file不在、archive/manifest/module/ABI不正は起動エラーとする。診断にはslotとpackage pathを含める。複数entryを読み込む場合は全entryをcompile/validateしてからBusへ登録し、途中失敗時に部分登録してはならない。

