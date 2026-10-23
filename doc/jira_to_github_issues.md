# Jira (KAN) から GitHub Issues への移行対応表

2026-10-23 に Jira プロジェクト `KAN` の全課題を GitHub Issues へ移しました。Jira 側の課題は削除していません。

* 完了した課題は closed、それ以外は open です。分野はマイルストーン（Foundation OS／開発基盤／エミュレータ・UI/UX／SRC32 CPU）で表し、エピックの親子リンクは使いません。
* 移行元のエピック KAN-4 / KAN-5 / KAN-39 / KAN-44 は #60 / #61 / #62 / #63（いずれも not planned で close、廃止）に対応します。
* MEMORY.md などに残る `KAN-n` の記述はこの表で読み替えます。

| Jira | Issue | タイトル |
|---|---|---|
| KAN-6 | #64 | フェーズ1 最小カーネル（U モード・優先度付きラウンドロビン） |
| KAN-7 | #65 | sol の !abi_compat "Foundation"（PROTECTED WAX 出力） |
| KAN-8 | #66 | カーネルの WAX ローダー（PROTECTED v2・WAX/WAL v3） |
| KAN-9 | #67 | シェル仕様 v1 と API 仕様ドラフト |
| KAN-10 | #68 | 最低限のシェル（PCG 表示）と UNIX 風コマンド名 |
| KAN-11 | #69 | シェルの外部コマンド化（/bin）とフェーズ2（ハンドル・IPC・proc_spawn） |
| KAN-12 | #70 | WAL 対応ツールチェーンと WAL の置き場の決定 |
| KAN-13 | #71 | FS2 ディスクイメージからの起動（stage1 + FNDBOOT） |
| KAN-14 | #72 | top コマンドと CPU 使用量の統計（sys_info sel 2） |
| KAN-15 | #73 | fs サービス（FS2 ドライバ・VFS・blk_io・disk サービス） |
| KAN-16 | #74 | ゲストへのメモリ容量の通知（BMC メモリ容量レジスタ） |
| KAN-17 | #75 | ファイル系コマンド（ls・cat・cp・rm・mv・mkdir・rmdir）を /bin に追加 |
| KAN-18 | #76 | thread_create・irq_bind・irq_ack の実装 |
| KAN-19 | #77 | シェルのスクリプト（.fsh）・ワイルドカード・書き込みリダイレクト |
| KAN-20 | #78 | GitHub Actions による CI/CD と高速化 |
| KAN-21 | #79 | エミュレータの Esc 終了を無効化し、ゲストへキー入力として渡す |
| KAN-22 | #80 | FS2 イメージをホストから操作する 9P2000 サーバー |
| KAN-23 | #81 | Foundation 起動時のエミュレータ引数転送（run.py --arg） |
| KAN-24 | #82 | CLAUDE.md の追加と Jira 運用ルール |
| KAN-25 | #83 | dos_smoke テストでキー入力が取りこぼされる |
| KAN-26 | #84 | Ctrl+C による外部コマンド・スクリプトの中断を完成する |
| KAN-27 | #85 | FS2 のオフライン fsck 修復と DIRTY 媒体の復旧を実装する |
| KAN-28 | #86 | 一貫した RTC スナップショットと fnd_time_now を仕様化・実装する |
| KAN-29 | #87 | カーネル管理の UID 資格情報と FS セッション権限を仕様化・実装する |
| KAN-30 | #88 | DEV_OPEN の直接呼出しによる /dev 権限検査の迂回を閉じる |
| KAN-31 | #89 | IDC のホスト共有ディレクトリを FS2 互換媒体として提供する |
| KAN-32 | #90 | 再配置可能な WAX の ABI と複数インスタンス起動を設計する |
| KAN-33 | #91 | sol/foundation に libfnd のファイル・プロセス・コンソール API を追加する |
| KAN-34 | #92 | Foundation 用拡張カードドライバの権限・IRQ・公開 API を設計する |
| KAN-35 | #93 | Foundation 実装案の進捗・コマンド名を現行仕様と実装に同期する |
| KAN-36 | #94 | シェルテストが極端に遅い（Foundation の起動が遅い） |
| KAN-37 | #95 | Foundation shell再起動テストのps状態がBLOCKではなくRUNになり失敗する |
| KAN-38 | #96 | Foundation起動の暫定対策としてFS2マウント時の全走査を省略 |
| KAN-40 | #97 | シェルで ls が動かない（ls: command not found） |
| KAN-41 | #98 | PR #13 以降、Foundation のカーネルと API の性能が大きく低下している |
| KAN-42 | #99 | AGENTS.md に Jira 課題の追加と状態管理ルールを明記 |
| KAN-43 | #100 | AGENTS.md の Jira ルールにエピック KAN-39 を追記 |
| KAN-45 | #101 | AGENTS.md の Jira ルールにエピック KAN-44 を追記 |
| KAN-46 | #102 | Dependabot による依存関係更新を導入する |
| KAN-47 | #103 | LZMA圧縮・展開ライブラリ liblzma.wal を追加する |
| KAN-48 | #104 | Extension V（仮想記憶）の仕様化とエミュレータ実装 |
| KAN-49 | #105 | vmfoundation: Foundation の仮想記憶対応 |
| KAN-50 | #106 | コマンド起動（プロセス spawn）が 1 回約 13 Mcycle かかる原因の調査 |
| KAN-51 | #107 | パイプライン CPU で CPUID の R1/R2 書き込みを interlock する |
| KAN-52 | #108 | デバッグメニューに MMU/TLB ウィンドウを追加 |
| KAN-53 | #109 | Debug Controls に CPU 内部レジスタ（EPC/CAUSE/特権モード等）を表示 |
| KAN-54 | #110 | C版TinyBASICをFoundationゲストとして実装する |
| KAN-55 | #111 | Foundationシェルの12MiB構成でfreeテストの期待値と実測が一致しない |
| KAN-56 | #112 | /bin/hex: DOS の hex コマンドの移植 |
| KAN-57 | #113 | Foundation の tick とクオンタムを 10ms にする |
| KAN-58 | #114 | liblzma.wal を既定ディスクの /lib/lzma.1.wal に含める |
| KAN-59 | #115 | llvm-src32 の可変長引数と softfloat 浮動小数点への対応 |
| KAN-60 | #116 | Foundation 用オリジナル CHIP-8 エミュレータを実装する |
| KAN-61 | #117 | Foundation fb0 が表示を無効にして CHIP-8 が黒画面になるバグを修正する |
| KAN-62 | #118 | main マージ時の MEMORY.md 追記競合を解消する |
| KAN-63 | #119 | CHIP-8 のコマンドライン引数で VSync あたりのサイクル数を指定する |
| KAN-64 | #120 | llvm-src32 のコンパイル並列化 |
| KAN-65 | #121 | foundation/run.py に -j を追加（既定は物理コア数） |
