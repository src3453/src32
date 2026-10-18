# CPT32 GUI ランチャー

Windows／Linuxで使えるPython 3.10+・Tkinter製ランチャーです。追加Pythonパッケージなしで動作し、PyYAMLが入っていれば一般的なYAML構文も使えます。

```powershell
python tools/cpt32-launcher/launcher.py
```

ROM／WAX／CPT32PKG、CPU方式、通常GUI／monitor、デバッグGUI、profile、music stream、BIOS ROM、PCG font、FDD／HDDイメージ（4スロット）、WASM expansion card（8スロット）、IDC disk sound asset rootを設定できます。`pipeline`、`cached`、`cached-relaxed`、`jit`、`scalar`に対応し、JITを選ぶとCargo起動に`--features jit`を追加します。既存実行ファイルを指定してビルドせずに起動することもできます。

ランチャーはGUIの起動時に `config.yaml` を固定名で読むエミュレーターの仕様に合わせ、一時フォルダーに現在の設定を生成して実行します。相対のBIOS／font／disk／WASM package／sound asset pathは、選択したYAMLのある場所を基準に絶対化します。指定がなければdisk soundの探索先も選んだYAMLの場所になります。cargo projectの場所からビルドし、一時フォルダーを作業ディレクトリにするので、リポジトリ直下のconfig.yamlを書き換えずにプロファイルを切り替えられます。読み書き可能で起動中に変更しないdisk imageは元のファイルを直接使います。

「設定を保存」は画面に選択したYAMLへ変更を保存する明示操作です。「プロファイル保存」は設定内容とROM／CLI設定を、Windowsでは `%LOCALAPPDATA%\CPT32\Launcher\profiles.json`、その他の環境では `~/.config/CPT32/Launcher/profiles.json` に保存します。プロファイル名を入力して「読込」を押すと戻せます。

PyYAMLなしの標準ライブラリfallbackは通常のmapping／sequence、引用文字列、数値、真偽値、null、JSON形式のflow collectionを扱います。YAML alias／tag／block stringなど対応していない拡張構文を見つけたときは、元ファイルを変更せずエラーを表示します。PyYAMLが利用できる環境ではその機能に従います。

`cargo`とRust toolchainをPATHから起動できるようにしてください。ディスクやWASM packageは選択した実ファイルを参照します。起動ログはウィンドウ下部に表示され、「停止」でemulator processを終了します。YAMLのコメントはランチャーの「設定を保存」で落とされますが、未知のYAMLキーは保持します。`RUST_LOG`を設定している環境では、エミュレーター本体の規則によりそちらがログレベル欄より優先されます。
