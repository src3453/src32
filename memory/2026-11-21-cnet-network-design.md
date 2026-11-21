# CNET ネットワークの設計方針（草案、未実装）

- 仕様草案は `doc/spec_NET.md`。2026-11-21 にユーザーと相談して決めた。§1.1「決定事項」は合意済み。バイト配置、レジスタ番地、タイムアウトなどの具体値は案で、実装時に見直してよい。
- TCP/IP 非互換の AppleTalk 風。アドレスは 32 bit で、上位 16 bit がネットワーク番号、下位 16 bit がノード番号。ユーザーの言う「下 16 ビット＝サブネット」は、ネットワーク内のノード番号の意味だった。
- スタックは WASM カード内（「ハードウェア側」）。ホストは物理層だけで、`net_send`／`net_recv`／`net_info` の生フレーム import を持つ。カードが外部の宛先 IP を選べないことが安全境界で、過去の TCP/UDP オフロード案（`2026-10-26-wasm-network-permission-design.md`）とは別方針。
- 将来は TCP/IP ブリッジを考えている。ブリッジはカードに IP を入れず、セグメント上の 1 ノード（予約ノード `0xFF00–0xFFFE`、PROTO `0x40–0x7F`）として作る前提で、そのための予約を最初の版から入れてある。消さないこと。
- manifest の `capabilities: [network]` は旧ホストに `deny_unknown_fields` で拒否させる意図がある。宣言は許可ではなく、許可は `config.yaml` の `expansion_card[].network.segment` 接続だけ。
- 検証はまだしていない（文書のみ）。実装時の検証項目は spec §10。
