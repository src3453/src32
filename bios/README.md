# CPT32 BIOS (C)

`main.c` は CPT32 の BIOS ROM です。POST でハードウェアを調べて表示し、120 VSync 保持した後、次の順に起動デバイスを探します。

1. RAM `0x00000000`（ホストがロード済み = 先頭 word が 0 以外）
2. ROM（予約・未実装。常にスキップ）
3. IDC dd0 → dd1 → dd2 → dd3 のブートセクタ（LBA 0 を `0x00000000` へ読み込み、`0x1FE` = `55 AA` を確認）

起動時は R1 に起動デバイス（RAM は `0xFFFFFFFF`、IDC は drive 番号）、SP に `0x000F0000` を入れて `0x00000000` へ分岐します。IRQ は全て無効化・ack 済みです。契約の詳細は [doc/spec_BIOS.md](../doc/spec_BIOS.md) を参照してください。

## ビルド

[LLVM/SRC32 ツールチェーン](../tools/llvm-src32/README.md)（Clang + llvmlite）が必要です。

```
python bios/build.py
```

`assets/bios.rom`（raw、`0xFF000000` 起点、64 KiB 以下）を上書きします。`-o PATH` で出力先、`-O0`/`-O1`/`-O2` で最適化を変えられます（既定 `-O1`）。使用する SDK は `pcg.c`、`idc.c`、`irqc.c` だけです。

ROM は書込みを無視するため、書込み可能な静的データ（SDK の PCG カーソル、IRQ コールバック表など）はドライバーの `--data-origin 0x000F0000` で RAM の BIOS 作業領域に置き、スタートアップが初期値をコピーします。BIOS のコードで新たにグローバル変数を使ってもかまいません。

## ブートセクタの例

```
; R1 = 起動 drive, SP = 0x000F0000
    ADDI R2, R0, 0x41      ; 'A'
    ...                    ; IDC/DMAC を直接操作して残りを読み込む
    HALT
.ORG 0x1FE
.BYTE 0x55
.BYTE 0xAA
```

`python tools/asm/asm.py -o boot.bin boot.a` で 512 bytes の image になります。ディスク image の先頭 512 bytes に書き込んでください（DSFS 媒体では block 0 のブートコード領域に相当し、署名を維持する必要があります）。

## 動作確認

testbench は作業ディレクトリの `config.yaml` から BIOS とディスクを読みます。リポジトリ直下の `config.yaml` は使わず、検証用ディレクトリで実行してください。

- RAM 起動: `bios.image` だけを持つ config で `src32_testbench PROGRAM.bin EXPECTED --frames 140`。
- ディスク起動: 先頭 word が 0 の 4-byte ファイルをプログラムとして渡し、`disk:` に起動 image を設定。FDD は 1 sector 約 108 ms なので `--frames 170` 程度必要です。
