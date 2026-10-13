# PeC (Peripheral Controller)

## 概要
PeCは、ペリフェラルデバイスの管理とイベントバッファリングを担当する周辺機器コントローラです。キーボード、マウス、ゲームパッド、FDD/HDDインターフェース、UARTなどのデバイスをサポートし、FIFOバッファとDMA転送に対応しています。

## 機能
- ペリフェラルデバイス管理
- キーボード、マウス、ゲームパッドのサポート
- FDD/HDDインターフェースのサポート
- 乱数生成器 (PRNG/TRNG) のサポート
- UART通信のサポート
- イベントバッファリング
- DMA転送対応
- IRQCとの連携による割り込み管理
- 48 MHz クアッドハードウェアタイマ
- RTC (Real-Time Clock)

## アドレスマップ

アドレス範囲: 0x80040000 ~ 0x8004FFFF (64KB)

| アドレス範囲 | 機能 |
|---------------|------|
| 0x80040000 ~ 0x8004000F | UARTコントロールレジスタ |
| 0x80040010 ~ 0x8004002F | 乱数生成器 (PRNG/TRNG) |
| 0x80040030 ~ 0x8004017F | キーボード・マウス・ゲームパッド入力 |
| 0x80040180 ~ 0x8004018F | クアッドタイマ共通レジスタ |
| 0x80040200 ~ 0x80040223 | RTC |
| 0x80040400 ~ 0x800404FF | クアッドタイマ4チャネル |


### キーボード・マウス・ゲームパッド入力

入力MMIOは `0x80040030` から `0x8004017F` までです。レジスタオフセットはPeC基準の絶対オフセットです。UARTおよび乱数生成器の予約領域は変更しません。

| アドレス | レジスタ | 説明 | 属性 |
|----------|----------|------|------|
| `0x80040030` | EVENT_DATA | FIFO先頭の32ビットイベント。読み出しでは消費しない。空なら0。 | R |
| `0x80040034` | EVENT_STATUS | bit 0: FIFO非空、bit 1: FIFO overflow。bit 1をLSBへ書くとoverflowをクリア (W1C)。 | R/W |
| `0x80040038` | EVENT_POP | big-endianで1を書き込むとFIFO先頭を1件消費。 | W |
| `0x80040040` | MOUSE_X | ゲスト画面上のX座標 (0–639)。 | R |
| `0x80040044` | MOUSE_Y | ゲスト画面上のY座標 (0–479)。 | R |
| `0x80040048` | MOUSE_DX | 最新のX移動量 (符号付き32ビット、ゲスト画素)。 | R |
| `0x8004004C` | MOUSE_DY | 最新のY移動量 (符号付き32ビット、ゲスト画素)。 | R |
| `0x80040050` | MOUSE_WHEEL | 最新の縦ホイール量 (符号付き32ビット、120単位/lineまたは100物理pixel)。 | R |
| `0x80040054` | MOUSE_BUTTONS | bit 0–4: 左、右、中、X1、X2。 | R |
| `0x80040080–0x8004017F` | PAD0–PAD3 | 4スロットのゲームパッド状態 (各64バイト)。 | R |

EVENT_DATAは `[31:28 type, 27:24 device, 23:16 code, 15:0 value]` です。typeは0=KEY、1=MOUSE_BUTTON、2=MOUSE_MOVE、3=MOUSE_WHEEL、4=PAD_BUTTON、5=PAD_AXIS。deviceは0=キーボード、1=マウス、2–5=ゲームパッドslot 0–3です。KEYのcodeはUSB HID keyboard usage (page 0x07)、valueは押下=1/解放=0です。マウス移動・ホイールのcode 0/1はX/縦または横、valueは符号付き16ビットです。PAD_BUTTONのcodeはボタンbit番号、valueは押下=1/解放=0です。PAD_AXISのcode 0–3は左X/Y・右X/Y、4/5はLT/RTで、stick値は符号付き16ビット、trigger値は符号なし16ビットです。Xは右向き、Yは下向きが正です。

各ゲームパッドslotのレジスタは `0x80040080 + slot * 0x40` を基準にします。`+0 STATUS` (bit 0 connected、bit 1 digital、bit 2 analog)、`+4 BUTTONS` (A, B, X, Y, 上/下/左/右, Select, Start, LB, RB, 左/右stick clickがbit 0–13)、`+8/+0x0C LEFT_X/Y`、`+0x10/+0x14 RIGHT_X/Y` (符号付き16ビット値の符号拡張)、`+0x18 TRIGGERS` (LT bits 15:0、RT bits 31:16)、`+0x1C ID` (接続中はslot+1)、`+0x20 CAPS` (接続中は標準button/stick/triggerを示すbit 0–2)。未定義位置は0です。

FIFO容量は256件です。満杯時は新規イベントを破棄してoverflowを設定しますが、対応する入力状態は更新します。FIFO非空はIRQC source 3をlevel assertionし、FIFOが空になると解除します。イベント読出しはpeekであり、EVENT_POPで明示的に消費します。

ウィンドウ入力ではF1でマウスcaptureを切り替えます。capture中は相対移動を受け取り、絶対座標は最後のuncaptured位置を維持します。画面外の座標はゲスト画面端へclampします。ImGuiがマウスをcaptureしている間はゲストへマウスイベントを送りません。

## 48 MHz クアッドハードウェアタイマ

PeCタイマはシステムクロック `MASTER_CLOCK = 48,000,000 Hz` を基準にします。CPU命令数ではなく、CPUが消費したシステムクロック数で進みます。各チャネルは独立した64-bitアップカウンタを持ち、`PRESCALER = P` のときカウンタ周波数は `48,000,000 / (P + 1)` Hzです。エミュレーターはstepごとに消費クロック数をデバイスへ渡し、HALT中の時間スキップでは次のタイマ／RTC IRQ期限まで進めます。

入力レジスタが `0x100–0x17F` を使うため、タイマ共通レジスタは提案位置の `0x100` ではなく `0x180` に配置します。PeC基準オフセットは次のとおりです。

| オフセット | レジスタ | 説明 |
|---|---|---|
| `0x180` | TIMER_ID | 固定値 `0x51540001` (quad timer, version 1) |
| `0x184` | TIMER_ENABLE | bit 0–3: チャネルのグローバル実行許可。各チャネルCTRLのENABLEも必要。 |
| `0x188` | TIMER_PENDING | bit 0–3: チャネル比較一致。IRQ enableに関係なくsticky。 |
| `0x18C` | TIMER_ACK | bit 0–3へ1を書くと対応するTIMER_PENDINGをW1C。 |

Timer 0–3のチャネル基準オフセットは `0x400 + channel * 0x40` (`0x400`, `0x440`, `0x480`, `0x4C0`) です。各レジスタは32-bit big-endian MMIOです。

| 相対オフセット | レジスタ | 説明 |
|---|---|---|
| `0x00` | CTRL | bits 1:0 mode: 0=one-shot、1=periodic、2=free-running、3=予約。bit 2 ENABLE、bit 3 IRQ_ENABLE、bit 4 RESET (write-one)。 |
| `0x04` | STATUS | bit 0: channel ENABLE、bit 1: sticky compare pending。 |
| `0x08/0x0C` | COUNT_LO/HI | 64-bit counter。両ワード読書き可能。LOを読むとHIをラッチし、続くHI読出しに返す。 |
| `0x10/0x14` | COMPARE_LO/HI | 64-bit比較期限。 |
| `0x18/0x1C` | RELOAD_LO/HI | 64-bit周期間隔。 |
| `0x20` | PRESCALER | 32-bit `P`。変更時に分周位相を0へ戻す。 |

One-shotはカウンタがCOMPAREに達するとその値で停止します。PeriodicはCOMPAREを最初の絶対期限として使用し、期限ごとにRELOADをCOMPAREへ加算します。カウンタはリセットせず、遅延したエミュレーターstepでも期限系列の位相を保ちます。RELOADが0なら一致後に停止します。Free-runningは64-bitでラップし、COMPARE一致で一度pendingを立てます。比較値を書き直すと一致検出を再armします。RESETはカウンタ、分周位相、pendingをクリアします。比較／リロード値を書き終えてからCTRLのENABLEを設定してください。

IRQ4は、IRQ有効なタイマチャネルまたはRTCアラームの共有sourceです。デバイス側のACKでpendingを消した後、IRQCのedge-latched pending bit 4もIRQCへACKしてください。複数のタイマ／RTC要因は個別のPeC pending/statusで判別します。

## RTC

RTCはUTC Unix epoch秒 (`1970-01-01 00:00:00 UTC`) を64-bitで保持し、サブ秒レジスタは0–999,999,999 nanosecondsです。初期値は仮想時間の0秒で、RTCは有効です。カレンダー・timezone変換はソフトウェア側で行います。

| オフセット | レジスタ | 説明 |
|---|---|---|
| `0x200` | RTC_CTRL | bit 0 ENABLE、bit 1 SET_ENABLE、bit 2 REALTIME。初期値1。 |
| `0x204` | RTC_STATUS | bit 0 sticky alarm pending、bit 1 alarm armed。 |
| `0x208/0x20C` | RTC_SECONDS_LO/HI | 64-bit Unix秒。LO読出しでHIをラッチ。SET_ENABLE中に両ワードを書き終えると設定をcommitし、サブ秒を0にする。 |
| `0x210` | RTC_SUBSEC | 現在のサブ秒 nanoseconds (read-only)。 |
| `0x214/0x218` | RTC_ALARM_LO/HI | 64-bit Unix秒のone-shotアラーム。両ワードの書込み完了でarmし、過去／現在の時刻なら直ちに発火。 |
| `0x21C` | RTC_IRQ_CTRL | bit 0: アラームIRQ enable。 |
| `0x220` | RTC_ACK | bit 0へ1を書くとalarm pendingをW1C。再armにはALARMを書き直す。 |

REALTIME=0では48 MHzシステムクロックに同期する決定的な仮想時刻です。REALTIME=1ではホストUTCを参照し、RTC_SECONDS書込みはホスト時計そのものではなくRTCのoffsetを設定します。時間源を切り替えると現在値を引き継ぎます。ENABLE=0では時刻を停止・保持します。RTC IRQもタイマと同じIRQC source 4へ集約します。

## UART仕様
通信方式: 全二重
バス幅: 8ビット
バッファサイズ: TXD, RXDともに16バイト (FIFO)
ボーレート: 115200 bps (デフォルト)
ハードウェアフロー制御: なし

## UARTコントロールレジスタ
| アドレス | レジスタ名 | 説明 | 属性 |
|----------|-------|------|--|
| 0x80040000 | DATA_RW | UARTデータの読み書き | RW |
| 0x80040001 | STATUS | UARTの状態フラグ | R |
| 0x80040001.0 | TX_READY | 送信バッファが空かどうかを示すフラグ | R |
| 0x80040001.1 | RX_READY | 受信バッファにデータがあるかどうかを示すフラグ | R |
| 0x80040002 | BAUD_RATE | UARTのボーレート設定 | RW |
| 0x80040003 | CONTROL | UARTの制御レジスタ (有効化、割り込み設定など) | RW |
| 0x80040003.0 | ~ENABLE | UARTの有効化フラグ(0で有効) | RW |
| 0x80040003.1 | INT_ENABLE | UART割り込みの有効化フラグ | RW |
| 0x80040004 | RX_BYTES | 受信バッファにあるバイト数 | R |
| 0x80040005 | TX_BYTES | 送信バッファにあるバイト数 | R |

**備考:** UARTはエミュレータでは標準入出力に接続され、ホストOSのコンソールと連携します。また、ボーレートやその他の設定は無視されます。(必要に応じて速度シミュレーションを追加)

## 乱数生成器仕様
乱数生成器は、擬似乱数生成器 (PRNG) と真の乱数生成器 (TRNG) の両方をサポートします。PRNGはソフトウェアでシード値を設定して使用することができ、TRNGはハードウェアのノイズを利用してランダムな値を生成します。
PRNGはソフトウェアでシード値を設定して使用することができ、TRNGはハードウェアのノイズを利用してランダムな値を生成します。
PRNGはGFSRを使用しており、32ビットのシード値を設定することで、擬似乱数列を生成します。
**備考:** PRNGのデフォルトシード値は0x55555555です。
**備考:** TRNGはエミュレータではOSの乱数生成器に接続され、ホストOSの乱数生成器と連携します。


## 乱数生成器レジスタ
| アドレス | レジスタ名 | 説明 | 属性 |
|----------|-------|------|--|
| 0x80040010 ~ 0x80040013 | PRNG_DATA | 疑似乱数生成器の出力データ (32ビット) | R |
| 0x80040014 ~ 0x80040017 | PRNG_SEED | 疑似乱数生成器のシード値設定 (32ビット) | RW |
| 0x80040018 ~ 0x8004001B | TRNG_DATA | 真の乱数生成器の出力データ (32ビット) | R |
| 0x8004001C | TRNG_STATUS | 真の乱数生成器の状態フラグ | R |
| 0x8004001C.0 | TRNG_READY | 真の乱数生成器がデータを生成可能かどうかを示すフラグ (エントロピーの状態) | R |
| 0x8004001D | TRNG_CONTROL | 真の乱数生成器の制御レジスタ (有効化、割り込み設定など) | RW |
| 0x8004001D.0 | ~TRNG_ENABLE | 真の乱数生成器の有効化フラグ(0で有効) | RW |
