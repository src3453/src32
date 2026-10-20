# Windows 終了時の 0xC0000409 調査 (2026-10-20)

## 確認済みの事実

対象は `target/release/cpt32.exe target/tinybasic-c/tinybasic.wax --cpu-execution best-effort --cpu-time-slice-ms 12 --debug-gui --cpu jit` の終了時の異常終了。

Windows Application ログの Event 1000/1001 と保存済み minidump を確認した。

- Report ID: `9d32dbe2-bb1d-4d5e-a2f5-e6c4ad3f6cac`
- 障害モジュール: AMD OpenGL ICD `atio6axx.dll`, version `32.0.31041.1004`
- 例外: `0xC0000409`, exception parameter 0 = `7` (`FAST_FAIL_FATAL_APP_EXIT`)
- 障害位置: `atio6axx.dll+0x12AC465`
- ダンプ: `%LOCALAPPDATA%/CrashDumps/cpt32.exe.17760.dmp`
- ダンプの exception thread ID は 38320。例外ストリームのアドレスと x64 CONTEXT の RIP が上記位置に一致した。
- 別ビルドの Report ID `28ebbe68-2d6e-4a17-a227-38defce1e708` でも同じドライバーバージョン・位置・例外パラメーター。
- `C:/ProgramData/obs-studio-hook/graphics-hook64.dll` もロードされていた。ロード済みというだけで、原因と断定しない。

`STATUS_STACK_BUFFER_OVERRUN` はこのコードの歴史的な名称であり、今回の記録だけでスタックバッファー破壊と判断してはいけない。今回確認できたのは AMD ドライバー内の明示的な fail-fast。ドライバー内部の具体的な異常条件、OBS の関与、JIT の間接的な影響は未確定。

参考: [Microsoft: Misinterpreting the misleadingly-named STATUS_STACK_BUFFER_OVERRUN](https://devblogs.microsoft.com/oldnewthing/20230731-00/?p=108505)。

## ソースから見つかった終了処理の問題

調査時点の `src/main.rs` の `GuiApp` は `window`, `presenter`, `debug_gui` の順にフィールドを宣言していた。終了要求は `event_loop.exit()` のみで、専用の `Drop` / `exiting` による描画資源の解放はなかった。

Rust の構造体フィールドは宣言順に破棄される。そのため `Window` が `WgpuPresenter` 内の `Surface<'static>` より先に破棄される。

`src/render.rs` は `SurfaceTargetUnsafe::from_window` と `create_surface_unsafe` を使っている。wgpu 29.0.3 の `SurfaceTargetUnsafe::RawHandle` は、window/display handle が Surface の破棄後まで有効であることを要求する。現状の順序はその lifetime 契約に反する。これは修正すべき独立した問題で、今回の AMD fail-fast を直接発火させたかは比較実行で確認する必要がある。

また、Instance は全既定バックエンドを初期化する。実際の表示に選ばれていなくても GL の初期化でドライバーがロードされ得るため、障害 DLL 名だけで選択された表示バックエンドを断定しない。

## 次の切り分け

1. debug GUI の GPU 資源、presenter / Surface、window の順に明示的に破棄し、同じ TinyBASIC コマンドで終了を複数回比較する。`create_surface_unsafe` の安全条件を満たすこと自体が必要。
2. 選択 adapter の `get_info()` をログに出し、初期化対象を DX12 だけにした比較を行う。現状は `InstanceDescriptor::new_with_display_handle` を使うため `WGPU_BACKEND` は読み込まれない。環境変数だけを設定した比較は有効でない。
3. OBS のゲームキャプチャーを外した比較、CPU を Cached にした比較を独立に行う。設定を同時に変えない。
4. なお失敗する場合、WinDbg で上記ダンプの `.exr -1; .ecxr; k` を取得し、DLL detach / Surface 解放 / hook のどの経路か調べる。

初回調査ではログ・ダンプの例外情報・ソースを確認した。スタックの完全な unwind は未実施。

## 解放順序の修正と GUI 比較

`GuiApp::release_graphics` を追加し、debug GUI → presenter / Surface → Window の順に `Option::take` で破棄する。`ApplicationHandler::exiting` でイベントループ終了時に解放し、`Drop` でも同じ処理を呼んでエラー／unwind 時をカバーする。二度目の呼び出しは空の Option を処理するだけになる。adapter 名と backend の info ログも追加した。バックエンドの選択設定は変更していない。

- ビルド: `cargo build --release --features jit --bin cpt32` 成功。
- `rustfmt --check --edition 2024 --config skip_children=true src/main.rs src/render.rs` と `git diff --check` 成功。
- 元のバイナリ: 元の cwd / config.yaml と対象 TinyBASIC コマンドで起動し、画面の OK / prompt を確認後 Alt+F4。1回、exit code 0。今回の比較では変更前にもクラッシュは再現しなかった。
- 修正版: 同じ cwd / config.yaml / TinyBASIC / CPU 引数で3回起動。各回 OK / prompt の表示を確認し、1回目 Alt+F4、2回目 Escape、3回目タイトルバーの閉じるボタンで終了。全回 exit code 0。強制終了は使用していない。
- adapter ログ: `AMD Radeon RX 7600`, `backend=Vulkan`。GL ドライバーでの障害記録は、表示 backend が GL であったことを意味しない。
- ログ出力用に修正版の試行では `RUST_LOG=info` を指定した。出力はローカルの `target/shutdown-check/after-{1,2,3}.{stdout,stderr}.log`。この追加ログ設定は比較条件の差である。

この修正は unsafe Surface の lifetime 契約違反を解消するが、元の障害が今回再現していないため AMD fail-fast の完全解消を実証したものではない。再発した場合は新しいダンプと adapter ログを比較し、上記の OBS / backend 切り分けへ進む。
