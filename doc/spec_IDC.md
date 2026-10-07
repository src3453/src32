# IDC (IDE Disk Controller) 仕様

## 1. 概要

IDCはPeCの仮想FDD/HDDコントローラである。最大4台のrawディスクイメージをslot 0～3に割り当て、512-byte sector単位のREAD/WRITE、媒体情報の取得、DMAC転送、完了/エラー割り込みを提供する。本仕様はハードウェア/エミュレータの外部契約を定める。Rust実装と実行時設定読込みの実装詳細は定めない。

## 2. アドレスとアクセス

IDCはPeC MMIO範囲内の`0x80041000–0x80041FFF`に配置する。base=`0x80041000`、size=`0x1000`。レジスタ値は32-bit big-endian、offsetは4-byte alignedで、通常レジスタは32-bitアクセスとする。`DATA`だけは8-bitアクセス専用FIFO endpointである。定義外offsetと未定義bitは0読み/書込み無視。`DATA`への8-bit以外のアクセスは受理しない。

| Offset | Register | Access / definition |
|---:|---|---|
| `0x0000` | ID | R=`0x49444301` |
| `0x0004` | CONTROL | RW, bit 0 enable, reset=1 |
| `0x0008` | STATUS | R/W1C: bit 0 BUSY live RO, bit 1 DONE W1C, bit 2 ERROR W1C, bit 3 DRQ live RO |
| `0x000C` | IRQ_ENABLE | RW: bit 0 completion, bit 1 error |
| `0x0010` | IRQ_STATUS | RW1C: bit 0 completion pending, bit 1 error pending |
| `0x0014` | DRIVE_SELECT | RW: selected slot 0–3 |
| `0x0018` | COMMAND | WO, self-clearing: 1 READ, 2 WRITE, 3 IDENTIFY, 4 ABORT |
| `0x001C` | LBA | RW, 32-bit logical block address |
| `0x0020` | SECTOR_COUNT | RW, READ/WRITE count 1–8,388,607 |
| `0x0024` | DATA | R/W, 8-bit FIFO endpoint at `0x80041024` |
| `0x0028` | DRIVE_TYPE | R: 0 absent, 1 FDD, 2 HDD |
| `0x002C` | SECTOR_SIZE | R: 0 absent, otherwise 512 |
| `0x0030` | CAPACITY_SECTORS | R, 32-bit sector count |
| `0x0034` | DRIVE_FLAGS | R: bit 0 present, bit 1 read-only |
| `0x0038` | ERROR_CODE | R |

Reset sets every register except ID to 0, then sets CONTROL.bit0=1. BUSY, DONE, ERROR, IRQ_ENABLE, IRQ_STATUS and FIFO are clear/empty. DRIVE_SELECT, LBA and SECTOR_COUNT reset to 0. STATUS.BUSY and STATUS.DRQ are live read-only state; writes to those bits have no effect. STATUS.DONE and STATUS.ERROR are sticky W1C bits. ERROR_CODE is cleared to 0 when the next command is accepted.

Error codes: 0 none, 1 invalid command/configuration, 2 no media, 3 LBA range, 4 write-protect, 5 DMA mismatch/abort, 6 media I/O failure.

## 3. 媒体とスロット

slot 0～3はそれぞれ未接続、またはFDD/HDDを1台だけ持つ。イメージは拡張子で判別しないraw byte列であり、ヘッダやコンテナ形式を持たない。sector sizeは常に512 bytes、capacityは`image_length / 512` sectors。イメージは空でなく、長さが512の倍数であり、sector countが32-bit unsigned値に収まらなければならない。これらに反する媒体は起動時構成エラーである。

LBAは32-bit unsignedで、sector範囲は`[LBA, LBA + SECTOR_COUNT)`。加算はオーバーフローしない幅で検査し、範囲末尾がcapacity以下の場合だけ有効とする。READ/WRITE全体のcountとLBA範囲を転送開始前に検査する。無効範囲では媒体もRAMも変更せず、ERROR_CODE=3で終了する。SECTOR_COUNTの最大値8,388,607は`count × 512`が32-bit DMAC byte countに収まる上限である。

DRIVE_SELECTが0～3以外ならコマンドはERROR_CODE=1で終了する。CONTROL.enable=0は新規コマンドの受理を禁止するが、進行中コマンドを停止しない。無効中の開始要求は新しい転送を開始しない。

## 4. コマンドと状態遷移

同時に実行できるIDCコマンドは1つだけである。受理時に選択slotを検査する。BUSY中の新規コマンド、DRIVE_SELECT/LBA/SECTOR_COUNTへの書込みは無視し、進行中のラッチ値と状態を変えない。未定義commandはERROR_CODE=1で終了する。

- **IDENTIFY (3):** 選択slotのDRIVE_TYPE、SECTOR_SIZE、CAPACITY_SECTORS、DRIVE_FLAGSを即時更新して返す。未接続slotなら情報レジスタをabsent値（type/size/capacity/flagsを0）に更新し、ERROR_CODE=2で終了する。成功は通常完了としてDONEを立てる。
- **READ (1) / WRITE (2):** 選択slot、開始LBA、sector countを受理時にラッチし、媒体の存在、count（1～8,388,607）、全LBA範囲、read-only条件を検査する。未接続媒体はERROR_CODE=2、範囲外LBAはERROR_CODE=3、無効countはERROR_CODE=1で終了する。READ/WRITEには一致するDMAC channelが必要である。WRITE_PROTECTではイメージを書き換えず、ERROR_CODE=4で終了する。検査に成功した後だけDMA転送を開始する。
- **ABORT (4):** 実行中DMAをsector境界で中止する。すでに完了・commitしたsectorは保持し、staging中のsectorは破棄してERROR_CODE=5で終了する。idle時のABORTは状態を変更しない。

正常なIDENTIFY/READ/WRITEの完了ではBUSYを下げ、DONEを立てる。IRQ_ENABLE.bit0が1ならIRQ_STATUS.bit0にcompletion pendingを立てる。エラー完了ではBUSYを下げ、ERRORを立てる。IRQ_ENABLE.bit1が1ならIRQ_STATUS.bit1にerror pendingを立てる。IRQ_STATUSのpending maskとIRQ_ENABLEのANDが非zeroの間、PeCのIDC IRQ source 2をassertする。IRQ_STATUSは対応bitへの1書込みで個別にclearする。DONE/ERRORのclearとIRQ pendingのclearは独立している。

## 5. DMAとsector staging

READ/WRITEはDMACの固定アドレスsource/destination handshakeで行う。唯一許可するIDC peripheral endpointは`DATA`の完全一致アドレス`0x80041024`であり、幅は8-bitに限る。

| IDC operation | DMAC source | DMAC destination | Transfer width |
|---|---|---|---|
| READ | `0x80041024`, fixed | RAM, increment | 8-bit |
| WRITE | RAM, increment | `0x80041024`, fixed | 8-bit |

DMAC byte countはIDCの`SECTOR_COUNT × 512`と完全一致しなければならない。READ/WRITEがactiveでない状態、方向・幅・byte count・endpoint mode/addressの不一致、または他channelが同時に同じendpointを使用する設定はDMAC設定エラー（DMAC `ERROR_CODE=1`）として開始拒否する。不一致したIDC READ/WRITEはERROR_CODE=5で終了する。DATA以外のMMIO endpointは不許可のままである。DMAC abortまたは実行中DMA失敗はIDC ERROR_CODE=5に、媒体I/O失敗はIDC ERROR_CODE=6に反映し、IDC側にERROR状態とerror IRQ pendingを記録する。
IDCは512-byte sector staging bufferを使う。READでは各sectorのtransfer time到来後にそのsectorのDATA byteをDMACがread可能にする。WRITEではDMAから受信した完全な512-byte sectorだけを、同sectorのtransfer time到来後にraw imageへcommitする。次sectorのstagingはcommit後に開始し、commit待ちのsectorでbufferを上書きしない。DRQ=1はDATA byteがread可能、またはDATA byteを受信可能な期間だけである。sectorがreadyになる前はDMAC向けIDC requestを出さず、対応するbus beatも開始しない。

DMAC abort、DMA失敗、またはmedia I/O errorではcommit済みの完全sectorを保持し、staging中の不完全sectorは破棄する。READ中のmedia I/O errorではRAMに転送済みのprefixを保持し、残りの転送を停止する。READ/WRITE全体の事前LBA検査に失敗した場合はDMAを始めず、RAM/mediaのいずれも変更しない。

## 6. 転送速度とサイクル

各slotは`transfer_rate_bytes_per_second`と`access_latency_ms`を持つ。型別既定値はFDDが62,500 B/sおよび100 ms、HDDが5,000,000 B/sおよび10 ms。遅延はコマンドにつき一度だけ加算し、転送byte数は`SECTOR_COUNT × 512`である。

master clockは48 MHz。コマンド開始から累積転送済みbyte数`n`のsectorが完了する時刻は、次のcycle数とする。

`ceil(access_latency_ms × 48,000) + ceil(n × 48,000,000 / transfer_rate_bytes_per_second)`

ここで`n`はsector境界の累積byte数（512, 1024, …）である。完了はこの絶対deadline以前に発生せず、整数cycleで決定論的に進行するため、指定rateを超えない。例: FDD既定値で1 sectorは`4,800,000 + 393,216 = 5,193,216` cycles、すなわち108.192 ms。設定値を上書きした場合も同じ式を適用する。

## 7. 起動時コンフィグ

起動時にカレントディレクトリの`config.toml`を読む。ファイルがない場合、または`[[disk]]`配列/sectionが省略もしくは空の場合は全slot未接続で起動する。各`[[disk]]`要素は次の項目を持つ。

| Key | Required | Definition |
|---|---|---|
| `slot` | yes | 整数0～3。一つのslotは一度だけ指定可能 |
| `type` | yes | `"fdd"`または`"hdd"` |
| `image` | yes | raw image path |
| `read_only` | no | boolean、default `false` |
| `transfer_rate_bytes_per_second` | no | 正整数。省略時はtype別既定値 |
| `access_latency_ms` | no | 非負整数。省略時はtype別既定値 |

相対image pathはconfig fileの親directoryを基準とする。絶対pathも許可する。rawへの書込みはimage fileに反映する。既存configの構文/値エラー、重複または範囲外slot、不明type、存在しない/読めないimage、不正な長さ、rate=0、負の遅延は起動時エラーとし、対象slotと原因を示す。

```toml
[[disk]]
slot = 0
type = "fdd"
image = "disk0.img"
read_only = false

[[disk]]
slot = 1
type = "hdd"
image = "images/data.bin"
read_only = true
transfer_rate_bytes_per_second = 8000000
access_latency_ms = 4
```

上例はslot 0をFDD既定速度で読書き可能、slot 1をHDD型のread-only媒体として8,000,000 B/s・4 msで割り当てる。`disk0.img`と`images/data.bin`はいずれも拡張子によらずraw byte列として扱う。

## 8. 契約例

2-sector (1024-byte) raw imageをslot 0に接続した場合:

1. `DRIVE_SELECT=0`, `COMMAND=IDENTIFY`でFDD、512-byte sector、capacity=2、presentを即時取得する。
2. `LBA=1`, `SECTOR_COUNT=1`, `COMMAND=READ`は、正しいREAD向きDMAC設定（DATA fixed source、RAM increment destination、8-bit、512 bytes）でイメージ末尾512 bytesをRAMへ転送する。正常完了時にDONEを立て、completion IRQ有効時はIRQ source 2をassertする。
3. `LBA=1`, `SECTOR_COUNT=2`のREADは全範囲検査でERROR_CODE=3となり、DMA開始前に停止するためRAM/mediaを変更しない。
4. `LBA=0`, `SECTOR_COUNT=1`のWRITEはRAMの512 bytesをraw image先頭へ反映する。read-only時はERROR_CODE=4となり、image内容を保持する。
