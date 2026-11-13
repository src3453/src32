# SGUB Player: ミキサー（チップ・拡張カードごとのゲイン）

- 左パネルの最後のビュー **MIXER**（`X` で開閉）。ストリップは SGU0、SGU1、34SI22（stream が使うときだけ）、各カード slot。
- **ゲインは Engine 側**（`src/music/monitor.rs`）: `sgu_gains: [u16; 2]`、`si22_gain`、`CardSlot::gain`。単位は percent（`UNITY_GAIN` = 100、`MAX_GAIN` = 200）。`apply_gain` は unity のとき標本をそのまま返すので、既定のミックスは参照ミキサーとビット一致のまま（`per_channel_mix_matches_reference_mixer`）。unity で掛け算・割り算を通すと 1 LSB ずれ得るので、この分岐を消さない。
- **掛ける位置**: SGU は各チャンネルの標本（ミュートの判定と同じ場所）、34SI22 は DAC 出力を SGU 和に足す前、カードは i16 にクランプしてから飽和加算。どれも既存のミックス順（SGU + 34SI22 → 1 回飽和 → カード飽和加算）を変えない。
- **メーター**: チャンネル別メーター・`si22_meters`・`CardSlot::meters` はフェーダー前（ミュートも無視）のまま。新しい `Engine::sgu_meters`（コア別 L/R）はミュートとゲインの後。34SI22 とカードのストリップのメーターは GUI 側でフェーダー前の値にゲインを掛け、ミュート中は 0 にしている。
- **マスター**: `Engine::master_gain`／`master_muted` はカードを飽和加算した後の i16 出力に掛けてクランプする（ホストの音量つまみ相当。100% 超では飽和済みの値をさらに上げるのでクリップし得る）。`master_meters`・スコープ・スペクトラムはマスター後の値を見る。ミキサーの MASTER ストリップと、再生コントロール部の FRAME 表示の右（(1160, 136)、マスターメーターの上）の `VOL` バーは同じ値を操作する。`VOL` のドラッグは `master_drag` で別に持つ（`fader_drag` を共用すると、MIXER 表示中にミキサー側がマウスの y からゲインを上書きする）。最初はステータス行の右端に置いたが、ユーザー指定で再生コントロール部へ移した（ステータス文字列は 84 文字のまま）。
- SGU コアには専用のミュートがないので、ストリップの MUTE はそのコアの 8 ボイスの `muted` をまとめて切り替える（全部ミュートならミュート表示）。
- ゲインはファイルを開き直しても残す（`Monitor::open` でチップはそのまま、カードは slot 番号で引き継ぐ）。seek は Engine のゲインに触れない。
- bash ツールに大きなヒアドキュメント（Python スクリプト）を渡すと "unexpected EOF while looking for matching `'`" で丸ごと失敗することがあった。長い置換は Edit ツールで行う。
- 確認: `cargo test --release --test sgub_monitor`（`mixer_gains_scale_each_chip_and_card`: 200% で正確に 2 倍、0% で無音かつ `sgu_meters` 0、34SI22 0% = ミュート、偽カードの +1000 が 50% で +500）。GUI のミキサー画面は目視していない。
