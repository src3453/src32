# IRQC (Interrupt Request Controller) 仕様

## 1. 目的と範囲

IRQCは、16個の外部IRQ sourceを受け取り、pending状態を保持し、有効な要求から一つを固定優先度で選択してCPUへ通知するデバイスです。IRQベクタ、CPUの割り込み許可、例外復帰はIRQCではなくCPUが管理します。CPU側の割り込み入口と`IRET`は[CPU仕様](spec_CPU.md#46-extension-i-interrupts)を参照してください。

IRQCはデバイス固有の状態を保持しません。割り込み処理では、原因デバイスの状態とIRQCのpending bitをそれぞれ確認・ACKします。

## 2. MMIOアドレス

| 項目 | 値 |
|---|---|
| ベースアドレス | `0xFFFF0040` |
| マップサイズ | `0x10` bytes |
| バス形式 | big-endian |
| source数 | 16 (`IRQ0`–`IRQ15`) |
| 初期値 | PENDING=`0x0000`、ENABLE=`0x0000`、全source level low |

IRQCはCPU予約領域内に配置されます。アドレスマップ上の範囲は`0xFFFF0040–0xFFFF004F`です。

## 3. レジスタ

| IRQCオフセット | 絶対アドレス | レジスタ | 属性 | 説明 |
|---:|---:|---|---|---|
| `0x00–0x01` | `0xFFFF0040–0xFFFF0041` | PENDING | R/W1C | 16-bit pending mask。bit `n`はIRQ`n`。1を書いたbitをクリアします。 |
| `0x02–0x03` | `0xFFFF0042–0xFFFF0043` | Reserved | R/— | 読み出しは0、書き込みは無視します。 |
| `0x04–0x05` | `0xFFFF0044–0xFFFF0045` | ENABLE | R/W | 16-bit interrupt mask。bit `n`が1ならIRQ`n`をCPU出力の選択対象にします。 |
| `0x06–0x0F` | `0xFFFF0046–0xFFFF004F` | Reserved | R/— | 読み出しは0、書き込みは無視します。 |

PENDINGとENABLEは16-bit big-endian値です。`LDH`/`STH`でそれぞれ`0xFFFF0040`、`0xFFFF0044`からアクセスしてください。バイトアクセスでは各レジスタの先頭アドレスが上位8-bit、次のアドレスが下位8-bitです。

このデバイスは32-bitワードの下位2 byteをレジスタとして実装していません。したがって、32-bit read at `0xFFFF0040` は`PENDING << 16`、32-bit read at `0xFFFF0044` は`ENABLE << 16`になります。32-bit storeは16-bitレジスタへのhalfword storeの代用になりません。特にPENDINGのW1Cにはhalfwordまたは各byteへの書き込みを使います。

### 3.1 PENDING (W1C)

- 読み出しは全16 sourceのpending状態を返します。読み出しによるクリアはありません。
- 書き込んだ値の1のbitだけをクリアします。0のbitは変化しません。
- W1CはIRQC側の状態のみをクリアします。原因デバイスのpending/statusは別途そのデバイスの仕様に従ってACKしてください。

### 3.2 ENABLE

- bit `n`が1なら、pending中のIRQ`n`をIRQC出力の選択対象にします。
- 0にしてもPENDINGはクリアされません。再度1にすると、既にpendingだった要求が出力対象になります。
- ENABLEはIRQ発生源の動作を止めず、IRQC出力をmaskするだけです。

## 4. Source latchとpending動作

IRQCはMMIOに公開されない現在のsource levelを各bitについて保持します。source levelがlowからhighへ変化したとき、そのsourceのPENDING bitをセットします。highの維持やhighからlowへの変化だけではPENDINGをクリアしません。

| Source入力 | 結果 |
|---|---|
| low → high | 対応するPENDING bitをセット |
| high → high | 変化なし。新しいedgeではない |
| high → low | source levelをlowにする。PENDINGは保持 |
| PENDINGへ1を書き込む | 対応するPENDING bitをクリア |

PENDING bitをクリアした時点でsource levelがまだhighの場合、bitは直ちには再セットされません。同じsourceが再通知されるには、sourceが一度lowになった後、改めてhighへ遷移する必要があります。デバイス側の条件を先にACKしてsourceをdeassertし、その後IRQC PENDINGをクリアする順序を推奨します。

## 5. 優先順位とCPU出力

選択対象maskは次のとおりです。

```text
active = PENDING & ENABLE
```

- `active == 0`: `irq_valid = false`、`irq_number = 0`。
- `active != 0`: `irq_valid = true`。最小番号のpending sourceを選び、`irq_number`にその番号を出します。
- 優先順位は固定です。IRQ0が最優先、IRQ15が最低優先です。優先順位設定レジスタはありません。

CPU入力は立ち上がりedgeでIRQをラッチします。CPUは命令完了後に割り込みを受け付け、`CAUSE`に選択された番号を保存し、`0xFFFF0100 + irq_number * 4`のIRQ vectorへ分岐します。IRQ受付時にCPUの割り込み許可は無効になり、`IRET`で復帰します。

IRQC出力がhighのまま選択番号だけ変わった場合、CPU入力には新しい立ち上がりedgeがありません。複数sourceが同時にpendingになった場合、ハンドラは`PENDING & ENABLE`を確認し、有効なpending要因を処理してから復帰してください。IRQCにはEOIレジスタはなく、`IRET`もIRQCのPENDINGまたはENABLEを変更しません。

## 6. 現在のsource割り当て

| Source | 現在の割り当て | 入力レベルの意味 |
|---:|---|---|
| 0 | VBlank/VSYNC | フレーム通知pulse。立ち上がりでpendingになります。 |
| 1 | DMAC | DMACのIRQ pending状態。DMAC内部のIRQ enableで通知を制御します。 |
| 2 | IDC | IDCのIRQ asserted状態。IDC内部のIRQ enableで通知を制御します。 |
| 3 | PeC input | 入力イベントFIFOが非空の間high。FIFOが空になるとlowになります。 |
| 4 | PeC quad timer / RTC | IRQ有効なタイマpendingまたはRTC alarm pendingがある間high。要因はPeC status/pendingで識別します。 |
| 5–15 | 未割り当て | 現在の標準エミュレーター経路では未使用。 |

この割り当てはプラットフォームのsource配線です。IRQC自体は全16 sourceを受け付けます。

## 7. 推奨される割り込み処理手順

1. 起動時に必要なIRQ bitをENABLEへ設定します。CPU側の割り込み許可とIRQ vectorも別途設定します。
2. IRQ handlerで`PENDING & ENABLE`を読み、処理するsourceを確認します。
3. 原因デバイスのstatus/pendingを処理してACKし、source levelをdeassertさせます。
4. IRQCのPENDINGへ該当source bitを書き込み、edge-latched pendingをW1Cします。
5. 同時pendingをすべて処理した後、`IRET`で復帰します。

デバイス側ACKとIRQC PENDING ACKは独立しています。片方だけをACKしても、もう片方の状態は残ります。PeC timer/RTC source 4の詳細は[PeC仕様](spec_PeC.md)、CPUのvectorおよび復帰動作は[CPU仕様](spec_CPU.md)を参照してください。
