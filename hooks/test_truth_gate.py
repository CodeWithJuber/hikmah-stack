#!/usr/bin/env python3
"""Golden-case and robustness tests for the Python Truth Gate fallback (run in CI)."""
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import truth_gate  # noqa: E402


def run_hook(stdin_bytes):
    result = subprocess.run(
        [sys.executable, str(HERE / "truth_gate.py")], input=stdin_bytes, capture_output=True
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def main():
    cases = json.loads((HERE / "truth_gate_cases.json").read_text(encoding="utf-8"))["cases"]
    failures = []
    for case in cases:
        got = "block" if truth_gate.rules_verdict(case["message"]) else "allow"
        if got != case["expect"]:
            failures.append(f"expected {case['expect']}, got {got}: {case['message']!r}")
    for raw in [b"", b"not json", b"[1,2]", b"null", b'"hello"', b'{"last_assistant_message": ["x"]}',
                b'{"last_assistant_message": 5}', b'{"stop_hook_active": "true", "last_assistant_message": "Done. TODO"}',
                b'{"last_assistant_message": "Done. TODO \xff"}']:
        out = run_hook(raw)
        if raw.endswith(b'\xff"}'):
            if out.get("decision") != "block":
                failures.append(f"invalid UTF-8 message should still be judged: {out}")
        elif out != {}:
            failures.append(f"expected allow for {raw!r}, got {out}")
    if failures:
        print("\n".join(failures))
        return 1
    print(f"ok: {len(cases)} golden cases + robustness checks")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
