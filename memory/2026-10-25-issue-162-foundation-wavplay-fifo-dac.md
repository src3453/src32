# Foundation WAV再生とFIFO DACの投入契約（#162）

- `wavplay FILE` は整数PCM WAV（8/16bit、mono/stereo、8〜192kHz）を4KiBずつ読む。
  全RIFFチャンクの境界とfmt/dataをSGU取得前に検証する。ファイル全体のheap確保は不要。
  RIFF長・実ファイル長の一致、奇数チャンクのpad、重複fmt/data、部分フレームを確認する。
- SGU FIFO DACは48kHz、BE signed PCM16。WAVはLE（8bitはunsigned）なので変換が必要。
  他レートは整数位相累積のゼロ次ホールド。ダウンサンプリングにフィルターはない。
- `/dev/sgu` のCONFIG/SUBMIT/START（0x1300〜0x1302）はWRITE権限が必要。
  monoはSGU0 ch0/PAN=0xFF、stereoはch0/PAN=0xF0・ch1/PAN=0x0F。
  FIFOを停止中に先行充填し、同じカーネル呼び出しで両chを開始する。
  旧SGUはWAVEFORM=5のreadbackで拒否する。
- SUBMITは4096 bytes以下のBE PCM16 interleaved framesを部分受理する。
  最大残量のchに合わせて左右同数だけ送り、未送信分は利用側が保持する。
  満杯はWOULD_BLOCK、len=0は残量照会。seek位置は変更しない。
  FIFO_COUNTはhigh/low/highを読み、highが変化したら再試行する。
  描画がbyte読み取り間に進むと、単純なhigh/lowでは2048を超える偽の値も生じ得る。
- FIFO空時は最後の値を保持するため、正常終了では0フレームを追加し残量0まで待つ。
  停止・異常・kill時はcloseで全chのvolume/controlを0にする既存契約を使う。
- 48kHz stereoの汎用デコードは-O1ゲストでリアルタイム予算を超えた。
  native PCM16はreadとエンディアン交換の専用経路にし、他レートも変換済みフレームを使い回す。
  定数波形を「出た」とだけ確認するとアンダーラン中の保持値まで成功と誤認する。
  ゲスト音声の非ゼロサンプル数と左右比も検証して、再生時間の伸び・左右ずれを検出する。
- 従来のtestbenchはSGUをclockしないため、DACの満杯以降は進まない。
  `--dump-audio PATH` を付けるとVSyncごとに800サンプル進め、両SGUの合成を
  48kHz/stereo/PCM16 LE rawとして保存する。既存テストの既定動作は変えない。
  このキャプチャは実スピーカーでの聴取確認ではない。
- 検証: `cargo build --release --bin src32_testbench`、
  `python -m unittest discover -s foundation/tests/wav -v` を通常と`FND_MM=slot`で実行。
  ホストでは8〜192kHzの変換・符号・エンディアン・入力境界を確認し、ゲストでは
  FIFO部分受理/満杯/権限、native・44.1kHz・8bit stereo、mono、停止/kill/再取得、旧SGU拒否を確認する。
  WindowsはTEMP/TMPをworkspace内の専用ディレクトリへ設定する。
- 手元のサンプル生成は `python examples/foundation-wavplay/demo.py target/wav-root/music/stereo.wav`。
  左440Hz・右660Hzの44.1kHz WAVを生成する。
