# BMC / DMAC 仕様

本書はCPT32の単一システムバスにおける調停とDMA転送の初版仕様を定める。アドレスとデータは32-bit、レジスタ値はビッグエンディアンとする。MMIOレジスタは32-bitアクセスを基本とし、未定義オフセットおよび未定義ビットは読み出し0、書き込み無視とする。

## 1. BMC (Bus Master Controller)

### 1.1 役割とバスマスター

BMCは複数のバスマスターからの要求を一つずつ選び、単一バスの所有権を与える。マスターは次の8つとする。

| Master ID | マスター |
|---:|---|
| 0 | SRC32 CPU |
| 1 | VPU |
| 2–7 | DMAC channel 0–5 |

一つのマスターは同時に一件の未完了トランザクションを保持できる。BMCはトランザクションの完了またはエラー応答まで次のマスターへ所有権を移さない。マスターは完了応答まで要求とアドレス、方向、書き込みデータを保持する。

### 1.2 優先度スロット

各マスターにはP0～P7のいずれか一つを割り当てる。P0が最高、P7が最低の優先度である。BMCは要求中のマスターから最も高い優先度を選び、同じ優先度に複数要求がある場合はラウンドロビンで選ぶ。選択後の次回探索は直前に選ばれたマスターの次から始める。

連続要求による飢餓を避けるため、待機中マスターは他マスターへの調停grantが256回発生するごとに実効優先度を一段上げる。実効優先度はP0を上限とし、要求がgrantされると待機時間をリセットする。DMAバーストは設定上限のbeat数で必ず一度バスを解放するため、CPU/VPU要求を無期限に塞がない。

初期優先度はCPU=P0、VPU=P1、DMA channel 0–5=P2とする。ソフトウェアはマスターごとにP0～P7を設定できる。DMAC転送中の優先度変更は次の調停から反映する。

### 1.3 アクセスとエラー

- 1 beatは8/16/32-bitの一回のバスアクセスである。アラインメント規則と未定義アドレスのBus Error規則はシステム仕様に従う。
- 一つのbeatの途中で所有権を切り替えない。DMAは複数beatを連続発行できるが、最大バースト長到達時に再調停する。
- バスマスターが応答待ちの間は他の要求をgrantしない。デバイスは有限時間内に完了またはエラーを返す。
- CPUは同期アクセスを維持し、バスが空くまで待機する。DMA/VPUは要求を保持して非同期にgrantを待つ。
- BMCリセット時は現在のgrant、待機時間、優先度設定を初期値へ戻す。進行中のアクセスはシステムリセットにより中断される。

### 1.4 BMC MMIO

ベースアドレスは`0x80060000`。全レジスタは32-bit、4-byte境界。

| Offset | Register | Access | 内容 |
|---:|---|:---:|---|
| `0x000` | ID | RO | `0x424D4301` ("BMC", version 1) |
| `0x004` | CONTROL | RW | bit 0: 調停有効。0ではCPU以外の新規grantを停止する。reset値1 |
| `0x008` | STATUS | RO | bit 0: bus busy、bit 1: grant中マスター有効 |
| `0x00C` | ACTIVE_MASTER | RO | grant中Master ID、grantがなければ`0xFFFFFFFF` |
| `0x010` | PENDING | RO | bit 0–7: Master IDごとの要求中ビット |
| `0x020`–`0x03C` | MASTER_PRIORITY[n] | RW | Master ID `n`の優先度。bit 2:0=P0..P7。他ビットは0。reset値はCPU P0、VPU P1、DMA P2 |
| `0x040` | MAX_BURST | RW | DMAの最大連続beat数。値0は1 beat、reset値16、範囲1–256 |

CONTROLで調停を無効にしても既に開始したbeatは完了させる。MASTER_PRIORITYへの書き込みは値の下位3-bitのみ受理する。

## 2. DMAC (Direct Memory Access Controller)

### 2.1 機能と転送モデル

DMACは6チャネルの独立したDMA要求を管理する。各チャネルはプログラム可能なsource、destination、byte countを持ち、BMC上では独立したバスマスターとして調停される。複数チャネルの転送は同時に設定・待機できるが、単一バス上では一度に一つのbeatだけが実行される。

- 1チャネルの転送長は1～`0xFFFFFFFF` byte。0 byteは開始エラー。
- 転送幅は8/16/32-bitから選び、byte countは転送幅の倍数とする。source/destinationは転送幅に整列させる。
- sourceとdestinationはそれぞれ固定または転送幅分のincrementを選択できる。初版ではdecrement、2D/stride、scatter-gather、chainingはサポートしない。
- メモリ転送対象はMain RAM、VRAM、PCMRAM。MMIOは通常の転送先として不許可だが、既存の32-bit固定destination例外としてVPUの`FIFO_DATA` (`0x8003002C`)を許可する。加えてIDC仕様で定める場合に限り、唯一の追加MMIO endpoint `0x80041024` (PeC IDC `DATA`) を8-bit転送で使用できる。その他のMMIO、ROM、未定義領域は不許可とし、違反は当該チャネルのADDR_ERRORで停止する。
- IDC endpoint例外の向きはIDC operationで固定する。READはsource=`0x80041024` fixed、destination=許可されたRAM increment、WRITEはsource=許可されたRAM increment、destination=`0x80041024` fixed。幅は8-bitのみ、byte countはactiveなIDC READ/WRITEの`SECTOR_COUNT × 512`と完全一致させる。VPU例外は既存どおり32-bit fixed destinationに限り、通常メモリDMA制約は変わらない。IDC endpointをこの形式以外で使う設定は開始時の設定エラー（`ERROR_CODE=1`）とする。
- 読み出しbeat完了後に書き込みbeatを発行する。途中エラー時は残りを中止し、既に書き込んだデータはロールバックしない。
- sourceとdestinationが重なる場合の動作は逐次コピーと同じで、memmove相当の逆方向コピーは行わない。
- 完了時にDONEを立てる。IRQ_ENABLEの該当チャネルbitが1なら共有IRQ1を要求する。IRQ_STATUSとDONEはソフトウェアがクリアする。

### 2.2 DMAC MMIO

ベースアドレスは`0x80050000`。全レジスタは32-bit、4-byte境界。

| Offset | Register | Access | 内容 |
|---:|---|:---:|---|
| `0x000` | ID | RO | `0x444D4101` ("DMA", version 1) |
| `0x004` | GLOBAL_CONTROL | RW | bit 0: DMAC有効、reset値1 |
| `0x008` | CHANNEL_ENABLE | RW | bit 0–5: channel enable。0のチャネルは開始不可 |
| `0x00C` | IRQ_ENABLE | RW | bit 0–5: channelごとのIRQ許可 |
| `0x010` | IRQ_STATUS | RW1C | bit 0–5: 完了またはエラーIRQ pending。1書き込みで該当bitをクリア |
| `0x014` | ABORT | WO | bit 0–5に1を書き込むと該当チャネルを中止 |
| `0x100 + n*0x20` | CHn_SRC | RW | source address |
| `0x104 + n*0x20` | CHn_DST | RW | destination address |
| `0x108 + n*0x20` | CHn_COUNT | RW | 転送byte数 |
| `0x10C + n*0x20` | CHn_CONTROL | RW | 転送条件とSTART |
| `0x110 + n*0x20` | CHn_STATUS | RW1C | 状態bit。DONE/ERRORは1書き込みでクリアし、ERROR_CODEも0に戻る |
| `0x114 + n*0x20` | CHn_REMAIN | RO | 未転送byte数。開始前はCHn_COUNT |
| `0x118 + n*0x20` | CHn_PRIORITY | RW | bit 2:0=BMC優先度P0～P7。reset値P2 |

チャネルnは0～5。CHn_CONTROLのbit定義:

| Bit | 意味 |
|---:|---|
| 1:0 | WIDTH: `00`=8-bit、`01`=16-bit、`10`=32-bit、`11`=予約（開始エラー） |
| 3:2 | SRC_MODE: `00`=increment、`01`=fixed、他=予約 |
| 5:4 | DST_MODE: `00`=increment、`01`=fixed、他=予約 |
| 8 | START: 1を書き込むと開始。自己クリア |
| 9 | ABORT: 1を書き込むと中止。自己クリア |
| 31:10, 7:6 | 予約、読み出し0 |

CHn_STATUSのbit定義:

| Bit | 意味 |
|---:|---|
| 0 | BUSY |
| 1 | DONE |
| 2 | ERROR |
| 7:4 | ERROR_CODE: 0=なし、1=設定不正、2=アドレス不許可/Bus Error、3=中止 |

### 2.3 開始・進行・終了

開始時にDMACは設定値を検査し、SRC/DST/COUNTを内部にラッチする。BUSY中の設定レジスタ書き込みと再STARTは無視し、進行中の転送状態は変更しない。開始条件不正の場合はデータ転送を行わず、ERROR_CODE=1でERRORを立てる。CHn_PRIORITYはBUSY中に変更しても次の調停から使う。

IDC DATA endpointを含む開始時検査では、IDC READ/WRITE commandがactiveであること、operationに対応するsource/destination addressとfixed/increment mode、8-bit幅、IDC sector countと完全一致するbyte countを検証する。IDC commandがactiveでない場合、方向/幅/mode/count/addressが合わない場合、または別channelの設定が同じIDC endpointを使用する場合、STARTを拒否してDMAC `ERROR_CODE=1`でERRORを記録する。IDC側の進行中commandはDMA mismatch (`ERROR_CODE=5`)として終了する。IDC DATA以外のMMIO endpointをこの例外で許可してはならない。

IDC channelはIDCがassertするrequest/DRQがあるbyteだけを転送する。IDCがsector readyを示す前はDMAC requestを出さず、BMC bus beatを発行しない。DRQが下がっている間、転送byte count/REMAINを進めずchannelはready待ちでBUSYを維持する。IDC sector timing、staging、media error、sector境界ABORTの詳細は[`spec_IDC.md`](spec_IDC.md)に従う。DMAC側ABORT自体は従来どおりbeat境界で停止し、IDCに中断を通知する。

DMA有効化、チャネル有効化、CHn_CONTROL.STARTの順に設定する。転送中はCHn_REMAINをbyte単位で更新する。正常完了時はBUSYを下げ、REMAIN=0、DONEを立てる。IRQ_ENABLEの該当チャネルbitが1ならIRQ_STATUSにpendingを立て、IRQCのIRQ1へ通知する。エラーでもERRORとIRQ_STATUSを立て、該当チャネルのIRQが許可されていればIRQ1を通知する。

ABORTまたはCHn_CONTROL.ABORTは次のbeat境界で停止し、完了済みの書き込みを保持したままBUSYを下げる。中止はERROR_CODE=3としてERRORを立てる。GLOBAL_CONTROLを0にすると新しいチャネル開始を禁止し、進行中チャネルも次のbeat境界で中止してERROR_CODE=3を記録する。

## 3. リセット値と予約事項

リセット直後、DMACの全チャネルは無効かつidle、IRQ_ENABLE/IRQ_STATUSは0である。BMCは有効で、CPU=P0、VPU=P1、各DMA=P2、MAX_BURST=16とする。IDC DATA endpointのsector-ready handshakeは本書2.3節と[`spec_IDC.md`](spec_IDC.md)で定める。その他のデバイス待ち時間やPeC FIFO handshakeは個別のデバイス仕様に従う。
