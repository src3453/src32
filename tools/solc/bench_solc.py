"""Measure solc output size and SRC32 execution cycles on fixed workloads."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools" / "asm"))
sys.path.insert(0, str(ROOT / "tools" / "solc"))

from asm import Assembler
from sol_compiler import compile_to_src32_asm


BASELINE_PATH = Path(tempfile.gettempdir()) / "solc-benchmark-baseline.json"
RUNNER_PATHS = (ROOT / "target" / "debug" / "src32_testbench.exe", ROOT / "target" / "debug" / "src32_testbench")
WORKLOADS = {
    "arithmetic-loop": ("0 while dup 1 add swap drop dup 10 lt end", 10),
    "comparison": ("7 3 le 7 3 ge add", 1),
    "stack-operations": ("1 2 swap add dup mul", 9),
    "function-memory": ("fn bump (x) : local y x 1 add >y y ret ; 41 bump", 42),
}


def runner_path() -> Path:
    for path in RUNNER_PATHS:
        if path.is_file():
            return path
    raise RuntimeError("src32_testbench is not built; run cargo build --bin src32_testbench")


def measure() -> dict:
    runner = runner_path()
    results = {}
    for profile, use_short_mode in (("size", True), ("speed", False)):
        for name, (source, expected) in WORKLOADS.items():
            assembly = compile_to_src32_asm(source, use_short_mode=use_short_mode)
            binary = Assembler().assemble(assembly)
            with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as image:
                image.write(binary)
                image_path = Path(image.name)
            try:
                process = subprocess.run(
                    [str(runner), str(image_path), str(expected)],
                    check=False,
                    capture_output=True,
                    text=True,
                    timeout=15,
                )
            finally:
                image_path.unlink(missing_ok=True)
            output = process.stdout + process.stderr
            if process.returncode:
                raise RuntimeError(f"{profile}/{name}: runner failed ({process.returncode}): {output.strip()}")
            match = re.search(r"PASS: .* => R1=(\d+) cycles=(\d+)", process.stdout)
            if not match:
                raise RuntimeError(f"{profile}/{name}: missing R1/cycles runner output: {output.strip()}")
            actual, cycles = map(int, match.groups())
            if actual != expected:
                raise RuntimeError(f"{profile}/{name}: R1={actual}, expected {expected}")
            results[f"{profile}/{name}"] = {
                "profile": profile,
                "workload": name,
                "bytes": len(binary),
                "cycles": cycles,
                "expected_r1": expected,
                "actual_r1": actual,
            }
    return results


def print_results(results: dict, baseline: dict | None = None) -> None:
    print(f"{'profile':<8} {'workload':<20} {'bytes':>8} {'cycles':>10} {'R1':>8} {'Δ bytes':>10} {'Δ cycles':>10}")
    for key, result in results.items():
        old = baseline.get(key) if baseline else None
        byte_delta = result["bytes"] - old["bytes"] if old else 0
        cycle_delta = result["cycles"] - old["cycles"] if old else 0
        suffix = f"{byte_delta:+d}" if old else "—"
        cycle_suffix = f"{cycle_delta:+d}" if old else "—"
        print(f"{result['profile']:<8} {result['workload']:<20} {result['bytes']:>8} {result['cycles']:>10} {result['actual_r1']:>8} {suffix:>10} {cycle_suffix:>10}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--record-baseline", action="store_true")
    mode.add_argument("--compare-baseline", action="store_true")
    args = parser.parse_args()

    results = measure()
    if args.record_baseline:
        BASELINE_PATH.write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
        print(f"Baseline saved: {BASELINE_PATH}")
    baseline = None
    if args.compare_baseline:
        if not BASELINE_PATH.is_file():
            raise RuntimeError(f"baseline not found: {BASELINE_PATH}")
        baseline = json.loads(BASELINE_PATH.read_text(encoding="utf-8"))
        if set(baseline) != set(results):
            raise RuntimeError("baseline workload/profile set does not match current measurements")
        improvements = {"size": False, "speed": False}
        regressions = []
        for key, result in results.items():
            old = baseline[key]
            metric = "bytes" if result["profile"] == "size" else "cycles"
            if result[metric] > old[metric]:
                regressions.append(f"{key}: {metric} {result[metric]} > baseline {old[metric]}")
            if result[metric] < old[metric]:
                improvements[result["profile"]] = True
        if regressions:
            raise RuntimeError("benchmark profile regression: " + "; ".join(regressions))
        if not all(improvements.values()):
            raise RuntimeError("benchmark must improve size bytes and speed cycles in at least one workload")
    print_results(results, baseline)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
