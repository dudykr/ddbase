#!/usr/bin/env python3
"""Publish the completed, fixed-size SurrealDB experiment without rerunning it."""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import shutil
import statistics
import tarfile

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
VARIANTS = ("tokio", "goexec", "goexec-blocking")
LABELS = {"tokio": "Tokio", "goexec": "goexec / existing policy",
          "goexec-blocking": "goexec / handoff"}


def read_csv(path):
    with path.open() as stream:
        return list(csv.DictReader(stream))


def number(row, key):
    return float(row[key])


def geo(values):
    return math.exp(statistics.mean(math.log(v) for v in values))


def table(headers, rows):
    return "\n".join(["| " + " | ".join(headers) + " |",
                      "| " + " | ".join("---" for _ in headers) + " |"] +
                     ["| " + " | ".join(str(v) for v in row) + " |" for row in rows])


def validate(directory, expected_runs, expected_groups):
    rows = read_csv(directory / "runs.csv")
    latest = {r["run_id"]: r for r in rows}
    valid = [r for r in latest.values() if r["valid"] == "True"]
    assert len(valid) == expected_runs, (directory, len(valid), expected_runs)
    assert all(int(r["errors"]) == 0 and int(r["records"]) == 100000 and
               float(r["warmup_requested"]) == 3 and float(r["seconds_requested"]) == 10
               for r in valid)
    summary = read_csv(directory / "summary.csv")
    assert len(summary) == expected_groups
    assert all(int(r["repeats"]) == 5 for r in summary)
    return valid, summary, sum(r["valid"] != "True" for r in rows)


def group_key(row):
    return tuple(row[k] for k in ("backend", "workload", "parallelism", "lanes"))


def detailed(rows):
    output = []
    for r in sorted(rows, key=lambda r: (r["backend"], r["workload"],
                                         int(r["parallelism"]), int(r["lanes"]), r["variant"])):
        n = lambda name: number(r, name)
        output.append([r["backend"], r["workload"], r["parallelism"], r["lanes"],
                       LABELS[r["variant"]], f'{n("ops_per_second_median"):,.0f}',
                       f'{n("ops_per_second_min"):,.0f}–{n("ops_per_second_max"):,.0f}',
                       f'{n("throughput_vs_tokio"):.3f}×',
                       f'{n("p50_ns_median") / 1000:,.1f}', f'{n("p99_ns_median") / 1000:,.1f}',
                       f'{n("peak_rss_bytes_median") / 2**20:,.0f}',
                       f'{n("peak_threads_sampled_median"):g}', f'{n("handoffs_median"):,.0f}'])
    return table(["Backend", "Workload", "P", "Lanes", "Variant", "Median ops/s",
                  "Ops/s min–max", "vs Tokio", "p50 µs", "p99 µs", "Peak RSS MiB",
                  "Peak threads", "Handoffs"], output)


def publish(root, output):
    sdk_runs, sdk, sdk_invalid = validate(root / "results", 750, 150)
    core_runs, core, core_invalid = validate(root / "core-results", 150, 30)
    classic = read_csv(root / "classic/runs.csv")
    assert len(classic) == 150 and all(int(r["exit_code"]) == 0 for r in classic)
    verification_file = root / "verification/verification.json"
    if not verification_file.exists():
        verification_file = root / "verification.json"
    verified = json.loads(verification_file.read_text())
    assert len(verified) == 30 and all(r["passed"] for r in verified)
    env = json.loads((root / "results/environment.json").read_text())
    assert hashlib.sha256((HERE / "surrealdb.patch").read_bytes()).hexdigest() == env["patch_sha256"]
    output.mkdir(parents=True, exist_ok=True)
    for source, prefix in (("results", "sdk"), ("core-results", "core"), ("classic", "classic")):
        for filename in ("runs.csv", "summary.csv", "environment.json"):
            path = root / source / filename
            if path.exists():
                shutil.copy2(path, output / (prefix + "-" + filename))
    shutil.copy2(verification_file, output / "verification.json")
    # Preserve query responses, failures, and Criterion samples without large DBs.
    with tarfile.open(output / "process-logs.tar.gz", "w:gz") as archive:
        for suite in ("results", "core-results", "classic"):
            for path in sorted((root / suite).rglob("*")):
                if path.is_file() and (path.suffix in (".stdout", ".stderr") or
                                       path.name in ("stdout", "stderr", "result.json") or
                                       (path.parent.name == "new" and path.suffix == ".json")):
                    archive.add(path, arcname=str(path.relative_to(root)))
    baseline = {group_key(r): r for r in sdk if r["variant"] == "tokio"}
    aggregate = []
    widths = []
    extremes = []
    for backend in ("memory", "rocksdb"):
        for variant in VARIANTS[1:]:
            rows = [r for r in sdk if r["backend"] == backend and r["variant"] == variant]
            if not rows:
                continue
            ratios = [number(r, "throughput_vs_tokio") for r in rows]
            tails = [number(r, "p99_ns_median") / number(baseline[group_key(r)], "p99_ns_median") for r in rows]
            aggregate.append([backend, LABELS[variant], f"{geo(ratios):.3f}×",
                              f"{min(ratios):.3f}–{max(ratios):.3f}×", f"{geo(tails):.3f}×",
                              sum(r > 1.05 for r in ratios), sum(r < .95 for r in ratios), len(rows)])
            for p in (1, 4, 16):
                selected = [r for r in rows if int(r["parallelism"]) == p]
                widths.append([backend, LABELS[variant], p,
                               f'{geo([number(r, "throughput_vs_tokio") for r in selected]):.3f}×'])
            for label, row in (("lowest", min(rows, key=lambda r: number(r, "throughput_vs_tokio"))),
                               ("highest", max(rows, key=lambda r: number(r, "throughput_vs_tokio")))):
                extremes.append([backend, LABELS[variant], label, row["workload"], row["parallelism"],
                                 row["lanes"], f'{number(row, "throughput_vs_tokio"):.3f}×'])
    variability = []
    for variant in VARIANTS:
        rows = [r for r in sdk if r["variant"] == variant]
        spreads = [(number(r, "ops_per_second_max") - number(r, "ops_per_second_min")) /
                   number(r, "ops_per_second_median") for r in rows]
        variability.append([LABELS[variant], f"{statistics.median(spreads)*100:.1f}%", f"{max(spreads)*100:.1f}%"])
    paired = []
    sdk_by_key = {(r["variant"], *group_key(r)): r for r in sdk}
    for r in sorted(core, key=lambda r: (r["backend"], r["workload"], int(r["parallelism"]), r["variant"])):
        s = sdk_by_key[(r["variant"], *group_key(r))]
        paired.append([r["backend"], r["workload"], r["parallelism"], LABELS[r["variant"]],
                       f'{number(r, "ops_per_second_median"):,.0f}', f'{number(s, "ops_per_second_median"):,.0f}',
                       f'{number(s, "ops_per_second_median") / number(r, "ops_per_second_median"):.3f}×'])
    classic_groups = {}
    for r in classic:
        key = tuple(r[k] for k in ("variant", "backend", "workload", "parallelism"))
        classic_groups.setdefault(key, []).append(number(r, "mean_ns"))
    assert len(classic_groups) == 30 and all(len(v) == 5 for v in classic_groups.values())
    classic_table = []
    for key, values in sorted(classic_groups.items()):
        variant, backend, workload, p = key
        tokio = classic_groups[("tokio", backend, workload, p)]
        classic_table.append([backend, workload, p, LABELS[variant],
                              f"{statistics.median(values)/1e3:,.2f}",
                              f"{min(values)/1e3:,.2f}–{max(values)/1e3:,.2f}",
                              f"{statistics.median(tokio)/statistics.median(values):.3f}×"])
    all_runs = sdk_runs + core_runs
    log_failures = sdk_invalid + core_invalid
    document = f"""# SurrealDB embedded executor comparison — 2026-09-17

The completed comparison includes 750 SDK runs, 150 matched core runs, and
75 original core Criterion processes (150 read/create measurements), with five
fresh-process repetitions per condition. All 30 correctness configurations
passed. The main and matched suites recorded zero response errors in valid runs;
{log_failures} invalid attempts were retained and excluded.

SurrealDB v3.2.4: `{env['surrealdb_commit']}`.
Goexec [PR #107](https://github.com/dudykr/ddbase/pull/107) runtime commit:
`{env['goexec_commit']}`. Later PR commits only package tests, reproduction, and
results; the benchmark dependency uses this exact runtime commit.

## SDK results

Ratios divide five-run medians. Throughput above 1 is faster; p99 below 1 is
lower latency. The geometric means weight each of the 30 conditions per
backend/variant equally. They are summaries of this matrix, not estimates for
an arbitrary production workload. Counts above/below 5% are descriptive and
do not imply statistical significance.

{table(['Backend', 'Variant', 'Throughput geomean', 'Condition ratio range', 'p99 geomean', '>5% faster', '>5% slower', 'Conditions'], aggregate)}

Each width below averages the five workloads at both P and 4P concurrency.

{table(['Backend', 'Variant', 'P', 'Throughput / Tokio geomean'], widths)}

The lowest and highest observed median throughput ratios are both reported.

{table(['Backend', 'Variant', 'Extreme', 'Workload', 'P', 'Lanes', 'Throughput / Tokio'], extremes)}

Repeat variation is `(maximum - minimum) / median` within each condition.
Five repetitions are insufficient to establish a general speedup. Per-condition
min/max values, p50/p99, RSS, threads, and handoffs are in
[SDK details](sdk-details.md) and [the summary CSV](sdk-summary.csv).

{table(['Variant', 'Median repeat spread', 'Largest repeat spread'], variability)}

## SDK versus matched core

These read/create cases use identical 100,000-record data, SQL, timing, and 4P
lanes. Core calls `Datastore::execute`; SDK calls traverse the native router,
spawned query task, and SDK response conversion. SDK membership tasks also run.
The suites execute consecutively, so host drift can affect this comparison.
This measures the combined access-path cost, not an isolated instruction-level
router cost.

{table(['Backend', 'Workload', 'P', 'Variant', 'Core ops/s', 'SDK ops/s', 'SDK / core'], paired)}

See [core details](core-details.md), [raw runs](core-runs.csv), and
[summary](core-summary.csv).

## Original core read/create routines

These are SurrealDB's original `sdb.rs` routines, adapted only for executor
selection, owned roots, and the actual datastore worker width. They retain
their small random records and original read predicate. They are separate
from the fixed-seed ~1 KiB workloads above. Values below are medians of five
Criterion mean estimates. The original read predicate compares the stored
field to a newly generated random value and normally returns an empty result;
the main/matched point reads require an existing, nonempty record. Values are
in microseconds per operation. The upstream routine
divides each 1,000-operation batch's elapsed time by 1,000; this is amortized
parallel batch time, not individual request latency. The speed ratio is Tokio
time divided by variant time. Criterion uses a 3-second
warmup, a 10-second requested measurement, 30 samples, and its own calibration;
slow cases can exceed the requested measurement duration.
The upstream builder defaults are preserved across variants: sync every and
RocksDB background threads equal to available CPUs. Inline capacity still uses
the actual P, and the affinity pool is max(4,P). Tokio retains its default 512
blocking threads plus P workers, while goexec caps workers at 512. These limits
differ from the main/matched harness and are included in the archived metadata.

{table(['Backend', 'Workload', 'P', 'Variant', 'Median µs/op', 'µs/op min–max', 'Speed / Tokio'], classic_table)}

## Reproduction and limits

- Host: `{env['hardware'].replace(chr(10), '; ')}`; `{env['platform']}`.
- Rust 1.95.0; release opt-level 3, LTO off, 16 codegen units, panic abort,
  system allocator. Binary hashes and full compiler versions are in
  [SDK environment](sdk-environment.json), [core environment](core-environment.json),
  and [original core environment](classic-environment.json).
- Main suites: fixed seed 20260917, 100,000 records, 960 deterministic
  alphanumeric payload bytes per record, 3-second warmup, 10-second timed
  phase, five fresh processes. Requests are closed-loop; latency includes
  submission, queueing, execution, and response extraction. SQL/payload string
  construction is excluded from latency but contributes to throughput.
- Workloads: point read, create, version update, 80/20 read/update mix, indexed
  32-record range. Write lanes own separate keys. Initial counts, create counts,
  and response errors are checked. Measured intervals exclude setup/shutdown.
- Main/matched storage: sync every, RocksDB threads P, affinity pool max(4,P), Rayon P,
  reserve two workers. The goexec existing-policy variant retains original
  inline/offload dispatch. Handoff replaces only InlineGuard dispatch;
  explicit count/compaction affinity jobs retain the original pools.
- Main/matched worker widths are 1, 4, 16. Both executors cap their worker/blocking threads
  at 512; goexec monitor/timer and native storage/CPU threads are additional.
  Effective datastore inline capacity is asserted for SDK and core.
- No Tokio runtime is constructed in goexec runs. Tokio runtime-independent
  primitives remain. Native networking, remote stores, scripting, ML, WASM,
  extension runtimes, bucket/file APIs, and SDK import/export are unvalidated.
- Peak RSS is process-lifetime maximum, including setup and teardown. Create
  runs insert different totals depending on speed, so their memory is not an
  equal-row-count executor-overhead comparison. Threads are sampled every
  20 ms; handoffs are deltas only across the measured phase.
- This is a warm-cache local Mac experiment, without cache flushing, CPU
  affinity, or exclusive host isolation. Variants run serially in seeded
  shuffled blocks; no compiler/test/other benchmark ran concurrently. It does
  not characterize cold disk, a server/network workload, or open-loop overload.

The [patch](../../surrealdb/surrealdb.patch), [runner](../../surrealdb/run.py),
and [commands](../../surrealdb/README.md) reproduce the experiment. The patch
was applied to a fresh pinned checkout and all 46 changed files matched the
tested source byte-for-byte. Patch SHA-256: `{env['patch_sha256']}`.

Validation includes CRUD, rollback, forced-index lookup, SLEEP/query timeout,
external cancellation, SDK background shutdown, and RocksDB reopen persistence
across 30 configurations; see [verification.json](verification.json). A separate
150-condition short smoke matrix passed before these measurements. Goexec
passed fmt, clippy, default/time-feature tests on pinned nightly and Rust 1.95,
plus six Loom models; CI also passed on Linux, macOS, and Windows. Timer tests
cover cancellation, reset, concurrent waits, missed ticks, shutdown, budget
fairness, and the inability to preempt a synchronous poll.

Raw data: [SDK CSV](sdk-runs.csv), [matched core CSV](core-runs.csv),
[original core CSV](classic-runs.csv), and [process logs and Criterion samples](process-logs.tar.gz).
Total successful requests in the main and matched timed phases:
{sum(int(r['operations']) for r in all_runs):,}.
"""
    (output / "README.md").write_text(document)
    for name, rows in (("sdk", sdk), ("core", core)):
        (output / (name + "-details.md")).write_text(
            "# " + name.upper() + " condition details\n\n"
            "Five-run medians unless the column says min–max. Peak RSS/threads include setup and shutdown; handoffs cover only the measured phase.\n\n" + detailed(rows) + "\n")
    checksums = [hashlib.sha256(path.read_bytes()).hexdigest() + "  " + path.name
                 for path in sorted(output.iterdir()) if path.is_file() and path.name != "SHA256SUMS"]
    (output / "SHA256SUMS").write_text("\n".join(checksums) + "\n")
    print(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=REPO / "target/surrealdb-integration")
    parser.add_argument("--output", type=Path, default=HERE.parent / "results/2026-09-17-surrealdb")
    args = parser.parse_args()
    publish(args.input.resolve(), args.output.resolve())
