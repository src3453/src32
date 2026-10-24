# GitHub Issue と実装状況の照合

- 2026-10-25、Issues 86件（旧エピック #60–63 を含む）の状態と、未完了17件を中心に現在のソース・テスト・既存検証記録を照合し、26件を更新した。リモート main の基準は `921a935d89c02b9c1308e9c2e2d58e12cc868151`。監査後の open は15件。
- `git fetch origin main` 後もローカル main `c72ab37` とリモート main は分岐していた。ローカルにある機能をリモート反映済みと判断しない。今回 checkout の統合・実装修正・Push はしていない。
- #106 は計測 `f27ac63` と4窓LRU修正 `a832904` がリモートに含まれるため close。本文に計測結果と cold/4窓超の限界を反映した。性能数値は既存の `doc/foundation_spawn_profile.md` の診断計測で、今回の再計測値ではない。
- #135 は `1c272a5` の反映を確認して close。後続 #150 により console の FONT grant が復活しているが、これは全角フォントの別領域への書き込み用。設定された1 byte 8×16フォントを保持する契約とは矛盾しない。
- #141 は `e47e6f7` の TUI 中断後 CAN/SGR/カーソル表示/画面消去と反復中断テストを確認して close。既存コメントの通常/slot成功記録と、今回の静的照合を区別した。
- #76 は thread_create/THREAD/wait/thread_exit 実装済み、irq_bind/irq_ack は trap.c で UNSUPPORTED。#77 は書き込みリダイレクト実装済み、script/glob/-c/profile は未実装。#84 は PR #125 の外部コマンド中断がマージ済みだが、組み込み・入れ子スクリプト中断が残る。3件とも本文を更新し open を維持、#84 の in-review を除去した。
- #90 の ADDRESS_CONFLICT と8保護領域制約はスロット構成の契約。VM はプロセスごとのページ表で同じアドレスの独立プロセスを扱える。異なる基底への WAX 再配置とは区別して本文を更新した。
- #152 はローカル `c6ac3a9` に実装がある一方、リモート main には sgubplay.c・仕様・テスト・bootコマンド登録がないため再オープンし on-hold とした。リンク済み PR #153 は MERGED だが、実際のコミットは #150 の全角 console 対応。PRタイトルや Issue の closed 状態だけで完了判定しない。リモート反映と現行SGU/SDKとの focused check が残作業。
- 完了済み18件（#121/#127/#130/#132/#133/#134/#136/#137/#140/#142/#143/#146/#147/#149/#150/#154/#156/#158）の残存状態ラベルを除去。未設定のマイルストーンは #150=Foundation OS、#154/#158=エミュレータ・UI/UX に設定した。旧エピック #60–63 の分類は変更していない。
- #85–89/#92/#93/#138/#139/#151 は未完了の根拠が残るため close しなかった。特に foundation_os_plan.md の古いフェーズ記述は #93 の未完了理由として有効。今回バグの実装修正はしていない。
- 再実行した focused check: `python -m unittest discover -s foundation/tests/disk_cache -v`（1 test / 7 scenarios）と `python -m unittest foundation.tests.console.test_console_host -v`（1 test）は成功。CC は `C:/Program Files/AMD/ROCm/6.4/bin/clang.exe`、TEMP/TMP は workspace 内の `target/issue-audit-temp`。ゲスト・性能・全体テストは再実行していない。
- GitHub 更新後に gh で再取得し、26件の本文・状態・ラベル・指定マイルストーンの一致を確認。closed Issue の作業中ラベルは0件。監査用JSONは ignoredなtarget/に置き、認証情報を保存しない。
