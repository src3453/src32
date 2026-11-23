# CNET アドレス表記を 10 進既定・末尾 H で 16 進にした

- 日付: 2026-11-23
- Issue: なし（FPGA 用でなく通常 main の作業だが、ユーザー指示で Issue は作成せず）

## 決めたこと

- `NET.NODE[:SOCK]` は **10 進が既定**（`65535.65535:255`）。文字列の**最後**に `H`/`h` を付けると、その文字列全体が 16 進（`FFFF.FFFF:FFH`）。`H` は部分ごとには付けない（`20H:67H` は不正、`20:67H` が正しい）。10 進と 16 進の混在は不可で、`H` なしの `A–F` はエラー。`H` ありでは `1.A:5H` のように全部が 16 進。
- 旧 16 進既定（`0001.0020`、`.FF01`）は受け付けない。`0001.0020` は 10 進の 1.20 になる。例: ブリッジ `.65281`（FF01H）、CFP サーバー `.65282`（FF02H）、ソケット 23／chat 既定 77（以前は 0x77 に変えていたが 10 進の 77 に戻した）。
- ソケットは構文上 0–255。ただし `cnet send` は 0 を拒否する。旧 `cnet_parse_endpoint` は 0 と FF を拒否していたが、表記の例 `:255` を通すため構文は 0–255 に広げた（使えるかは呼び出し側）。
- 出力は 10 進（`cnet_format_addr` は 12 byte、`cnet_format_endpoint` は 16 byte のバッファが必要）。`netfs` の列は 12 桁に合わせた。

## 実装

- `foundation/lib/cnet/cnet.c`（WAL の ABI は変わらない）、`foundation/bin/{cnet,netfs,telnet}.c`、`examples/cnet-chat`、`tools/l3p/l3p.py`（`parse_endpoint`/`parse_node`/`parse_addr`/`format_addr`）。ホストの `--net`/`--node` も同じ記法（`l3p.parse_node`、以前は `--node` が 16 進）。
- 仕様は `doc/spec_NET.md` §2 が正。`spec_NET_bridge`/`_FP`/Foundation API/シェルも追随。

## テスト

- `foundation/tests/cnet`、`foundation/tests/cfp`（要 Clang と testbench）。`test_endpoint_notation_names_the_socket` が 10 進・H 付き・混在・範囲外を確かめる。
