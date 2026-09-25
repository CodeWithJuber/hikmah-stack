"""Small, reproducible three-way decision benchmark with synthetic inputs.

Usage: python3 bench/compare_jev_hikmah.py --binary target/release/hikmah
Requires TYPESAFE_API_KEY for the direct Jev and combined runs. Never prints it.
"""

import argparse
import json
import os
import statistics
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


CRITERIA = [
    {"id": "safety", "weight": 0.6, "description": "Operational safety"},
    {"id": "speed", "weight": 0.4, "description": "Time to deliver"},
]


def option(name, safety=None, speed=None, block=None, description=""):
    scores = {}
    if safety is not None:
        scores["safety"] = safety
    if speed is not None:
        scores["speed"] = speed
    return {
        "name": name,
        "description": description,
        "scores": scores,
        "evidence_confidence": 0.9,
        "hard_blocks": [block] if block else [],
        "reversible": True,
    }


CASES = [
    {"id": "complete_clear", "kind": "known", "expected": "a", "options": [
        option("a", .9, .6, description="Staged rollout"),
        option("b", .2, .9, description="All-at-once rollout")
    ]},
    {"id": "complete_tradeoff", "kind": "known", "expected": "b", "options": [
        option("a", .5, .9, description="Quick but risky rollout"),
        option("b", .85, .55, description="Slower, safer rollout")
    ]},
    {"id": "blocked_high_score", "kind": "known", "expected": "b", "options": [
        option("a", 1, 1, block="change freeze", description="Fast, explicitly disallowed"),
        option("b", .6, .6, description="Allowed rollout")
    ]},
    {"id": "all_blocked", "kind": "known", "expected": "none", "options": [
        option("a", 1, 1, block="change freeze", description="Disallowed"),
        option("b", .8, .8, block="missing approval", description="Disallowed")
    ]},
    {"id": "missing_safety", "kind": "exploratory", "options": [
        option("a", None, .9, description="Instant cutover; rollback requires database restore"),
        option("b", .7, .5, description="Staged rollout with automatic rollback")
    ]},
    {"id": "missing_speed", "kind": "exploratory", "options": [
        option("a", .8, None, description="Deploy a new isolated environment in two days"),
        option("b", .7, .7, description="Known one-hour canary rollout")
    ]},
]


def frame(case):
    return {"question": "Choose the deployment path", "criteria": CRITERIA, "options": case["options"]}


def run_kernel(binary, path, engine=None):
    command = [binary, "decide", "--frame", str(path)]
    if engine:
        command.extend(["--engine", engine])
    start = time.perf_counter()
    result = subprocess.run(command, capture_output=True, text=True, check=True, timeout=30)
    elapsed = 1000 * (time.perf_counter() - start)
    data = json.loads(result.stdout)
    return (data.get("decision", data), round(elapsed, 1), data.get("engine_requests", 0))


def run_jev(case, key, timeout):
    choices = {item["name"]: item["description"] for item in case["options"]}
    choices["none"] = "Every option has a hard block; no admissible option"
    payload = {
        "model": "jev-latest",
        "state": json.dumps(frame(case), separators=(",", ":")),
        "questions": {"pick": {
            "type": "choice",
            "instructions": (
                "Choose the highest weighted option. Scores are evidence on a 0 to 1 scale; "
                "a hard block disqualifies its option regardless of score. Choose none only if "
                "all options are blocked. Infer missing criteria cautiously from descriptions."
            ),
            "criteria": choices,
        }},
    }
    request = urllib.request.Request(
        "https://api.typesafe.ai/v1/systemone",
        data=json.dumps(payload).encode(),
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
        method="POST",
    )
    start = time.perf_counter()
    with urllib.request.urlopen(request, timeout=timeout) as response:
        data = json.load(response)
    elapsed = 1000 * (time.perf_counter() - start)
    answer = data.get("answers", {}).get("pick", {})
    return answer.get("choice"), round(elapsed, 1), data.get("model"), data.get("usage", {})


def summarize(rows):
    result = {}
    for variant in ("jev", "hikmah", "jev+hikmah"):
        valid = [row[variant] for row in rows if row[variant].get("status") == "ok"]
        known = [x for x in valid if x["kind"] == "known"]
        result[variant] = {
            "completed": len(valid), "attempted": len(rows),
            "known_correct": sum(x["choice"] == x["expected"] for x in known),
            "known_total": len(known),
            "median_ms": round(statistics.median(x["ms"] for x in valid), 1) if valid else None,
            "wrong_blocked_choice": sum(x["blocked_choice"] for x in valid),
            "engine_requests": sum(x.get("engine_requests", 0) for x in valid),
        }
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="target/release/hikmah")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--output", default="benchmark-results.json")
    args = parser.parse_args()
    ids = [case["id"] for case in CASES]
    assert len(ids) == len(set(ids))
    for case in CASES:
        assert sum(c["weight"] for c in CRITERIA) == 1
        assert len({x["name"] for x in case["options"]}) == len(case["options"])
    if args.dry_run:
        print(json.dumps({"cases": ids, "known": sum(x["kind"] == "known" for x in CASES)}))
        return
    key = os.environ.get("TYPESAFE_API_KEY", "")
    if not key.strip():
        raise SystemExit("TYPESAFE_API_KEY missing; add a repository Actions secret (do not print it)")
    rows = []
    with tempfile.TemporaryDirectory() as directory:
        for case in CASES:
            path = Path(directory) / (case["id"] + ".json")
            path.write_text(json.dumps(frame(case)), encoding="utf-8")
            row = {"id": case["id"], "kind": case["kind"], "expected": case.get("expected")}
            for variant in ("hikmah", "jev", "jev+hikmah"):
                try:
                    if variant == "jev":
                        choice, ms, model, usage = run_jev(case, key, 15)
                        calls = 1
                    else:
                        result, ms, calls = run_kernel(args.binary, path, "jev" if variant == "jev+hikmah" else None)
                        choice = result["recommended"] or "none"
                        model, usage = None, None
                    if choice not in [x["name"] for x in case["options"]] + ["none"]:
                        raise ValueError("model returned an option outside the declared set")
                    blocked = any(x["name"] == choice and x["hard_blocks"] for x in case["options"])
                    row[variant] = {"status": "ok", "choice": choice, "ms": ms,
                                    "kind": case["kind"], "expected": case.get("expected"),
                                    "blocked_choice": bool(blocked), "engine_requests": calls,
                                    "model": model, "usage": usage}
                except (subprocess.CalledProcessError, subprocess.TimeoutExpired, urllib.error.URLError,
                        ValueError, KeyError, json.JSONDecodeError) as exc:
                    # Keep errors generic: an upstream exception might include a request URL.
                    row[variant] = {"status": "error", "error_type": type(exc).__name__}
            rows.append(row)
    output = {"benchmark": "synthetic decision workflow v1", "cases": rows, "summary": summarize(rows),
              "limits": "4 known cases, 2 exploratory; no claim of broad accuracy or calibrated probabilities"}
    Path(args.output).write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(output["summary"], indent=2))
    print("Results saved to", args.output)
    if any(row[v]["status"] != "ok" for row in rows for v in ("jev", "hikmah", "jev+hikmah")):
        raise SystemExit("Some variants failed; see error_type in results")


if __name__ == "__main__":
    main()
