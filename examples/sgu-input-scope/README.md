# SGU v3 入力波形テスト

34SI22 の録音 A/B でホスト入力の L/R を PCM16 として PCM RAM に取得し、SGC で波形を表示するベアメタルプログラムです。緑が L、青が R、灰色がゼロ線です。ゲインは 0 dB、AGC 無効、48 kHz、各 160 標本（約 3.33 ms）を表示します。フルスケールは各ゼロ線から約 45 ピクセルです。同期トリガーやアンチエイリアスはありません。SGC は順次描画するため、更新中に描画途中の画面が見えます。録音期間の間には描画による空白があります。

## ビルド

LLVM/Clang 20、Python と `tools/llvm-src32/requirements.txt`、CMake、Ninja が必要です。パスは環境に合わせてください。

```powershell
cmake -S examples/sgu-input-scope -B target/sgu-input-scope -G Ninja `
  '-DSRC32_CLANG=C:/Program Files/LLVM/bin/clang.exe' `
  '-DSRC32_PYTHON_EXECUTABLE=C:/path/to/python.exe' `
  '-DCMAKE_MAKE_PROGRAM=C:/path/to/ninja.exe'
cmake --build target/sgu-input-scope
cargo build --bin cpt32
```

## 実行

BIOS を設定しないディレクトリに `config.yaml` を作成します。`sgu.version` は省略時 3 です。入力デバイス名は OS に表示される名前と完全一致させます。省略すると OS の既定入力です。

```yaml
sgu:
  version: 3
audio:
  input_device: "Voicemeeter Out A1 (VB-Audio Voicemeeter VAIO)"
```

そのディレクトリから、ビルドしたエミュレータと WAX の絶対パスを指定して起動します。

```powershell
& C:/path/to/src32/target/debug/cpt32.exe C:/path/to/src32/target/sgu-input-scope/sgu_input_scope.wax
```

VoiceMeeter の A1 に音を送ってください。入力が無音・接続不可の場合は中央の水平線になります。音が入力されても左右いずれかの録音に進展がない場合は画面が更新されません。ゲストは VSync の人工生成をせず、エミュレータのフレームループによる音声更新を使います。

## ヘッドレス確認

入力は無音です。BIOS のないディレクトリから以下を実行すると左右の水平波形を検証できます。画素の指定は描画途中の 14 フレーム目に合わせています。

```powershell
& C:/path/to/src32/target/debug/src32_testbench.exe C:/path/to/src32/target/sgu-input-scope/sgu_input_scope.wax --cpu cached --allow-running --frames 14 --expect-pixel 250:65=0x40ff80 --expect-pixel 50:175=0x40b4ff
```
