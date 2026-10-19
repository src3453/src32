# 3WS88PWN4 WASM音源カード仕様

## 資料と互換性

本カードの名前は3WS88PWN4とする。参照元はFurnaceの `src/engine/platform/sound/s3hs88pwn4/` にある3HS88PWN4実装 (`sound.cpp`, `envelove.cpp`, `ram.cpp`, `lib/`) と、ユーザー提供の `3HS88PWN4 Register Sheet 2.xlsx` の第1シート。シートはレジスタ配置の資料として扱う。

2026-10-19に確認したSHA-256:

| 資料 | SHA-256 |
|---|---|
| sound.cpp | `69b3cda74393ab74015e396330cb514c4755f2b1a61e326de267433bb5bcc1dc` |
| Register Sheet 2.xlsx | `e8663002d4a4d96444ed28fa2fc71ef790632b64b9271ee4368b059fb9e29001` |

元コードの音色、13合成方式、16波形、ADSR、PCM/WT/noise/FIFO、filter、pan/mute、EQ/compressorを維持し、所有状態を明確にしたRust実装に置き換える。未実装のADC/GM機能は追加しない。位相の固定小数点化、定義されていなかったゼロ周波数と範囲外アクセスの処理、関数操作のMMIO化は本カードで定義する変更である。

## アドレスとreset

以下はslot相対byteアドレス。slot 0なら `0xA0000000` を加える。

| 範囲 | 内容 |
|---|---|
| `0x000000..0x3FFFFF` | 4 MiB unsigned 8-bit PCM RAM、read/write |
| `0x400000..0x4001FF` | 8 HS channel × 64 bytes |
| `0x400200..0x4002BF` | 4 PCM/WT channel × 48 bytes |
| `0x4002C0..0x4002DF` | master controls、mute |
| `0x400300..0x40033F` | 本カードのFIFO data/count拡張 |
| `0x4002E0..0x4002FF`, `0x400340..0x4003FF` | 予約RAM (read/write、音声には未使用) |
| その他 | read 0、write無視 |

複数byte値は上位byteから並ぶbig-endian。書き込みはbyteごとに有効。card resetはPCM RAM・全register・位相・filter/envelope/feedback・FIFO・resamplerを初期化し、noiseをseed 0で再生成する。IRQ/tickは使用しない。PCM RAMの初期値は0、unsigned PCMの中点は128だが、初期volumeが0なので出力は無音。

## HSレジスタ

HSチャンネル `h=0..7` のbase `H=0x400000+64*h`。

| offset | 内容 |
|---|---|
| `00..01` | OP1基準周波数 `F` (u16 Hz) |
| `02..0F` | OP2..8の倍率、各u16 Q4.12、倍率 `R/4096` |
| `10..17` | OP1..8 volume `0..255` |
| `18..1B` | 2 operator分のwave番号、先のoperatorが上位nibble |
| `1C` | 合成mode `0..12`、他は無音 |
| `1D` | L/R pan nibble、上位がL、下位がR |
| `1E` | GateおよびResetGate書き込み |
| `1F` | feedback、`(value/256-0.5)*2`、128が中立 |
| `20..3F` | 各operatorのA,D,S,R (4 bytes×8) |

### Gate / ResetGate

レジスタシート第1シート `Q3` (`Gate (W:Reset)`、各HSチャンネルに反復) に従い、`H+0x1E` を使用する。
`1`の書き込みごとに8 operatorのenvelopeをAttackに戻し、8位相を0に戻す。同じ1の再書き込みでも必ず実行する。`0`はgateがonの場合Releaseを開始する。その他の値は保存するがgateを変化させない。readは最後に書いたbyteを返す。

元の `resetGate(ch)` と同様、envelopeの現在level・feedbackの前回出力・他チャンネルは消去しない。Attack経過時間だけを0にするため、retrigger直後の第1sampleは直前のenvelope levelを参照する。Gate on/offは書き込み時点で処理するので、render前に連続したon/offを行ってもstrobeを失わない。

### 固定小数点位相

内部生成rateは48,000 Hz、pitch量子化clockは192,000 Hz。
`F>0` なら `P=floor(192000/F)`、`F=0`なら増分0。HS位相はu64のQ32.32 turns。
OP1の倍率は4096、OP2..8の倍率はregister `R`。増分は整数演算で
`round((4 * 2^32 * R)/(P*4096))` とし、sampleごとにwrapping加算する。
最大丸め誤差は1 sampleあたり半LSB。位相をf32/f64に戻して蓄積しない。発音は位相加算後の値を参照する。

変調信号の振幅演算はf32、位相変位は `trunc(signal*4*2^32)` のsigned整数に変換する。
HSの位相と変位の和から `trunc(sum/2^24) & 255` でtable indexを得る。負値も0方向へ切り捨てる元実装の規則を保つ。加算時は広い整数で符号とcarryを保持する。direct phaseではcarrierの進行位相を足さない。

### 波形

256点signed i8 table、HSは最近傍sampleを使用する。

| 番号 | 波形 |
|---|---|
| 0 | sine、振幅127 |
| 1 | 正のhalf sine |
| 2 | 前半に1周期sine、後半0 |
| 3 | 前半127、後半-128のsquare |
| 4 | triangle (1/4周期shift) |
| 5 | saw (1/2周期shift) |
| 6 | 前半127、後半0 |
| 7 | triangleの負値を0にしたもの |
| 8 | sawの負値を0にしたもの |
| 9 | 固定noise table |
| A..D | PCM/WT channel 0..3の32-byte tableを線形補間して256点化し、128を引く |
| E | 前半にabs(sine)1周期、後半0 |
| F | 前半にabs(sine)1周期、後半に負のabs(sine)1周期 |

noiseは標準MT19937 seed 0から最初の65,536個のlow bitをPCM用tableに、続く256個をHS wave 9 (`bit*255-128`) に使う。インスタンス間で共有しない。

### 合成アルゴリズム

`Oi`を各operatorのwave×volume、`A+B → C`を和の4周期相当によるCの位相変調とする。feedbackは前回のchannel出力を `255*127` で正規化して[-1,1]に制限し、feedback register値を掛ける。以下のFB位置を維持する。結果に `255*127` を掛ける。

| mode | routing |
|---|---|
| 0 | O1+...+O8+FB |
| 1 | O5+O6+O7+O8+FB → O1+O2+O3+O4 |
| 2 | (O1+O2+O3+O4)×(O5+O6+O7+O8)+FB |
| 3 | O7+O8 → O5+O6 → O3+O4+FB → O1+O2 |
| 4 | O8 → O7 → O6 → O5 → O4 → O3 → O2+FB → O1 |
| 5 | (O8→O7→O6→O5) + (O4→O3→O2+FB→O1) |
| 6 | (O2+FB→O1)+(O8→O7)+(O6→O5)+(O4→O3) |
| 7 | mode 5の2系列を乗算 |
| 8 | (O1+O2)×(O3+O4)×(O5+O6)×(O7+O8)+FB |
| 9 | mode 6の4系列を乗算 |
| 10 | mode 1、carrierはdirect phase |
| 11 | mode 3、最上流以外をdirect phase |
| 12 | mode 4、最上流以外をdirect phase |

### ADSR

時間は秒。A=`Abyte/64*0.33`、D=`Dbyte/64*1.5` (Dbyte=0時は`0.5/64*1.5`)、S=`Sbyte/255`、R=`Rbyte/64*1.5`。
Attackは0から1へ線形、Decay/Releaseは `1-2^(-10*t)` の補間、SustainはSを維持する。A/R=0は即座に次状態/0へ移行する。levelを参照して `trunc(level*volume)` を得てからenvelopeを1/48000秒更新する。音色維持のため、envelope/振幅/filterの演算は浮動小数点を使用する。

## PCM / WTレジスタ

チャンネル `w=0..3` のbase `W=0x400200+48*w`。

| offset | 内容 |
|---|---|
| `00..01` | 周波数F (u16)、PCMのsample rateは量子化後のF×32 |
| `02` | volume、`value/255` |
| `03` | mode |
| `04` | filter mode (0 LP、1 HP、2 BP、3 notch、それ以外LPの別resonance scale) |
| `05` | cutoff、0ならfilter出力をbypass |
| `06` | resonance/bandwidth |
| `07` | L/R pan nibble |
| `08` | WTSync write strobe、read 0 |
| `09..0F` | 予約RAM |
| `10..2F` | 32 unsigned wave samples、PCM時は先頭9 bytesがstart/end/loop-startのu24 |

WTSyncはシート `K34,K37,K40,K43` に従い `W+8` の任意byte書き込みで該当cursorだけを0にする。filter、FIFO、RAMを変更しない。
cursorはu64 Q32.32 sample positions、増分 `round(128*2^32/P)`。整数部でsample選択し、小数部は線形補間時のみ振幅計算に変換する。元の `twt += float` によるsampleごとの小数周波数切り捨ては行わない。

| mode | 動作 |
|---|---|
| 0 | PCM RAMのunsigned sampleを線形補間 |
| 1 | 32点WTを線形補間 |
| 2 | 65,536点のnoise |
| 3 | noise table先頭64点をrepeat |
| 4 | 32点WTを最近傍で選択 |
| 5 | FIFOから内部sampleごとに1 byte取得、empty時は前回値維持 |
| 他 | sample値0として扱う |

PCMのendはinclusive。`start+floor(cursor)>end`、`loop_start<end`かつloop_start != `0xFFFFFF`なら、元実装どおり `start + floor(cursor) % (end-loop_start)` を読む。この式はloop_startを再生先baseとして加算しない。条件に入らなければendにclampする。補間の次sampleにも同じ条件を使う。4 MiB外は0。破損rangeでも除算0やhost範囲外アクセスを起こさない。

unsigned sampleから128を引き、volumeと255を掛ける。filter有効時はi16範囲へclamp。filterはbypass中も履歴を更新する。PCM/WTは最終mix時に1.3倍する。

### Filter

状態をchannelごとに保持する2次IIR。`y=b0/a0*x+b1/a0*x1+b2/a0*x2-a1/a0*y1-a2/a0*y2`。
`ω=2π*(cutoff+1)*8/48000`。LP/HPの `α=sin(ω)/(1+(resonance+1)/16)`、未定義modeのLPでは分母の16を64にする。BP/notchは `α=sin(ω)*sinh(ln(2)*(resonance+1)/256*ω/sin(ω))`。
`a=[1+α,-2cos(ω),1-α]`。LPのbは`[(1-c)/2,1-c,(1-c)/2]`、HPは`[(1+c)/2,-(1+c),(1+c)/2]`、BPは`[α,0,-α]`、notchは`[1,-2c,1]`。

### FIFO拡張

元の `putDMABuffer()` が使うmode 5をMMIOから利用するため、本カードの追加配置を設ける。シートの予約範囲に置くもので、シート既定の配置ではない。
channel wは `0x400300+16*w`。`+0`はwrite-only data (read 0)、`+1,+2`はread-only pending byte count、上位から。capacityは4096 byte。fullへのwriteは無視し、emptyは前回DAC値を保持する。既存DMACでdata registerに固定宛先転送できる。専用host DMA/ADCはない。

## Master

`0x4002C0`: compressor enable、`+1`: EQ enable (それぞれ1で有効)。
`+2`: threshold/255、`+3`: compressor gain上昇速度/value÷255 per sample、`+4`: volume/32。
`+5..7`: EQ low/mid/high gain (unsigned byte/8 dB)。`+8,+9`: シート上Comp RateU/RateLだが元実装では未使用、本カードでも保存のみ。
`0x4002D0..DB`: 12 channel mute、0のみaudible。

panは各nibble/15、両nibbleが0ならL/R両方15として扱う。mixはHS出力/32768、PCM出力/32768×1.3。順序はmix → EQ → compressor → `32767/4`倍 → i16飽和。
EQは400 Hz low shelf、1000 Hz peaking、4000 Hz high shelfの直列biquad、Q/BW=1/√2。元filter libraryの既定44100 Hzによる係数を互換性のため維持する。
compressorはabs(input)を50 Hz/Q1/48kHz LPに通して検出し、target gain=threshold/detected、上昇最大ratio/255、下降最大4 per sample、gain範囲0..256。検出値0時はtarget=256としてNaNを防ぐ。係数と履歴は `src/filter.rs` に分離する。

## シートと旧実装の差

| 項目 | 採用契約 |
|---|---|
| mode 1/4 | シートは1=WT/4=Interp WTだが、音色維持のためコードの1=補間/4=最近傍を採用 |
| Gate | シートの書き込みresetを採用、旧関数操作をMMIO化 |
| WTSync | シートの+8を採用、旧関数操作をMMIO化 |
| Comp RateU/L | 元コード未使用、追加実装せず保存のみ |
| FIFO | 元コードmode5を保持、予約領域にMMIO data/countを追加 |
| 0 Hz | 旧コードの0除算/未定義変換を再現せず位相停止 |
| 位相 | 全oscillator/cursorは整数32.32、旧float累積とPCMの小数pitch切り捨てを廃止 |

## ホスト接続と検証

WASMカードは [音声ABI](spec_WASM_expansion_card.md) のrenderを実装する。内部48 kHzを整数の有理数accumulatorでhost rateへzero-order hold resampleする。frame/block境界で位相をresetしない。ホストのフレーム粒度のMMIO/音声処理を使用する。

nativeテストは旧C++出力49ケース×512 framesと比較する。13 algorithms/16 waves/6 PCM modes/4 filters/EQ/compressor/muteを網羅。固定小数点で正確に表せる750 Hzと整数倍率、neutral feedbackを使い、phase変更由来の差と機能欠落を混同しない。エフェクトなしは全sample一致、EQ/compressorは最大1 LSB。一般の周波数とfeedbackでのbit一致は主張しない。別テストで一年分の整数位相skipとcarry/overflow、Gate再書き込み/Release、WTSync、FIFO境界を検査する。

fixtureと旧実装比較用C++ harnessは `examples/wasm-3ws88pwn4/tests/`。実WASM packageをWasmtimeで読み込む統合テストは全8 HS×8 operator、8k/44.1k/48k/96k、reset、分割renderの一致を検査する。物理音声デバイスでの試聴は自動テストに含まない。
