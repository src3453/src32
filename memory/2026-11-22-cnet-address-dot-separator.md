# CNET アドレスの区切りをコロンからドットへ

- CNET アドレスの文字列表記は `NET.NODE`（例 `0001.0020`、短縮 `.20` と `20`）。`telnet://HOST:PORT` などのポート番号の区切りと紛らわしいので、2026-11-22 にユーザーの指示で `:` から `.` に変えた。旧表記 `1:20` は受け付けない（`cnet_parse_addr` が BAD_ARG）。互換のために `:` を再び受け付けないこと。区別のための変更なので、受け付けると意味がなくなる。
- 解析と表示は libcnet の `cnet_parse_addr`／`cnet_format_addr`（`foundation/lib/cnet/cnet.c`）だけにある。エミュレータ（Rust）とカードは文字列表記を持たない。ほかはドキュメント、テストの期待値、`telnet` の既定ブリッジ `.FF01`、ブリッジのログ（`0001.0010/128`）。
- テスト `foundation/tests/cnet/test_cnet.py` の `test_short_forms_address_the_local_network` は `cnet ping 1:20` が usage になることも確かめる。
- 検証（2026-11-22）: `python -m unittest discover -s tools/cnet-bridge`（13 件）、`python -m unittest discover -s foundation/tests/cnet -v`（12 件。cnet・chat デモ・telnet／ssh）。nommu では再実行していない。
