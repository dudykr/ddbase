#!/usr/bin/env python3
"""Reproduce the pinned native embedded SurrealDB executor comparison.

Requires Python 3.9+, psutil==7.0.0, git, and Rust 1.95.0. All measured processes
run serially. No server, network client, or Tokio runtime is used in goexec mode.
"""
import argparse
import csv
import hashlib
import itertools
import json
import os
from pathlib import Path
import platform
import random
import resource
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
UPSTREAM = "93ab219d69f09d8f999851b0359c80ebe6726102"
GOEXEC = "e57e96903c7c33a1c38fb34ee3c2ff11ed579145"
VARIANTS = {"tokio": [], "goexec": ["runtime-goexec"], "goexec-blocking": ["goexec-blocking"]}
FIELDS = ["run_id", "binary_sha256", "variant", "backend", "access", "workload", "parallelism", "lanes", "records", "seed", "repeat", "warmup_requested", "seconds_requested", "valid", "exit_code", "error", "setup_seconds", "warmup_ops", "operations", "seconds", "ops_per_second", "p50_ns", "p99_ns", "errors", "handoffs", "rocksdb_granted", "rocksdb_diverted", "peak_rss_bytes", "peak_threads_sampled", "cpu_user_seconds", "cpu_system_seconds", "wall_seconds"]


def command(args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def env_for(p):
    env = os.environ.copy()
    # Remove inherited Surreal tuning before applying the shared configuration.
    for key in list(env):
        if key.startswith("SURREAL_"):
            del env[key]
    env.update(RUSTC_WRAPPER="", RUST_LOG="off", RAYON_NUM_THREADS=str(p),
               SURREAL_KVS_THREADPOOL_SIZE=str(max(4, p)),
               SURREAL_RUNTIME_WORKER_THREADS=str(p), SURREAL_ROCKSDB_THREAD_COUNT=str(p),
               SURREAL_ROCKSDB_RUNTIME_RESERVE="2", SURREAL_DATASTORE_SYNC="every")
    return env


def prepare(source):
    if not source.exists():
        source.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "clone", "--depth", "1", "--branch", "v3.2.4", "https://github.com/surrealdb/surrealdb.git", str(source)], check=True)
    if command(["git", "rev-parse", "HEAD"], cwd=source) != UPSTREAM:
        raise RuntimeError("source must be pinned to SurrealDB v3.2.4")
    patch = HERE / "surrealdb.patch"
    if subprocess.run(["git", "apply", "--reverse", "--check", str(patch)], cwd=source, capture_output=True).returncode == 0:
        return
    subprocess.run(["git", "apply", "--check", str(patch)], cwd=source, check=True)
    subprocess.run(["git", "apply", str(patch)], cwd=source, check=True)


def build(source, bins, variants):
    bins.mkdir(parents=True, exist_ok=True)
    for variant in variants:
        args = ["cargo", "+1.95.0", "build", "--locked", "-p", "surreal-runtime-bench", "--release", "--config", "profile.release.lto=false", "--config", "profile.release.codegen-units=16"]
        if VARIANTS[variant]:
            args += ["--features", ",".join(VARIANTS[variant])]
        subprocess.run(args, cwd=source, env=env_for(4), check=True)
        shutil.copy2(source / "target/release/surreal-runtime-bench", bins / variant)
        shutil.copy2(source / "target/release/native-sdb", bins / (variant + "-sdb"))


def capture(binary, argv, env, stdout, stderr, timeout=600, cwd=None):
    import psutil
    started = time.monotonic()
    peak_threads = 0
    with stdout.open("w") as out, stderr.open("w") as err:
        child = subprocess.Popen([str(binary)] + argv, stdout=out, stderr=err, env=env, cwd=cwd)
        process = psutil.Process(child.pid)
        killed = False
        while True:
            pid, status, usage = os.wait4(child.pid, os.WNOHANG)
            if pid:
                child.returncode = os.waitstatus_to_exitcode(status)
                break
            try:
                peak_threads = max(peak_threads, process.num_threads())
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                pass
            if time.monotonic() - started > timeout and not killed:
                child.kill()
                killed = True
            time.sleep(0.02)
    report = {}
    for line in stdout.read_text().splitlines():
        try:
            value = json.loads(line)
            if isinstance(value, dict) and value.get("kind") in ("benchmark", "verification"):
                report = value
        except json.JSONDecodeError:
            pass
    rss = usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024)
    return report, {"exit_code": child.returncode, "peak_rss_bytes": rss,
                    "peak_threads_sampled": peak_threads, "cpu_user_seconds": usage.ru_utime,
                    "cpu_system_seconds": usage.ru_stime, "wall_seconds": time.monotonic() - started,
                    "error": "process watchdog expired" if killed else stderr.read_text()[-4000:] if child.returncode else ""}


def metadata(source, bins, output, variants, args):
    output.mkdir(parents=True, exist_ok=True)
    file = output / "environment.json"
    binaries = {v: sha256(bins / v) for v in variants}
    if file.exists():
        previous = json.loads(file.read_text())
        if previous["binaries"] != binaries:
            raise RuntimeError("refusing to mix binaries; use a new output directory")
        return binaries
    data = {"surrealdb_commit": UPSTREAM, "goexec_commit": GOEXEC,
            "goexec_pr": "https://github.com/dudykr/ddbase/pull/107",
            "platform": platform.platform(), "machine": platform.machine(),
            "rustc": command(["rustc", "+1.95.0", "-Vv"]), "python": sys.version,
            "build": "release, opt-level=3, lto=false, codegen-units=16, panic=abort, system allocator",
            "binaries": binaries, "argv": sys.argv, "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            "thread_sampling_seconds": 0.02,
            "memory_scope": "OS-reported process lifetime maximum RSS, including setup and teardown",
            "latency_scope": "closed-loop request submission through checked response extraction; query string generation excluded",
            "temperature": "warm cache; no OS page-cache flushing",
            "payload": "960 deterministic alphanumeric bytes per record, derived from record ID and fixed seed",
            "storage": "sync=every; RocksDB threads=P; KVS pool=max(4,P); Rayon=P; inline reserve=2",
            "runtime": "worker parallelism=P; executor thread cap=512; goexec monitor/timer and storage pools are additional threads",
            "timing": {"records": args.records, "warmup": args.warmup, "seconds": args.seconds, "repeats": args.repeats, "seed": args.seed}}
    if sys.platform == "darwin":
        data["hardware"] = command(["sysctl", "machdep.cpu.brand_string", "hw.ncpu", "hw.memsize"])
    if (HERE / "surrealdb.patch").exists():
        data["patch_sha256"] = sha256(HERE / "surrealdb.patch")
    file.write_text(json.dumps(data, indent=2) + "\n")
    return binaries


def run_matrix(args, source, bins, output):
    variants = args.variants.split(",")
    fingerprints = metadata(source, bins, output, variants, args)
    logs = output / "logs"
    logs.mkdir(exist_ok=True)
    existing = {}
    csvfile = output / "runs.csv"
    if csvfile.exists():
        with csvfile.open() as f:
            existing = {r["run_id"]: r for r in csv.DictReader(f) if r["valid"] == "True"}
    blocks = list(itertools.product(args.backends.split(","), args.parallelism, args.factors,
                                    args.workloads.split(","), range(args.repeats), args.access.split(",")))
    rng = random.Random(args.seed)
    rng.shuffle(blocks)
    jobs = []
    for backend, p, factor, workload, repeat, access in blocks:
        order = [v for v in variants if not (v == "goexec-blocking" and backend == "memory")]
        rng.shuffle(order)
        for v in order:
            key = f"{v}-{backend}-{access}-{workload}-p{p}-c{p*factor}-r{repeat}-n{args.records}-w{args.warmup}-s{args.seconds}-seed{args.seed}"
            jobs.append((key, v, backend, p, p * factor, workload, repeat, access))
    completed = 0
    for key, variant, backend, p, lanes, workload, repeat, access in jobs:
        if key in existing:
            completed += 1
            continue
        with tempfile.TemporaryDirectory(prefix="surreal-", dir=output) as dbdir:
            argv = ["--backend", backend, "--path", str(Path(dbdir) / "db"), "--access", access,
                    "--workload", workload, "--parallelism", str(p), "--lanes", str(lanes),
                    "--records", str(args.records), "--warmup", str(args.warmup), "--seconds", str(args.seconds), "--seed", str(args.seed)]
            report, process = capture(bins / variant, argv, env_for(p), logs / (key + ".stdout"), logs / (key + ".stderr"))
        row = dict(run_id=key, binary_sha256=fingerprints[variant], variant=variant, backend=backend,
                   access=access, workload=workload, parallelism=p, lanes=lanes, records=args.records,
                   seed=args.seed, repeat=repeat, warmup_requested=args.warmup, seconds_requested=args.seconds)
        row.update({k: v for k, v in report.items() if k in FIELDS})
        row.update(process)
        row["valid"] = process["exit_code"] == 0 and report.get("errors") == 0 and report.get("operations", 0) > 0
        write_header = not csvfile.exists()
        with csvfile.open("a", newline="") as f:
            writer = csv.DictWriter(f, fieldnames=FIELDS)
            if write_header:
                writer.writeheader()
            writer.writerow(row)
        completed += 1
        print(json.dumps({"completed": completed, "total": len(jobs), "run": key, "valid": row["valid"], "ops_per_second": row.get("ops_per_second"), "elapsed_seconds": round(process["wall_seconds"], 2)}), flush=True)
        if not row["valid"]:
            raise RuntimeError(f"invalid run {key}: {process['error']}")
    summarize(output)


def verify(args, bins, output):
    output.mkdir(parents=True, exist_ok=True)
    results = []
    for variant, backend, access, p in itertools.product(args.variants.split(","), args.backends.split(","), args.access.split(","), args.parallelism):
        if variant == "goexec-blocking" and backend == "memory":
            continue
        key = f"verify-{variant}-{backend}-{access}-p{p}"
        with tempfile.TemporaryDirectory(prefix="verify-", dir=output) as dbdir:
            report, process = capture(bins / variant, ["--verify", "--backend", backend, "--path", str(Path(dbdir) / "db"), "--access", access, "--parallelism", str(p)], env_for(p), output / (key + ".stdout"), output / (key + ".stderr"), timeout=120)
        result = dict(variant=variant, backend=backend, access=access, parallelism=p, binary_sha256=sha256(bins / variant))
        result.update(process)
        result.update(report)
        results.append(result)
        (output / "verification.json").write_text(json.dumps(results, indent=2) + "\n")
        print(json.dumps(result), flush=True)
        if process["exit_code"] or not report.get("passed"):
            raise RuntimeError(f"verification failed: {key}")


def summarize(output):
    with (output / "runs.csv").open() as f:
        rows = list(csv.DictReader(f))
    groups = {}
    keys = ["variant", "backend", "access", "workload", "parallelism", "lanes", "records"]
    # A rerun supersedes an earlier row for the same run ID; invalid attempts are
    # retained in raw CSV, and never counted in summaries.
    latest = {r["run_id"]: r for r in rows}
    for row in latest.values():
        if row["valid"] != "True":
            continue
        groups.setdefault(tuple(row[k] for k in keys), []).append(row)
    summaries = []
    for key, samples in sorted(groups.items()):
        row = dict(zip(keys, key))
        row["repeats"] = len(samples)
        for metric in ["ops_per_second", "p50_ns", "p99_ns", "peak_rss_bytes", "peak_threads_sampled", "handoffs", "rocksdb_granted", "rocksdb_diverted"]:
            values = [float(s[metric]) for s in samples]
            row[metric + "_median"] = statistics.median(values)
            row[metric + "_min"] = min(values)
            row[metric + "_max"] = max(values)
        summaries.append(row)
    for row in summaries:
        baseline = next((r for r in summaries if r["variant"] == "tokio" and all(r[k] == row[k] for k in keys[1:])), None)
        row["throughput_vs_tokio"] = row["ops_per_second_median"] / baseline["ops_per_second_median"] if baseline else ""
    if summaries:
        with (output / "summary.csv").open("w", newline="") as f:
            writer = csv.DictWriter(f, fieldnames=list(summaries[0]))
            writer.writeheader()
            writer.writerows(summaries)
    print(json.dumps({"valid_runs": len([r for r in latest.values() if r["valid"] == "True"]), "invalid_attempts": len([r for r in rows if r["valid"] != "True"]), "groups": len(summaries)}), flush=True)


def classic(args, bins, output):
    """Run upstream's core read/create Criterion workloads as ordinary release
    binaries, keeping the same panic/codegen profile as the main comparison."""
    output.mkdir(parents=True, exist_ok=True)
    binaries = {v: sha256(bins / (v + "-sdb")) for v in args.variants.split(",")}
    environment_file = output / "environment.json"
    if environment_file.exists():
        if json.loads(environment_file.read_text())["binaries"] != binaries:
            raise RuntimeError("refusing to mix classic benchmark binaries")
    else:
        environment_file.write_text(json.dumps({"surrealdb_commit": UPSTREAM,
            "goexec_commit": GOEXEC, "binaries": binaries, "argv": sys.argv,
            "description": "Upstream core read/create routines, 1000 operations per batch, original small random values; separate from the 1 KiB fixed-seed SDK matrix",
            "rustc": command(["rustc", "+1.95.0", "-Vv"]),
            "profile": "release opt-level=3 lto=false codegen-units=16 panic=abort",
            "measurement_seconds": int(args.seconds), "repeats": args.repeats,
            "sample_size": 30}, indent=2) + "\n")
    blocks = list(itertools.product(args.backends.split(","), args.parallelism, range(args.repeats)))
    rng = random.Random(args.seed)
    rng.shuffle(blocks)
    jobs = []
    for backend, p, repeat in blocks:
        variants = [v for v in args.variants.split(",") if not (backend == "memory" and v == "goexec-blocking")]
        rng.shuffle(variants)
        jobs.extend((v, backend, p, repeat) for v in variants)
    results = []
    for index, (variant, backend, p, repeat) in enumerate(jobs):
        key = f"{variant}-{backend}-p{p}-r{repeat}"
        case = output / key
        case.mkdir(exist_ok=True)
        record_file = case / "result.json"
        if record_file.exists():
            results.extend(json.loads(record_file.read_text()))
            continue
        target = "lib-mem" if backend == "memory" else "lib-rocksdb"
        env = env_for(p)
        env.update(BENCH_DATASTORE_TARGET=target, BENCH_WORKER_THREADS=str(p),
                   BENCH_NUM_OPS="1000", BENCH_DURATION=str(int(args.seconds)),
                   BENCH_SAMPLE_SIZE="30", CRITERION_HOME=str(case / "criterion"))
        with tempfile.TemporaryDirectory(prefix="classic-", dir=output) as directory:
            _, process = capture(bins / (variant + "-sdb"), ["--bench", "--noplot"],
                                 env, case / "stdout", case / "stderr", cwd=directory)
        if process["exit_code"]:
            raise RuntimeError(f"classic benchmark failed: {key}: {process['error']}")
        records = []
        for workload in ("reads", "creates"):
            estimate = json.loads((case / "criterion" / target / workload / "new/estimates.json").read_text())
            row = dict(variant=variant, backend=backend, parallelism=p, repeat=repeat,
                       workload=workload, binary_sha256=binaries[variant], **process)
            row.update(mean_ns=estimate["mean"]["point_estimate"],
                       median_ns=estimate["median"]["point_estimate"],
                       mean_ci_lower=estimate["mean"]["confidence_interval"]["lower_bound"],
                       mean_ci_upper=estimate["mean"]["confidence_interval"]["upper_bound"])
            records.append(row)
        record_file.write_text(json.dumps(records, indent=2) + "\n")
        results.extend(records)
        print(json.dumps({"completed": index + 1, "total": len(jobs), "classic": key}), flush=True)
    with (output / "runs.csv").open("w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=list(results[0]))
        writer.writeheader()
        writer.writerows(results)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "build", "verify", "bench", "classic", "summarize"])
    parser.add_argument("--source", type=Path, default=REPO / "target/surrealdb-integration/source")
    parser.add_argument("--bins", type=Path, default=REPO / "target/surrealdb-integration/bin")
    parser.add_argument("--output", type=Path, default=REPO / "target/surrealdb-integration/results")
    parser.add_argument("--variants", default="tokio,goexec,goexec-blocking")
    parser.add_argument("--backends", default="memory,rocksdb")
    parser.add_argument("--access", default="sdk")
    parser.add_argument("--parallelism", type=lambda s: [int(x) for x in s.split(",")], default=[1, 4, 16])
    parser.add_argument("--factors", type=lambda s: [int(x) for x in s.split(",")], default=[1, 4])
    parser.add_argument("--workloads", default="read,create,update,mixed,range")
    parser.add_argument("--records", type=int, default=100000)
    parser.add_argument("--warmup", type=float, default=3)
    parser.add_argument("--seconds", type=float, default=10)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--seed", type=int, default=20260917)
    args = parser.parse_args()
    args.source, args.bins, args.output = (p.resolve() for p in (args.source, args.bins, args.output))
    if args.command == "prepare": prepare(args.source)
    elif args.command == "build": build(args.source, args.bins, args.variants.split(","))
    elif args.command == "verify": verify(args, args.bins, args.output)
    elif args.command == "bench": run_matrix(args, args.source, args.bins, args.output)
    elif args.command == "classic": classic(args, args.bins, args.output)
    else: summarize(args.output)


if __name__ == "__main__":
    main()
