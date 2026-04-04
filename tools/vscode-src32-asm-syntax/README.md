# SRC16 Assembly Syntax for VS Code

SRC16 CPUエミュレータ向けアセンブリ言語のシンタックスハイライト拡張です。

## 対応内容

- 命令: `NOP`, `MOV`, `LD`, `ST`, `LDI`, `ADD`, `SUB`, `JMP`, `JZ`, `JNZ`, `PUSH`, `POP`, `CALL`, `RET`, `INT`, `IRET`, `EI`, `DI`, `AND`, `OR`, `XOR`, `SHL`, `SHR`, `CMP`, `TEST`, `NOT`, `INC`, `DEC`, `NEG`, `PUSHI`, `JR`, `JZR`, `JNZR`, `JC`, `JNC`, `JRI`, `LEA`, `STI`, `LDB`, `STB`, `CPUID`, `RESET`, `HALT`
- ディレクティブ: `.ORG`, `.WORD`, `.BYTE`, `.DB`, `.STRING`
- レジスタ: `R0`-`R15`, `SP`, `PC`, `FLAG`
- 数値リテラル: 10進, `0x`16進, `0b`2進
- ラベル定義/参照, 文字列, `;`コメント

## 拡張子

- `.a`
- `.asm`

## ローカルで試す

1. このフォルダをVS Codeで開く
2. `F5`でExtension Development Hostを起動
3. 新しいウィンドウで`.a`または`.asm`ファイルを開く

## VSIXとしてパッケージ

```bash
npm install -g @vscode/vsce
vsce package
```

生成された`.vsix`をVS Codeへインストールして利用できます。
