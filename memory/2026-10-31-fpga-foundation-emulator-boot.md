# FPGA 向け Foundation イメージとエミュレータ起動

- Tang Console 向け `fpga-boot.json` の `root_drive: 1` は、`foundation.img` を FPGA の RAM ディスク `0x20000000` に別途配置する契約であり、エミュレータの IDC slot 0 に同じイメージを接続するだけでは満たせない。
- 2026-10-31 の事例ではディスク内カーネルが生成済み `kernel.bin` と一致し、panic EPC `0x00022E38` は `LD R1, [R3]` だった。表示される last saved context は障害時のレジスタとは限らないため、アクセス先はログだけから断定しない。
- エミュレータ向け Foundation は `python foundation/run.py --disk` で生成・起動できる。ディスク起動経路の回帰確認には `python -m unittest discover -s foundation/tests/disk -p test_disk_boot.py -k test_boots_the_shell_from_disk -v` を使う。Windows のサンドボックスでは TEMP/TMP を書き込み可能な作業ツリー内に設定する。
- FPGA イメージの元ディスクを変更しないよう、調査時は複製を使う。エミュレータで Tang の RAM ディスクを実際に再現する場合は、別途メモリ配置とデバイス契約を検証する必要がある。
