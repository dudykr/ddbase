#!/usr/bin/env python3
"""Build and serially measure goexec/Go, optionally against an older harness.

Uses only Python's standard library. Run from any directory; outputs raw CSV,
environment metadata, and a median/range summary. No timing assertions.
"""

import argparse
import csv
from datetime import datetime, timezone
import hashlib
import io
import json
import math
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess


ROOT = Path(__file__).resolve().parents[3]
OPERATIONS = {
    "Yield": 1_000_000,
    "Empty": 1_000_000,
    "CachedRead": 100_000,
    "Cpu": 100_000,
    "SyscallWait": 4_096,
    "Mixed": 8_192,
}


def run(command, **kwargs):
    return subprocess.run(command, cwd=ROOT, check=True, text=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/goexec-comparison/results")
    parser.add_argument("--rust-binary", type=Path, help="use an already built compare harness")
    parser.add_argument("--baseline", type=Path, help="older compare harness with the same CLI")
    parser.add_argument("--go-binary", type=Path, help="use an already built Go companion")
    parser.add_argument("--engines", help="comma-separated goexec,go,baseline (default: all available)")
    parser.add_argument("--parallelism", default="1,4,16")
    parser.add_argument("--cases", default=",".join(OPERATIONS))
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--scale", type=float, default=1.0, help="scale each workload's operation count")
    parser.add_argument("--latency", action="store_true", help="collect per-operation p50/p99 as well")
    args = parser.parse_args()
    parallelisms = [int(p) for p in args.parallelism.split(",")]
    cases = args.cases.split(",")
    if args.repeats <= 0 or args.scale <= 0 or any(p <= 0 for p in parallelisms):
        parser.error("parallelism, repeats and scale must be positive")
    if any(case not in OPERATIONS for case in cases):
        parser.error("unknown case")
    selected = args.engines.split(",") if args.engines is not None else None
    if selected is not None:
        if any(label not in ("goexec", "go", "baseline") for label in selected):
            parser.error("unknown engine")
        if "baseline" in selected and args.baseline is None:
            parser.error("the baseline engine requires --baseline")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    rust_binary = args.rust_binary
    if rust_binary is None:
        build = run([
            "cargo", "bench", "-p", "goexec", "--locked", "--bench", "compare",
            "--no-run", "--message-format=json",
        ], stdout=subprocess.PIPE)
        artifacts = [json.loads(line) for line in build.stdout.splitlines()]
        rust_binary = Path(next(a["executable"] for a in artifacts
                                if a.get("target", {}).get("name") == "compare" and a.get("executable")))
    rust_binary = rust_binary.resolve()
    go_binary = args.go_binary
    if go_binary is None:
        go_binary = output / "go-scheduler"
        run(["go", "build", "-o", str(go_binary), str(ROOT / "crates/goexec/benches/go/main.go")])
    go_binary = go_binary.resolve()
    metadata = {
        "started_at": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(), "cpu_count": os.cpu_count(),
        "go": run(["go", "version"], stdout=subprocess.PIPE).stdout.strip(),
        "rustc": run(["rustc", "--version"], stdout=subprocess.PIPE).stdout.strip(),
        "revision": run(["git", "rev-parse", "HEAD"], stdout=subprocess.PIPE).stdout.strip(),
        "working_tree": run(["git", "status", "--short"], stdout=subprocess.PIPE).stdout,
        "options": {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()},
        "environment": {key: os.environ.get(key) for key in
                        ("GOGC", "GOMEMLIMIT", "GODEBUG", "GOEXPERIMENT", "RUSTFLAGS")},
        "order_seed": 0,
        "binaries_sha256": {
            label: hashlib.sha256(binary.read_bytes()).hexdigest()
            for label, binary in [("goexec", rust_binary), ("go", go_binary)]
            + ([("baseline", args.baseline.resolve())] if args.baseline else [])
        },
    }
    (output / "environment.json").write_text(json.dumps(metadata, indent=2) + "\n")
    engines = [("goexec", rust_binary, "Goexec"), ("go", rust_binary, "Go")]
    if args.baseline:
        engines.append(("baseline", args.baseline.resolve(), "Goexec"))
    if selected is not None:
        engines = [engine for engine in engines if engine[0] in selected]
    rng = random.Random(0)
    rows = []
    with (output / "runs.csv").open("w", newline="") as raw, (output / "progress.log").open("w") as log:
        writer = None
        for repeat in range(1, args.repeats + 1):
            for p in parallelisms:
                lanes = 4 * p
                for case in cases:
                    iterations = max(1, math.ceil(OPERATIONS[case] * args.scale / lanes))
                    order = engines.copy()
                    rng.shuffle(order)
                    for label, binary, mode in order:
                        print(f"repeat={repeat} P={p} {case} {label}", flush=True)
                        command = [str(binary), "--go-binary", str(go_binary), "--modes", mode,
                                   "--cases", case, "--parallelism", str(p), "--lanes", str(lanes),
                                   "--iterations", str(iterations)]
                        if not args.latency:
                            command.append("--no-latency")
                        result = run(command, stdout=subprocess.PIPE, stderr=log, timeout=180)
                        records = list(csv.DictReader(io.StringIO(result.stdout)))
                        steady = [r for r in records if r["phase"] == "steady"]
                        if len(steady) != 1 or int(steady[0]["operations"]) != lanes * iterations:
                            raise RuntimeError(f"invalid benchmark output: {result.stdout}")
                        row = {"label": label, "parallelism": p, "lanes": lanes, "repeat": repeat, **steady[0]}
                        if writer is None:
                            writer = csv.DictWriter(raw, fieldnames=list(row), lineterminator="\n")
                            writer.writeheader()
                        writer.writerow(row)
                        raw.flush()
                        rows.append(row)
    with (output / "summary.csv").open("w", newline="") as summary:
        writer = csv.writer(summary, lineterminator="\n")
        writer.writerow(["parallelism", "workload", "label", "median_ops_per_sec", "min_ops_per_sec", "max_ops_per_sec"])
        for p in parallelisms:
            for case in cases:
                for label, _, _ in engines:
                    rates = [float(r["ops_per_sec"]) for r in rows
                             if r["parallelism"] == p and r["workload"] == case and r["label"] == label]
                    writer.writerow([p, case, label, statistics.median(rates), min(rates), max(rates)])
    print(f"Results: {output}")


if __name__ == "__main__":
    main()
