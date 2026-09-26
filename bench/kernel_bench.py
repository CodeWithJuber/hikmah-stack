#!/usr/bin/env python3
"""Offline kernel capability measurements; Python stdlib, no API credentials or network calls."""
import argparse
import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/release/examples/bench_kernel"


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def execute(command, timeout, stdin=None):
    process = subprocess.Popen(command, cwd=ROOT, text=True, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               start_new_session=(os.name == "posix"))
    try:
        stdout, stderr = process.communicate(stdin, timeout=timeout)
    except subprocess.TimeoutExpired:
        if os.name == "posix":
            os.killpg(process.pid, signal.SIGKILL)
        else:
            process.kill()
        process.communicate()
        raise RuntimeError(f"stage timed out after {timeout}s") from None
    return process.returncode, stdout, stderr


def ratio(a, b):
    return a / b if b else None


def wilson(a, n):
    if not n:
        return None
    z = 1.959963984540054
    p = a / n
    divisor = 1 + z*z/n
    center = (p + z*z/(2*n)) / divisor
    half = z * math.sqrt(p*(1-p)/n + z*z/(4*n*n)) / divisor
    return [max(0, center-half), min(1, center+half)]


def confusion(labels, predictions):
    if len(labels) != len(predictions) or not labels:
        raise ValueError("nonempty equal-sized labels and predictions required")
    if any(type(x) is not bool for x in labels + predictions):
        raise ValueError("labels and predictions must be booleans")
    tp = sum(y and p for y, p in zip(labels, predictions))
    tn = sum(not y and not p for y, p in zip(labels, predictions))
    fp = sum(not y and p for y, p in zip(labels, predictions))
    fn = sum(y and not p for y, p in zip(labels, predictions))
    return {"n": len(labels), "tp": tp, "tn": tn, "fp": fp, "fn": fn,
            "precision": ratio(tp, tp+fp), "recall": ratio(tp, tp+fn),
            "f1": ratio(2*tp, 2*tp+fp+fn),
            "false_block_rate": ratio(fp, fp+tn), "false_pass_rate": ratio(fn, fn+tp),
            "recall_wilson95": wilson(tp, tp+fn),
            "false_block_wilson95": wilson(fp, fp+tn)}


def load_corpus(path):
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    if data.get("kind") not in {"real_labeled", "synthetic_regression"}:
        raise ValueError("corpus kind must be real_labeled or synthetic_regression")
    if not isinstance(data.get("source"), str) or not data["source"].strip():
        raise ValueError("corpus source is required")
    cases = data.get("cases")
    if not isinstance(cases, list) or not cases:
        raise ValueError("nonempty cases array required")
    ids, messages = set(), set()
    for case in cases:
        for key in ("id", "task_id", "message", "label_source"):
            if not isinstance(case.get(key), str) or not case[key].strip():
                raise ValueError(f"every case requires nonempty {key}")
        if type(case.get("false_completion")) is not bool:
            raise ValueError("false_completion must be a boolean independent outcome label")
        if case["id"] in ids:
            raise ValueError("duplicate case id")
        ids.add(case["id"])
        messages.add(case["message"])
    data["duplicate_message_rows"] = len(cases) - len(messages)
    return data


def evaluate_corpus(path, binary, timeout):
    data = load_corpus(path)
    cases = data["cases"]
    # Labels, task outcomes and source annotations never enter the gate request.
    payload = "".join(json.dumps({"id": c["id"], "message": c["message"]})+"\n" for c in cases)
    code, stdout, stderr = execute([str(binary), "gate"], timeout, payload)
    if code:
        raise RuntimeError(f"gate exited {code}: {stderr[:300]}")
    results = [json.loads(line) for line in stdout.splitlines()]
    if len(results) != len(cases) or any(r["id"] != c["id"] for r, c in zip(results, cases)):
        raise RuntimeError("gate output missing, reordered, or duplicated rows")
    metrics = confusion([c["false_completion"] for c in cases], [r["blocked"] for r in results])
    groups = {}
    for c, r in zip(cases, results):
        groups.setdefault(c["task_id"], []).append((c["false_completion"], r["blocked"]))
    by_task = {k: confusion([y for y, _ in rows], [p for _, p in rows]) for k, rows in groups.items()}
    return {"kind": data["kind"], "source": data["source"], "corpus_sha256": digest(path),
            "metrics": metrics, "task_count": len(groups), "by_task": by_task,
            "duplicate_message_rows": data["duplicate_message_rows"],
            "predictions": [{"id": c["id"], "false_completion": c["false_completion"],
                             "blocked": r["blocked"], "ms": r["ms"]} for c, r in zip(cases, results)],
            "limits": ["Rules-only message screen; not an execution verifier or an engine evaluation.",
                       "Labels and source annotations are supplied by the corpus author, not authenticated.",
                       "Message-level Wilson intervals assume independence; task clusters and duplicates may violate it.",
                       "No threshold tuning or corpus fitting performed by this evaluator."]}


def save(out, summary):
    tmp = out / "summary.tmp"
    tmp.write_text(json.dumps(summary, indent=2)+"\n", encoding="utf-8")
    tmp.replace(out / "summary.json")
    lines = ["# Pure Hikmah kernel benchmark", "", f"Status: **{summary['status']}**.",
             "Offline actual-kernel measurements. Synthetic results are not field accuracy.", "",
             "| Stage | Result |", "|---|---|"]
    for name, stage in summary["stages"].items():
        lines.append(f"| {name} | {stage['status']} |")
    scales = [s["result"] for n, s in summary["stages"].items()
              if n.startswith("scale-") and s.get("result")]
    if scales:
        lines.extend(["", "| Records | Recall p50 ms | Recall p95 ms | Replay ms | Peak RSS MiB | Exact-cue hits |",
                      "|---|---|---|---|---|---|"])
        for s in scales:
            peak = s["memory_after_replay"]["process_peak_rss_kib"]
            peak_text = f"{peak/1024:.1f}" if peak is not None else "unavailable"
            lines.append(f"| {s['records']} | {s['recall_ms']['p50']:.3f} | {s['recall_ms']['p95']:.3f} | "
                         f"{s['replay_ms']:.3f} | {peak_text} | {s['exact_cue_hits']}/{s['queries']} |")
    decisions = summary["stages"].get("decision-invariants", {}).get("result")
    if decisions:
        lines.extend(["", f"Decision invariants: **{decisions['violations']} violating cases / "
                      f"{decisions['cases']} generated cases**, seed {decisions['seed']}.",
                      "This is a finite synthetic sample, not proof of zero risk."])
    recovery = summary["stages"].get("recovery", {}).get("result")
    if recovery:
        lines.extend(["", "| Recovery check | Observed |", "|---|---|"])
        for key in ("acknowledged_survived", "torn_tail_repaired", "edit_detected", "truncation_detected"):
            lines.append(f"| {key} | {recovery[key]} |")
    gate = summary["stages"].get("truth-gate-corpus", {}).get("result")
    if gate:
        m = gate["metrics"]
        lines.extend(["", f"Truth Gate corpus kind: **{gate['kind']}**, {m['n']} messages, "
                      f"{gate['task_count']} tasks; duplicate message rows: {gate['duplicate_message_rows']}.",
                      f"TP={m['tp']}, TN={m['tn']}, FP={m['fp']}, FN={m['fn']}."])
        for key in ("precision", "recall", "false_block_rate", "false_pass_rate"):
            value = m[key]
            lines.append(f"- {key}: " + (f"{value:.2%}" if value is not None else "undefined (empty denominator)"))
    lines.extend(["", "Not evaluated here: semantic recall on real labeled memories; portable-skill on/off effects;",
                  "full host/tool integration; power-loss recovery; authenticated provenance or encryption.",
                  "Each stage records its further limits in summary.json."])
    if "truth-gate-corpus" not in summary["stages"]:
        lines.append("No labeled transcript corpus supplied: field Truth Gate rates are not measured.")
    (out / "report.md").write_text("\n".join(lines)+"\n", encoding="utf-8")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sizes", type=int, nargs="+", default=[1000, 10000, 100000])
    parser.add_argument("--queries", type=int, default=30)
    parser.add_argument("--decision-cases", type=int, default=10000)
    parser.add_argument("--seed", type=int, default=20260926)
    parser.add_argument("--timeout", type=int, default=1800, help="seconds per stage; timeout is a failure")
    parser.add_argument("--gate-corpus", type=Path)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args(argv)
    if any(n < 1 or n > 1000000 for n in args.sizes) or len(set(args.sizes)) != len(args.sizes):
        parser.error("sizes must be distinct positive counts, at most 1000000")
    if not 1 <= args.queries <= 10000 or args.decision_cases < 6 or args.timeout < 1:
        parser.error("queries must be 1..10000; decision-cases >= 6; timeout > 0")
    if not 0 <= args.seed < 2**64:
        parser.error("seed must fit u64")
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    out = (args.out or ROOT / ".benchmark-results" / f"kernel-{stamp}").resolve()
    out.mkdir(parents=True, exist_ok=False)
    os.chmod(out, 0o700)
    commit = execute(["git", "rev-parse", "HEAD"], 10)[1].strip()
    dirty = bool(execute(["git", "status", "--porcelain"], 10)[1].strip())
    paths = sorted((ROOT / "runtime/hikmah-kernel").rglob("*.rs"))
    paths += [ROOT / "Cargo.lock", ROOT / "runtime/hikmah-kernel/Cargo.toml", Path(__file__)]
    summary = {"protocol": "pure-kernel-v1", "status": "running", "started_utc": stamp,
               "commit": commit, "dirty_checkout": dirty,
               "source_sha256": {str(p.relative_to(ROOT)): digest(p) for p in paths},
               "platform": platform.platform(), "cpu_count": os.cpu_count(),
               "rustc": execute(["rustc", "--version", "--verbose"], 20)[1].strip(),
               "settings": {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()},
               "stages": {}}
    save(out, summary)
    print(f"Results: {out}", flush=True)

    def stage(name, command=None, operation=None):
        start = time.monotonic()
        result = None
        error = None
        try:
            if operation:
                result = operation()
            else:
                code, stdout, stderr = execute(command, args.timeout)
                (out / f"{name}.stdout.log").write_text(stdout, encoding="utf-8")
                (out / f"{name}.stderr.log").write_text(stderr, encoding="utf-8")
                if command[0] == str(BINARY):
                    try:
                        result = json.loads(stdout)
                    except json.JSONDecodeError:
                        pass
                if code:
                    raise RuntimeError(f"exit {code}; see {name}.stderr.log")
        except Exception as exc:
            error = str(exc)
        summary["stages"][name] = {"status": "failed" if error else "completed",
                                  "elapsed_s": time.monotonic()-start, "result": result, "error": error}
        save(out, summary)
        print(f"{name}: {'FAILED: '+error if error else 'completed'}", flush=True)
        return error is None

    try:
        if not stage("build", ["cargo", "build", "--locked", "--release", "-p", "hikmah-kernel",
                                "--no-default-features", "--example", "bench_kernel"]):
            summary["status"] = "failed"
            return 1
        summary["binary_sha256"] = digest(BINARY)
        stage("regression", ["cargo", "test", "--locked", "--workspace", "--no-default-features"])
        stage("package", ["cargo", "run", "--locked", "-p", "hikmah-kernel", "--no-default-features",
                          "--", "validate", "--root", "."])
        stage("python-truth-gate", [sys.executable, "hooks/test_truth_gate.py"])
        stage("release-hook-timing", ["cargo", "test", "--locked", "--release", "-p", "hikmah-kernel",
              "--no-default-features", "--lib", "long_unpunctuated_text_stays_fast", "--",
              "--ignored", "--test-threads=1", "--nocapture"])
        stage("decision-invariants", [str(BINARY), "decisions", "--cases", str(args.decision_cases),
                                       "--seed", str(args.seed)])
        stage("recovery", [str(BINARY), "recovery", "--dir", str(out / "recovery")])
        for n in sorted(args.sizes):
            stage(f"scale-{n}", [str(BINARY), "scale", "--records", str(n), "--queries", str(args.queries),
                                "--dir", str(out / f"scale-{n}")])
        if args.gate_corpus:
            stage("truth-gate-corpus", operation=lambda: evaluate_corpus(args.gate_corpus, BINARY, args.timeout))
        summary["status"] = "failed" if any(s["status"] == "failed" for s in summary["stages"].values()) else "completed"
        return int(summary["status"] != "completed")
    finally:
        if summary["status"] == "running":
            summary["status"] = "interrupted"
        summary["finished_utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
        save(out, summary)


if __name__ == "__main__":
    sys.exit(main())
