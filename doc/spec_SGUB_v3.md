# SGUB v3 音楽ストリーム仕様案

状態: **Draft**。v3 は v2 の pooled stream を拡張し、CPT32 WASM 拡張カードへの byte write を追加する。v1/v2 の読み取り契約とバイト列は変更しない。Furnace の S3HS 出力先は slot 0 の 3WS88PWN4 カードとする。

## v2 からの変更

v3 は v2 と同じ 40-byte header、16-byte pool directory、canonical section layout、frame timing、repeat semantics、14 MiB上限を使い、version byte のみ `3` とする。既存 v2 の operation encoding はそのまま有効。新しい pool type として type 3 (`S3HS RAM/register image`) を追加し、raw payload の上限は `0x400000` byte。type 1/2 の意味は v2 と同じ。

### Target

WRITE/FILL/COPY_POOL の `target` は下表の値を取る。

| Target | Destination | Address range |
|---:|---|---:|
| 0, 1 | SGU0/SGU1 | `0x000..0x8FF` |
| 2 | shared SGU PCMRAM | `0x00000..0xFFFFF` |
| 3..10 | WASM expansion slot 0..7 aperture | `0x00000000..0x0FFFFFFF` |

拡張slot target のaddressはslot aperture先頭からのbyte offset。host addressへの変換は `0xA0000000 + slot * 0x02000000 + address`。ターゲットカードの未実装範囲のread/write挙動はカード仕様に従い、SGUB readerはカード種類を推測しない。カードが未装着またはfault状態でも、他のtargetの処理は通常のバス契約に従う。

WRITE は副作用のあるregister/strobeを含め、ファイル順に個別byte writeする。card targetへのFILL/COPY_POOLはカードの通常RAM領域だけに許可し、実装は必ず各byteを昇順でwriteする。register/strobe領域へのbulk operationは禁止する。

### S3HS card image

Furnace の `CPT32 SGUB (S3HS)` ROM exportは `DIV_SYSTEM_S3HS` 1基だけを受け付け、target 3 (slot 0) に card WRITE を出力する。slot 0に `3WS88PWN4` WASM cardを装着する。S3HS RAM/register offsetは [spec_3WS88PWN4.md](spec_3WS88PWN4.md) のslot-relative mapに従う。Furnace の連続register write列は個々のWRITEとして保存し、同値・同一アドレスのwriteを統合しない。特に `0x400300..0x40033F` FIFO data/count と `0x40001E` Gate strobe は書き込み回数自体に意味がある。S3HS PCM RAM payloadはtype 3 poolに最大 `0x400000` byte格納し、COPY_POOLで `0x000000..0x3FFFFF` に転送する。

COPY_POOLのtype 3はcard PCM RAMのみにコピーでき、source範囲とdestination範囲を超えてはならない。FILLも同じPCM RAM範囲に限る。card register領域はWRITEのみとする。これにより4 MiB sample imageを小さく保持しつつ、strobe semanticsを壊さない。

S3HS register writeの48 kHz captureは既存exporterと同じ累積時間方式で60 Hz frameに量子化する。sub-frame timingは保持しない。カード起動/resetはstreamに含めない。playback開始前にカードをresetし、S3HS cardをslot 0へ装着するのは利用側の責任である。

## Validation

v3 readerは全header/directory/payload/operation/frameを検証してから初期化を適用する。v3 target 3..10はWRITEをaperture範囲内で許可し、FILL/COPY_POOLはcardのRAM範囲 `0x000000..0x3FFFFF` に限定する。COPY_POOLにはtype 3のみを使う。type 3はv3のみで有効。未知pool type、既知でないoperation、truncated frame、余分なtrailing bytesは拒否する。v1/v2 readerの制限と実行経路を変えてはならない。

相互運用試験ではv1/v2 byte compatibility、8 slot全てのWRITE先変換、WRITE順序と同一addressの連続strobe、S3HS PCM/register boundary、invalid target/address/opcode、14 MiB超過、Furnace real-engine exportのframe数と出力サイズを確認する。
