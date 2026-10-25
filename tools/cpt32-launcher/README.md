# CPT32 GUI ランチャー

Windows／Linuxで使えるPython 3.10+・Tkinter製ランチャーです。追加Pythonパッケージなしで動作し、PyYAMLが入っていれば一般的なYAML構文も使えます。

```powershell
python tools/cpt32-launcher/launcher.py
```

起動コマンドと各ビルド／エミュレータープロセスの出力はLauncher画面の起動ログに表示し、Pythonプロセスの標準出力にもflushして出力します。端末から`python tools/cpt32-launcher/launcher.py`で起動すると、コンソールでログを追えます。画面ログは末尾20万文字まで保持し、大量出力時は最大2048件の待機キューに入らない行を省略して件数を表示します。コンソールへの出力はこの省略の対象になりません。長い行は8192文字ずつ分割して読み取り、画面更新は1回最大256件／約64000文字で区切るため、出力が続いても操作や終了通知を処理できます。

ROM／WAX／CPT32PKG、CPU方式、通常GUI／monitor、CPU実行ポリシー、デバッグGUI、profile、music stream、BIOS ROM、PCG font、FDD／HDDイメージ（4スロット）、WASM expansion card（8スロット）、IDC disk sound asset rootを設定できます。WASM expansion cardは「選択…」でpackageを選ぶと、そのslotが自動的に有効になります。CPU実行ポリシーは Strict、Best-effort（既定）、Cycle-independent から選べ、時間枠は1〜16 ms（既定8 ms）です。GUI起動時に設定をコマンドラインへ渡し、保存済みランチャープロファイルにも保持します。monitor起動ではCPU実行ポリシーは適用しません。`pipeline`、`cached`、`cached-relaxed`、`jit`、`scalar`に対応し、JITを選ぶとCargo起動に`--features jit`を追加します。詳細設定でJITブロックをSmall=16、Medium=32、Big=64命令から選ぶか、2〜256の具体値を指定できます（既定16）。大きい値では生成コード量やコンパイル時間が増え、遅くなる場合があります。値は設定YAMLとランチャープロファイルに保持されます。既存実行ファイルを指定してビルドせずに起動することもできます。

ROM欄には `.sol` ソースも指定できます。「この設定で起動」を押すたびに、選択したCargo workspaceの `tools/solc/solc.py compile` と `tools/asm/asm.py` をランチャーと同じPythonで実行し、生成した `.wax` を起動します（通常GUI／monitor、既存エミュレーター指定のいずれにも対応）。起動設定タブの「solコンパイラ引数」に `-O speed --debug` のように入力すると、その引数をコンパイラへ渡します。空白で分割し、引用符内の空白はひとつの引数として扱います。空欄ならコンパイラ既定値です。設定は保存済みランチャープロファイルにも保持されます。ソースを元の絶対パスで渡すため、相対includeはソースの場所を基準に解決されます。アセンブリとバイナリは設定と同じ一時フォルダーに生成し、終了時に削除します。プロファイルには元の `.sol` パスを保存します。コンパイル／アセンブルの出力とエラーは起動ログに表示し、失敗した場合は起動しません。「停止」はコンパイル中にも使えます。

ランチャーはGUIの起動時に `config.yaml` を固定名で読むエミュレーターの仕様に合わせ、一時フォルダーに現在の設定を生成して実行します。相対のBIOS／font／disk／WASM package／sound asset pathは、選択したYAMLのある場所を基準に絶対化します。指定がなければdisk soundの探索先も選んだYAMLの場所になります。cargo projectの場所からビルドし、一時フォルダーを作業ディレクトリにするので、リポジトリ直下のconfig.yamlを書き換えずにプロファイルを切り替えられます。読み書き可能で起動中に変更しないdisk imageは元のファイルを直接使います。

「ROM・ディスク・拡張」の `PCG font 8x8` は `pcg.font`（2/4 KiB）、`PCG font 8x16` は `pcg.font_8x16`（4096 byte、256 glyph×16行、bit 7が左端）です。両方を独立して保存・復元でき、空欄ならそのキーを省略します。8×16は専用VRAMへの初期ロードだけで、表示モードはゲストが選択します。Foundation の `mode 8x16` も設定済みフォントを保持します。monitorではこれらのフォント設定を読み込みません。

「設定を保存」は画面に選択したYAMLへ変更を保存する明示操作です。「プロファイル保存」は設定内容とROM／CLI設定を、Windowsでは `%LOCALAPPDATA%\CPT32\Launcher\profiles.json`、その他の環境では `~/.config/CPT32/Launcher/profiles.json` に保存します。プロファイル名を入力して「読込」を押すと戻せます。

「CPT32PKG保存…」は保存先を選び、ROM欄のrawゲストバイナリ、有効なディスク4スロットとWASM拡張カード8スロットを、`manifest.yaml`付きのZIPへコピーします。未保存の画面設定を反映し、ディスクの種類と読み取り専用属性、カードのスロットを維持します。保存中とエミュレーター起動中はこのボタンを無効にします。保存失敗時は既存の出力を置き換えません。ROM選択や設定YAML、元のディスク／カードは書き換えません。

CPT32PKG v1の対象はアドレス0で実行するrawバイナリ（空でなく16 MiB以下）です。WAX・sol・既存CPT32PKGの変換、BIOSによるディスク起動の保存には対応しません。各ディスクは空でなく512 byte単位・64 MiB以下、各カードは64 MiB以下とし、カード自体の妥当性は起動時に既存のローダーが検証します。CPU方式、RAM／VRAM容量、BIOS、フォント、音楽、disk sound、ディスク速度や遅延、デバッグなどのホスト設定はパッケージに含みません。これらの設定も復元する場合は「プロファイル保存」を使ってください。タイトルには保存先のファイル名（拡張子を除く）を使い、保存するたびに新しいUUIDを割り当てます。そのため別のカートリッジとして扱われ、過去の保存時のCOW差分とは共有しません。書き込み可能なディスクの変更は起動後に外部COWへ保存されます。

PyYAMLなしの標準ライブラリfallbackは通常のmapping／sequence、引用文字列、数値、真偽値、null、JSON形式のflow collectionを扱います。YAML alias／tag／block stringなど対応していない拡張構文を見つけたときは、元ファイルを変更せずエラーを表示します。PyYAMLが利用できる環境ではその機能に従います。

`cargo`とRust toolchainをPATHから起動できるようにしてください。ディスクやWASM packageは選択した実ファイルを参照します。起動ログはウィンドウ下部に表示され、「停止」でemulator processを終了します。YAMLのコメントはランチャーの「設定を保存」で落とされますが、未知のYAMLキーは保持します。`RUST_LOG`を設定している環境では、エミュレーター本体の規則によりそちらがログレベル欄より優先されます。

GUIを開かずにビルド・起動処理の回帰テストを実行できます。

```sh
python -m unittest discover -s tools/cpt32-launcher -p 'test_*.py' -v
```
