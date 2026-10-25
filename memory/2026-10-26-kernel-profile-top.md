# カーネルの命令プロファイル（`top -n 4`）と pcprof

- 実機の `top` が kernel 77.6% / idle 8% になるのは、エミュレータの約 8 倍の忙しさ（CPI の差）で、UART の不具合ではない。エミュレータの同じ `top -n 1` は kernel 9.8% / idle 88.7%。ユーザーとカーネルの比も実機と同じ（約 1:8）。
- ツール: `tools/pcprof/pcprof.py`（`foundation` サブコマンドで再現）。テストベンチに `--profile-pc PATH` と `--profile-pc-reset-frame N` を追加した（パイプライン CPU だけ）。PipelineCpu に `enable_pc_profile` / `take_pc_profile`。
- 結果（フレーム 140 以降、`top -n 4`、カーネル 41.97M 命令 / ユーザー 5.38M 命令）:

| 関数 | 命令の割合 | 呼び出し | 命令/呼び出し |
|---|---|---|---|
| `take_next`（ipc.c） | 24.4% | 3,520 | 2,914 |
| `proc_user_range_ok`（mm.c。`vm_range_ok` を含む） | 14.4% | 12,886 | 470 |
| `kmem_copy` | 13.5% | 13,775 | 410 |
| `mm_vm_copy_from` / `_to` / `mm_vm_copy` | 7.0 / 4.3 / 2.2% | 9,356 / 3,359 / 2,917 | 314 / 535 / 314 |
| `kernel_trap` + `sched_next` | 6.4% + 5.5% | 8,905 | 301 + 258 |
| `sys_ipc_call` / `recv` / `reply` | 3.4 / 2.2 / 1.6% | 1,459 / 2,061 / 1,458 | 966 / 442 / 458 |
| `sched_tick` + `ipc_pec_poll` | 2.1% + 1.9% | 654 | 1,362 + 1,237 |

- `take_next` は受信のたびに `irqs[16]` を全部走査する（ホットな PC の実行回数が呼び出し回数のちょうど 16 倍）。1 周約 180 命令。保留 IRQ のビットマスクを持てば、ほぼ 0 にできる見込み（未実装）。
- 生成コードは値をすべてスタックに退避する（`irqs[i]` の添字計算ごとに MUL、ジャンプは LDIH/LDIL/JR）。C の 1 行がそのまま数十命令になるため、ループ回数がそのまま効く。`proc_user_range_ok` は IPC 1 回あたり約 4 回呼ばれ、`ipc.c` の文字列検査はバイトごとに 1 回呼ぶ。
- `top` は 10 ms ごとに `fnd_con_pollkey`（IPC）と `fnd_sys_sleep(1)` を呼ぶので、1 秒に約 100 回のシステムコールとコンテキストスイッチが起きる。これが負荷の大半。

## 修正と再計測（同日）

- 修正（`foundation/kernel/ipc.c`, `mm.c`）:
  - `take_next`: 保留 IRQ のビットマスク `irq_pending_mask` を持ち、0 なら 16 本の走査を飛ばす（`irq_set_pending` 経由でだけ更新）。`next_caller` は B_SEND で待つスレッドのビットマスク `send_blocked` を遅延クリアで持ち、0 ならスレッド表を走査しない。`__builtin_ctz` はバックエンドの対応が不明なので使わない。
  - `proc_user_range_ok`: 1 ページ内の成功を 8 エントリ（process と ページでインデックス、perm の和を保持）にキャッシュする。ページテーブルを変える `mm_vm_create/cow/map/map_phys/clone` と `vm_destroy` が `vm_epoch` を進めて無効化する。fork 後の W 検査が COW 解決を飛ばさないための要。`vm_range_ok` は COW を見たときだけ `vm_prepare_write` を呼ぶ（2 度引きをやめた）。
  - `mm_vm_copy_to`: 1 ページ内で COW でなければ 1 回の引きでコピーする。`write_recv`: 64 B 読み 64 B 書きをやめ、必要な 8 B を読み先頭 56 B だけ書く。
- 結果（`top -n 4`、同じ窓）: カーネル命令 41.97M → 31.41M（-25%）。`take_next` 10.26M → 3.46M、`proc_user_range_ok` 6.05M → 2.39M（切り出された `vm_range_ok` 1.36M を足して 3.74M）、`mm_vm_copy_to` 1.80M → 0.69M。`kmem_copy` 5.3M と `mm_vm_copy_from` 2.9M はほぼ変わらない。
- `take_next` の残り（1 呼び出し約 1000 命令）は、実際に送信待ちを受け取る 1,450 回の配送処理（`deliver_call` と `write_recv`）。`proc_user_range_ok` の命令は、キャッシュ引き 1 回が約 190 命令。生成コードの重さがそのまま効く。
- 検証: shell 30 件（VM / `FND_MM=nommu` の両方）、irq / pec / panic / fs / disk / perf、`test_toolchain` の phase1 / fork / vm_kernel / wal_from_c / wax_loader がすべて通過。
- 次に効きそうなもの: `kmem_copy`（32 B ごとに約 190 命令）と `mm_vm_copy_from` の呼び出し回数（IPC 1 回あたり約 6 回）、`kernel_trap` + `sched_next` + `ctx` の約 800 命令/トラップ、`top` の 10 ms ポーリング自体。
