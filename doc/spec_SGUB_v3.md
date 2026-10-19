# SGUB v3 音楽ストリーム仕様案

状態: **Draft**。v3 は v2 の pooled stream を拡張し、CPT32 WASM 拡張カードへの byte write を追加する。v1/v2 の読み取り契約とバイト列は変更しない。拡張slotは最大8枚だが、初版で対応するのはslot 0の1枚のみ。FurnaceのS3HS出力先はslot 0の3WS88PWN4カードとする。target 4..10は将来予約し、初版reader/writerは拒否する。

## v2 からの変更

v3 は v2 と同じ 40-byte header、16-byte pool directory、canonical section layout、frame timing、repeat semantics、14 MiB上限を使い、version byte を `3` とする。v3のoperation addressは32-bitへ拡張する。既存のv2 operation列をv3内に混在させず、v3 operationはすべて新しい長さでdecodeする。pool type 1/2はv2と同じ意味を保持し、新しいtype 3 (`S3HS PCM RAM image`) はraw payload最大 `0x400000` byte。

### Target

WRITE/FILL/COPY_POOL の `target` は下表の値を取る。

| Target | Destination | Address range |
|---:|---|---:|
| 0, 1 | SGU0/SGU1 | `0x000..0x8FF` |
| 2 | shared SGU PCMRAM | `0x00000..0xFFFFF` |
| 3..10 | WASM expansion slot 0..7 aperture | `0x00000000..0x01FFFFFF` |

拡張slot targetのaddressは該当slot aperture先頭からのbyte offset。host addressへの変換は `0xA0000000 + slot * 0x02000000 + address`。フォーマットは8 slot全てを表現するが、初期実装はtarget 3 (slot 0) のみを受け付け、他slotへのoperationは実行前に拒否する。ターゲットカードの未実装範囲のread/write挙動はカード仕様に従い、SGUB readerはカード種類を推測しない。カードが未装着またはfault状態でも、他のtargetの処理は通常のバス契約に従う。

v3 WRITE は `opcode:u8, target:u8, address:u32, value:u8` の7 byteである。副作用のあるregister/strobeを含め、ファイル順に個別byte writeする。FILL/COPY_POOLは各byteを昇順に個別writeする。card targetではカード仕様が明示する通常RAM領域に限り許可し、register/strobe領域へのbulk operationは禁止する。未対応カードやRAM領域を特定できない場合はそのstreamを拒否する。

### v3 FILL / COPY_POOL

FILLは `opcode:u8, target:u8, address:u32, length:u32, value:u8` の11 byte。COPY_POOLは `opcode:u8, target:u8, address:u32, pool_id:u16, source_offset:u32, length:u32` の16 byte。どちらもv2と同様に0..2の既存targetへ適用できる。カードtargetへのFILL/COPY_POOLは対象カードのRAMに限り、カード固有pool typeとdestination rangeの適合を検証する。S3HSではpool type 3をtarget 3の `0x000000..0x3FFFFF` にのみCOPY_POOLできる。S3HS register/FIFO map (`0x400000`以降) はWRITEのみ。

### S3HS card image

Furnace の `CPT32 SGUB (S3HS)` ROM exportは `DIV_SYSTEM_S3HS` 1基だけを受け付け、target 3 (slot 0) にcard WRITEを出力する。slot 0に `3WS88PWN4` WASM cardを装着する。C guest driverは再生前にslot-relative `0x4003F0..0x4003F8` のread-only ASCII card IDを照合し、不一致の場合だけエラーを表示して停止する。SGUB stream parser自体はcard kindを推測しない。S3HS RAM/register offsetは [spec_3WS88PWN4.md](spec_3WS88PWN4.md) のslot-relative mapに従う。Furnace の連続register write列は個々のWRITEとして保存し、同値・同一アドレスのwriteを統合しない。特に `0x400300..0x40033F` FIFO data/count と `0x40001E` Gate strobe は書き込み回数自体に意味がある。S3HS PCM RAM payloadはtype 3 poolに最大 `0x400000` byte格納し、COPY_POOLで `0x000000..0x3FFFFF` に転送する。

COPY_POOLのtype 3はcard PCM RAMのみにコピーでき、source範囲とdestination範囲を超えてはならない。FILLも同じPCM RAM範囲に限る。card register領域はWRITEのみとする。これにより4 MiB sample imageを小さく保持しつつ、strobe semanticsを壊さない。

S3HS register writeの48 kHz captureは既存exporterと同じ累積時間方式で60 Hz frameに量子化する。sub-frame timingは保持しない。カード起動/resetはstreamに含めない。playback開始前にカードをresetし、S3HS cardをslot 0へ装着するのは利用側の責任である。

## Validation

v3 readerは全header/directory/payload/operation/frameを検証してから初期化を適用する。全operationの32-bit addressとlengthをoverflow-safeに検査する。slotのaperture範囲外、カード固有RAM範囲外のbulk op、pool type/destination不整合、未知opcode、truncated frame、余分なtrailing bytesは拒否する。初期実装はslot 0以外のcard targetをunsupportedとして拒否する。type 3はv3のみで有効。v1/v2 readerの制限と実行経路を変えてはならない。

相互運用試験ではv1/v2 byte compatibility、8 slot全てのWRITE先変換、WRITE順序と同一addressの連続strobe、S3HS PCM/register boundary、invalid target/address/opcode、14 MiB超過、Furnace real-engine exportのframe数と出力サイズを確認する。
