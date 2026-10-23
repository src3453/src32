# 謝辞・ライセンス

CPT32 は多数のオープンソースプロジェクトに支えられています。各プロジェクトの作者・保守者の皆様に感謝します。

- 描画・ウィンドウ・UI: wgpu、winit、Dear ImGui、imgui-rs、imgui-wgpu、imgui-winit-support、stb。
- 音声・入力: CPAL、gilrs。
- WASM・JIT: Wasmtime、Cranelift、Bytecode Alliance の各プロジェクト。
- データ・実行基盤: serde、yaml_serde、Tokio、rand、zip、directories、sha2、crc32c、tracing、tracing-subscriber と、それらの推移依存。
- フォント: UNSCII（Viznut と協力者）、GNU Unifont、Source Sans Pro（Adobe）。
- ゲストの浮動小数点サンプルと llvm-src32 の浮動小数点ランタイム: Berkeley SoftFloat（John R. Hauser / The Regents of the University of California）。
- sol VS Code拡張: Microsoft の VS Code Language Client / Language Server Protocol ライブラリと関連パッケージ。

上記は主要プロジェクトの紹介です。バージョンを含む一覧、著作権表示、許諾・免責の全文は [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt) を参照してください。エミュレータの `Help → About / Third-party licenses` からも読めます。

CPT32 自作部分は [MIT License](LICENSE) です。第三者コンポーネントには各条件が適用されます。[確認結果と残る確認事項](doc/license-compliance.md) も参照してください。
