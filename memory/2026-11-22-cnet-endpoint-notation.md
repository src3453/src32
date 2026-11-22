# CNET のソケット表記 NET.NODE:SOCK（16 進）

- 日付: 2026-11-22
- Issue: なし（ユーザー指示で作成せず）

## 決めたこと

- エンドポイントは `NET.NODE:SOCK`。SOCK も NET・NODE と同じく **16 進**で `1–FE`（表示は 2 桁、入力は先頭 0 省略可）。0 と FF はソケットとして受け付けない。アドレス部の短縮表記（`.NODE`、`NODE`）はそのまま使える（`20:40`）。
- 以前の `cnet send ADDR SOCKET TEXT`（別引数・10 進）は受け付けない。`cnet recv SOCK` の SOCK も 16 進で、`:SOCK` とも書ける。
- ソケットの数値は変えず表記だけ変えた。TCP ブリッジは 23 のままで、表記は `.FF01:17`。chat の既定ソケットだけは「77」の見た目を保つため値を 0x77 に変えた（デモ内で閉じているので互換の問題なし）。

## 実装

- libcnet に `cnet_parse_endpoint(text, &addr, &socket)` と `cnet_format_endpoint(addr, socket, out[13])` を追加し、WAL を 1.1 に上げた（エクスポート追加はマイナー版を上げる。lzma 1.1 と同じ流儀。ファイル名は `cnet.1.wal` のまま）。
- `cnet_parse_endpoint` は `:SOCK` がないとき `*socket` を変えない。既定値を入れて呼ぶ（telnet は 23、chat は 0x77）。必須にしたい側（`cnet send`）は 0 を入れて呼び、0 のままなら usage。
- telnet/ssh の `-b` は `ADDR[:SOCK]`。`Trying ... via 0000.FF01:17...` の表示が変わったので `test_telnet.py` の期待値も更新した。

## テスト

- `foundation/tests/cnet`（VM、13 件）と `FND_MM=nommu` の `test_cnet.py`。`test_endpoint_notation_names_the_socket` が 16 進ソケット、短縮アドレス、旧形式・`:FF`・`:0` の拒否を確かめる。
