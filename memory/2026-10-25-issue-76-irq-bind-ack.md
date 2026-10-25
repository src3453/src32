# IRQ bind/ack の所有と通知の寿命

- `irq_bind` / `irq_ack` は `FND_CAP_IRQ` が必要。IRQ 0、3、5〜12 だけ bind でき、所有者は bind したプロセス。PORT の dup/転送で所有者は変わらない。同じ所有者でも再 bind は BUSY。
- 割り込みでは ENABLE を落とし PENDING を保持する。IPC は IRQ ごとに 1 件だけ通知を保持し、受信前なら PORT の次の recv、受信待ちなら即時配送する。受信後も ACK までは再通知しない。未受信の ACK は BAD_ARG で、通知を消さない。
- ACK は受信後に原因デバイスを ACK してから呼ぶ。カーネルが PENDING を W1C し ENABLE を戻す。edge latch の競合があるのでサービスは ACK 後にもデバイス状態を再確認する。
- IRQ の割り当ては PORT の ref を増やさない。最後の PORT クローズで割り当て・未受信通知を解放。所有プロセス終了では他プロセスが PORT を保持していても解放する。proc_exit の mm_release より前に解放し、プロセスレコード再利用や PORT 再利用に古い割り当てを残さない。
- ENABLE=0 でも次の VSync で PENDING は再度立つ。終了後の次 tick に PENDING=0 を要求するテストは不正。ENABLE と bind の解放を確認する。
- `python -m unittest discover -s foundation/tests/irq -v` は実際のホストフレーム由来の IRQ0 を使い、VM 選択を明示的に確認した構成と強制 slot を cached/pipeline で実行する。IRQ5〜7 はテストカーネルから通知関数を呼んで、複数通知・重複抑制・確実に B_RECV で待ったスレッドの起床・未受信通知の破棄を検証する。拡張カードの実デバイスは使わない。
- phase1 の旧 UNSUPPORTED プローブは IRQ capability 不足の ACCESS へ更新。`test_foundation_phase1_kernel` は O0/O1/O2 × scalar/cached/cached-relaxed/pipeline で確認。
- console はこの作業では従来の 1 tick ポーリングを継続。`python -m unittest discover -s foundation/tests/shell -k test_ctrl_c -v` を通常構成と `FND_MM=slot` の両方で行い、スレッドを使う外部コマンド中断・ブロック read 中断を回帰確認する。
