# DMA・IRQ・PeC・IDC チュートリアル

このページでは、CPU が DMAC に転送を依頼し、PeC 配下の IDC から 1 sector を RAM へ読む流れを説明します。IRQ は転送完了を通知する仕組みですが、最初の例は状態レジスタを polling して動作を追いやすくします。

レジスタの完全な定義は [BMC / DMAC 仕様](spec_BMC_DMAC.md)、[IDC 仕様](spec_IDC.md)、[システム仕様](spec.md)を参照してください。

## 1. 4つの用語と役割

- **DMA** は、CPU が byte ごとにコピーせずにデータを転送する方式です。このシステムでは DMAC が転送要求を出し、BMC が CPU・VPU・DMA channel のバス利用を調停します。
- **IRQ** は、デバイスが CPU にイベントを知らせる割り込みです。DMAC channel の完了/エラーは IRQ1、IDC の完了/エラーは IRQ2 を使います。
- **PeC** は周辺機器コントローラとその MMIO 領域の総称です。UART、乱数生成器、IDC などが含まれます。PeC 全体に共通する単一のデータ FIFO があるわけではなく、操作するレジスタは個々のデバイス仕様に従います。
- **IDC** は PeC 内の IDE/FDD/HDD コントローラです。raw image を 512-byte sector 単位で扱い、READ/WRITE のデータ本体は DMAC の `DATA` endpoint を通します。

## 2. 関連アドレス

| 領域 | アドレス | 使い方 |
|---|---:|---|
| IDC | `0x80041000` | PeC 内の IDC レジスタ |
| IDC `DATA` | `0x80041024` | IDC の 8-bit FIFO endpoint。READ/WRITE の DMA 転送に指定 |
| DMAC | `0x80050000` | 6 channel の DMA 設定と状態 |
| BMC | `0x80060000` | バス調停。通常は reset 値のままでよい |
| IRQC pending | `0xFFFF0040` | 16-bit IRQ pending mask |
| IRQC enable | `0xFFFF0044` | 16-bit IRQ enable mask |

PeC の UART / RNG は IDC とは別のデバイスです。IDC のレジスタ範囲は `0x80041000–0x80041FFF` です。

## 3. DMAC channel 0 の基本

DMAC は6 channel を持ち、channel `n` のレジスタは `0x80050100 + n × 0x20` から始まります。channel 0 の主なレジスタは次の通りです。

| アドレス | レジスタ | 意味 |
|---:|---|---|
| `0x80050004` | `GLOBAL_CONTROL` | bit 0 で DMAC 有効化 |
| `0x80050008` | `CHANNEL_ENABLE` | bit 0 で channel 0 を有効化 |
| `0x8005000C` | `IRQ_ENABLE` | bit 0 で channel 0 の完了/エラー IRQ を許可 |
| `0x80050010` | `IRQ_STATUS` | bit 0–5 は W1C。該当 bit に 1 を書くと pending を消去 |
| `0x80050100` | `CH0_SRC` | source address |
| `0x80050104` | `CH0_DST` | destination address |
| `0x80050108` | `CH0_COUNT` | 転送 byte 数 |
| `0x8005010C` | `CH0_CONTROL` | 幅、アドレス mode、START |
| `0x80050110` | `CH0_STATUS` | bit 0 BUSY、bit 1 DONE、bit 2 ERROR |
| `0x80050114` | `CH0_REMAIN` | 残り byte 数 |

`CH0_CONTROL` の bit 1:0 は幅（`00`=8-bit、`01`=16-bit、`10`=32-bit）、bit 3:2 は source mode、bit 5:4 は destination mode、bit 8 は START です。mode は `00`=increment、`01`=fixed です。IDC の `DATA` endpoint を使う場合は 8-bit 幅にします。

## 4. IDC の準備

| IDC アドレス | レジスタ | 意味 |
|---:|---|---|
| `0x80041004` | `CONTROL` | bit 0 で新規 command を許可。reset 値 1 |
| `0x80041008` | `STATUS` | bit 0 BUSY、bit 1 DONE (W1C)、bit 2 ERROR (W1C)、bit 3 DRQ |
| `0x8004100C` | `IRQ_ENABLE` | bit 0 completion、bit 1 error |
| `0x80041010` | `IRQ_STATUS` | bit 0 completion、bit 1 error pending (W1C) |
| `0x80041014` | `DRIVE_SELECT` | media slot 0–3 |
| `0x80041018` | `COMMAND` | 1 READ、2 WRITE、3 IDENTIFY、4 ABORT |
| `0x8004101C` | `LBA` | 開始 sector |
| `0x80041020` | `SECTOR_COUNT` | READ/WRITE の sector 数 |
| `0x80041024` | `DATA` | 8-bit FIFO endpoint。READ/WRITE では DMAC が使用 |
| `0x80041038` | `ERROR_CODE` | 直近の command error |

IDC READ/WRITE は有効な media、範囲内の LBA、`SECTOR_COUNT` を検査してから DMA を開始します。DMAC の byte count は `SECTOR_COUNT × 512` と完全一致させます。READ/WRITE 中に必要な設定は開始時にラッチされます。

## 5. 1 sector を IDC から RAM へ読む

以下は **slot 0 の LBA 0 を RAM `0x00010000` に読む** sol の例です。slot 0 には少なくとも 1 sector の raw image を接続してください。例の検証では image の先頭4 byte を `12 34 56 78` にし、読み込み後の RAM 値で確認します。

```sol
!const IDC 0x80041000
!const DMA 0x80050000
!const BUFFER 0x00010000
!const DATA 0x80041024

fn main () :
    # DMAC を有効にし、channel 0 を利用可能にする。
    1 DMA 0x004 add st
    1 DMA 0x008 add st

    # IDC READ: slot 0、LBA 0、1 sector。
    0 IDC 0x014 add st
    0 IDC 0x01C add st
    1 IDC 0x020 add st

    # IDC DATA は source 固定、RAM は destination increment。
    DATA DMA 0x100 add st
    BUFFER DMA 0x104 add st
    512 DMA 0x108 add st

    # IDC command を先に開始してから DMAC を開始する。
    1 IDC 0x018 add st
    0x104 DMA 0x10C add st # WIDTH=8-bit, SRC=fixed, START

    # DMA 完了後、IDC の sector 処理も完了するまで待つ。
    while
        IDC 0x008 add ld
        1 and 0 neq
    end

    IDC 0x008 add ld
    4 and 0 neq if
        1 ret # IDC error
    end

    BUFFER ld
    0x12345678 eq if
        0 ret # 読み込んだ先頭4 byte が一致
    end
    2 ret # 転送結果不一致
;

main
```

DMAC の IDC READ 設定は source=`DATA` fixed、destination=`BUFFER` increment、幅 8-bit、count 512 byte です。`CH0_CONTROL=0x104` は source fixed の bit 2 と START bit 8 を設定します。DMAC は IDC が sector を準備した byte だけを転送し、sector ready 前は bus beat を発行しません。

通常のメモリ間 DMA では source/destination の両方を RAM 等の許可領域に置き、アドレス増分と転送幅を揃えます。IDC `DATA` は唯一の追加 MMIO endpoint であり、8-bit 固定アドレス転送以外には使えません。

### WRITE に変える場合

WRITE は転送方向を反転します。DMAC source を RAM buffer に、destination を IDC `DATA` にします。`CH0_CONTROL` は WIDTH=8-bit、source increment、destination fixed、START を持つ `0x110` です。IDC 側では `COMMAND=2` を書き、byte count は引き続き `SECTOR_COUNT × 512` にします。接続 media が read-only でないことを確認してください。完全に受信し、sector の transfer time が到来した sector だけが raw image に commit されます。

## 6. IRQ を使う

Polling の代わりに IRQ を使うには、対象デバイスと IRQC の両方を設定します。

1. IDC completion IRQ は `IDC_IRQ_ENABLE.bit0`、error IRQ は bit 1 で許可します。DMAC IRQ は `DMAC_IRQ_ENABLE` の該当 channel bit で許可します。
2. IRQC の mask で source 1 (DMAC) と source 2 (IDC) を有効にします。たとえば IRQ1/IRQ2 を有効にする mask は `0x0006` です。
3. CPU 側の IRQ enable も有効にし、IRQ ごとの vector `0xFFFF0100 + 4 × n` に対応する handler で `IRQC_PENDING & IRQC_ENABLE` を確認します。IRQC は番号の小さい source を優先します。
4. 割り込みを処理したら、まず原因デバイスの pending を W1C で消し、その後に IRQC pending を W1C で消します。DMAC は `0x80050010` の channel bit、IDC は `0x80041010` の completion/error bit を消します。IRQC pending は `0xFFFF0040` の source bit を消します。
5. アセンブリ handler は `IRET` で復帰します。sol の `irq1`〜`irq15` handler は `retn` で戻り、sol の dispatcher が `IRET` を実行します。

デバイス pending と IRQC pending は別状態です。IRQ2 では IDC `IRQ_STATUS` の該当 bit を消し、IRQC pending bit 2 も消してください。IRQ1 は channel ごとの DMAC status と IRQ pending を確認してください。`DONE` / `ERROR` status bit の clear と IRQ pending の clear も独立しています。DMA/IDC のデバイス IRQ enable が 0 なら、そのデバイスの完了・エラーは IRQC に通知されません。

IRQ vector、CPU の割り込み enable、例外復帰の詳細は [CPU 仕様](spec_CPU.md) を参照してください。sol の handler 宣言と IRQ 番号取得 helper は [sol 仕様](spec_sol.md) と [`irqc.sol`](../sol/cpt32/irqc.sol) を参照してください。handler は割り込みを acknowledge してから復帰します。

## 7. 実行時設定とエラー確認

IDC は起動時の `config.toml` の `[[disk]]` から raw image を slot に割り当てます。例:

```toml
[[disk]]
slot = 0
type = "hdd"
image = "disk0.img"
read_only = true
transfer_rate_bytes_per_second = 48000000
access_latency_ms = 0
```

`disk0.img` は 512 byte 以上で、全体サイズが 512 の倍数の raw data です。相対 path は `config.toml` のある directory を基準にします。実媒体を書き換えない練習では `read_only = true` にします。

`idc_read.sol` を `$work` に保存し、image の先頭4 byte が `12 34 56 78` になるように `disk0.img` を用意します。次の例はリポジトリ root から実行します。作業用 directory に `idc_read.sol`、`disk0.img`、上記の `config.toml` を置くため、既存の `config.toml` は変更しません。

```powershell
$repo = (Get-Location).Path
$work = Join-Path $env:TEMP "src32-idc-tutorial"
New-Item -ItemType Directory -Force $work | Out-Null
py tools/solc/solc.py compile "$work\idc_read.sol" -o "$work\idc_read.a"
py tools/asm/asm.py "$work\idc_read.a" -o "$work\idc_read.bin"
Push-Location $work
cargo run --manifest-path "$repo\Cargo.toml" --bin src32_testbench -- idc_read.bin
Pop-Location
```

成功時は `PASS: idc_read.bin => R1=0` が出ます。`R1=1` は IDC error、`R1=2` は読み込んだ先頭4 byte の不一致です。

エラー時は IDC `STATUS.ERROR` と `ERROR_CODE`、DMAC `CH0_STATUS.ERROR` と `ERROR_CODE` を確認します。IDC の代表的なコードは 1=設定不正、2=media なし、3=LBA 範囲外、4=write protect、5=DMA mismatch/abort、6=media I/O failure です。DMAC の ERROR_CODE=1 は設定不正、2 は禁止 address / bus error、3 は abort です。DMA abort は beat 境界、IDC ABORT は sector 境界で停止するため、すでに完了した書き込み sector は保持されます。

## 関連資料

- [BMC / DMAC 仕様](spec_BMC_DMAC.md)
- [IDC 仕様](spec_IDC.md)
- [PeC 仕様](spec_PeC.md)
- [システム仕様](spec.md)
- [CPU 仕様](spec_CPU.md)
