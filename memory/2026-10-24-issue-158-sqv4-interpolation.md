# SGU v2.1: SQV4 の線形補間 (#158)

- SQV4 の補間は新しい版ではなく、v2.1（3WS8PNA）の補間機能の不足分の追加。`SguVersion` やチップ名は上げない。v1/v2 は従来と bit 一致（`frac` を無視）。
- 次の標本がフレーム境界（63→64）を越えるため、デコード済みフレームを 2 枚（`frame & 1`）保持する。デコーダは順方向にしか進まないので、`ensure_sqv4_frame` は目的フレームより後ろへ戻るときだけ先頭から再デコードする。
- 最後の標本の次の tap は保持（ループ先頭へは補間しない）。ループ先頭の標本を読むとデコーダ再開が毎 tick 起きるため。
- SQV4 の音量は Q8.8 のまま `q8 * volume / (255*256)` で適用する（整数 q8 なら従来の `sample*volume/255` と一致）。
- 検証: `cargo test --lib devices::sgu`（`sqv4_two_tap_interpolation_crosses_frames_and_holds_at_end`）。外部 golden に依存する SQV4 テストは無効のまま。
