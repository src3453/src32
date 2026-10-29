# Foundation から FPGA CPU 性能カウンタを読む

- FPGA CPU perf view は `0xFFFF02A0..0xFFFF02DF` にあり、Foundation の U-mode アプリから直接読む権限を与えない。`sys_info(6)` を追加し、カーネルが8個の64-bit値を低位・高位word順で返す。
- カウンタは SoC 全体の累積値なので、測定は前後snapshotの unsigned 64-bit 差分を使う。カウンタ読み出しsyscall、割り込み、並行サービスの実行サイクルも区間に含む。
- `foundation/tests/perf/bench_cpu.c` はALU、ロード、ストアの各workloadを実行し、8カウンタ差分と `INSTRET / CYCLE` を出す。Tang Console用 image は `foundation/boot/build.py ... --bin-dir build/bench-cpu` でベンチを `/bin` に含める。
- ベンチWAXと Foundation image のビルドは成功したが、2026-10-28 の作業環境では既存の両Gowin版 `gw_sh` が即時終了して bitstream を作れず、実機計測は未完了。ツールクラッシュの原因調査はCPUカウンタ実装の範囲外。
- 確認コマンド: LLVM SRC32 driverで `bench_cpu.c` をビルドし、`foundation/boot/build.py target/foundation-cpu-perf --platform tang-console --root-drive ramdisk --clang 'C:/Program Files/LLVM/bin/clang.exe' -j 6 --bin-dir build/bench-cpu` を実行する。FPGA対応RTLを書き込んだ後、`uart_load.py COM7 --foundation target/foundation-cpu-perf/fpga-boot.json` と `fnd_cmd.py COM7 bench_cpu` で測定する。
