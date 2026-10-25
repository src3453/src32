# Foundation WAVプレイヤー

`wavplay FILE` はFS2上のWAVファイルをSGU v2.1のFIFO DACへ再生する標準コマンドです。
相対パスはシェルの作業ディレクトリ基準です。終了後はシェルへ戻り、Esc / q / Qで停止できます。
Ctrl+Cやプロセス終了時にも `/dev/sgu` のcloseで消音し、占有を解除します。

## 使い方

PowerShellでステレオのサンプルを作り、起動ディスクへ追加します。

```powershell
python examples/foundation-wavplay/demo.py target/wav-root/music/stereo.wav
python foundation/run.py --root target/wav-root
```

Foundationのシェルで実行します。

```text
wavplay /music/stereo.wav
```

自分のWAVファイルも `target/wav-root/music/` へコピーできます。
`config.yaml` の `sgu.version` は既定の `"2.1"` を使います。v1/v2はunsupportedです。
音源は排他的に占有するため、他のSGU再生とは同時に使えません。

## 対応形式

- RIFF/WAVE、format tag 1（整数PCM）、モノラルまたはステレオ。
- 8bit unsigned / 16bit signed little-endian、8,000〜192,000 Hz。
- モノラルはSGU0 ch0を左右へ同じ音量で出力。ステレオはSGU0 ch0を左（PANPOT=0xF0）、
  ch1を右（0x0F）に振り切り、元の左右順を保ちます。
- 48kHzではそのまま出力し、それ以外は整数位相累積のゼロ次ホールドで48kHzに変換します。
  高品質な補間やダウンサンプリング時のアンチエイリアスフィルターはありません。
- float、圧縮WAV、WAVE_FORMAT_EXTENSIBLE、24/32bit、3ch以上、RF64は未対応です。
- RIFFサイズはファイル全体と一致する必要があります。fmt/dataは各1個で、順序は不問。
  未知のチャンクと奇数長チャンクのパディングをスキップし、全チャンクの境界、block align、
  byte rateを再生前に検証します。空のdata、途中で切れたチャンク、部分サンプルはformatエラーです。

## FIFOへの投入

ファイル全体を確保せず、4KiBの入力キャッシュと4KiBの出力バッファを使います。
WAVのLEサンプルをBE PCM16へ変換し、[SGU API](spec_Foundation_API.md#92-sgu-2基と共有-pcmram-devsgu)
でまとめて投入します。両FIFOを停止状態で先行充填してから、一度のカーネル呼び出しで開始します。
FIFOが満杯なら未送信分を保持して1 tick（10ms）後に再試行します。残量に基づいて補充し、
CPUサイクル由来のIRQは生成しません。FIFOの猶予は48kHzで約42.7msなので、
長いホスト停止やディスク・CPU負荷によってアンダーランする可能性があります。

最後に両chへ0サンプルを追加し、FIFOが空になるまで待ってcloseします。
これによりDACの最終値保持による直流出力と、末尾のFIFOを再生し終わる前の切断を避けます。
停止・I/Oエラー時は即座にcloseして消音します。

## 検証

```text
cargo build --release --bin src32_testbench
python -m unittest discover -s foundation/tests/wav -v
```

`FND_MM=slot` でも同じテストを実行します。WindowsではTEMP/TMPを書き込み可能な場所へ設定してください。
ホストテストは符号・エンディアン・レート変換・チャンク境界を検証します。
ゲストテストはFoundation上の実コマンドとDAC APIを実行し、左右の分離、再生完了、停止・再オープン、
権限拒否、FIFO満杯・再試行を検証します。

testbenchの `--dump-audio PATH` はSGUをVSyncごとに800サンプル進め、
両SGUの合成音を48kHz・stereo・signed PCM16 LEのrawファイルへ保存します。
音声デバイスは不要です。指定しない既存テストのSGUは従来通り進めません。
