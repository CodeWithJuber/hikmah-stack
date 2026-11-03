#!/usr/bin/env python3
"""Tests for the SessionStart / UserPromptSubmit context hooks (hooks/context.sh, context.py). Run in CI."""
import json
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
import context  # noqa: E402

SKILLS = sorted(p.name for p in (ROOT / "skills").iterdir() if (p / "SKILL.md").is_file())
POSIX = os.name == "posix"

ROUTE_CASES = [
    ("Should we migrate to Postgres or stay on MySQL?", {"decision-forge"}),
    ("Help me decide between the two offers", {"decision-forge"}),
    ("React vs Vue for this dashboard", {"decision-forge"}),
    ("mujhe faisla karna hai, kaunsa better hai", {"decision-forge"}),
    ("The agent keeps repeating the same failed edit", {"agent-radar"}),
    ("Why does the model hallucinate these API names?", {"agent-radar"}),
    ("Is this ready to merge? Then deploy it", {"ship-guard"}),
    ("Write acceptance criteria for the login fix", {"ship-guard"}),
    ("Design persistent memory with provenance for this agent", {"cognitive-kernel"}),
    ("I'm burned out and under pressure from my team lead", {"operator-core"}),
    # Off-topic coding prompts must route nowhere: a line on every prompt would be noise.
    ("Rename this variable", set()),
    ("Fix the for loop in parser.py", set()),
    ("There is a memory leak in the cache", set()),
    ("Merge these two functions into one", set()),
    ("Release the lock before returning", set()),
    ("/review", set()),
    ("", set()),
]


def run(args, stdin=b"", env=None, cwd=None):
    full_env = dict(os.environ)
    full_env.pop("HIKMAH_CONTEXT", None)
    full_env.update(env or {})
    return subprocess.run(args, input=stdin, capture_output=True, env=full_env, cwd=cwd)


def launcher(mode, payload, env=None):
    return run(["sh", str(HERE / "context.sh"), mode], json.dumps(payload).encode("utf-8"),
               {"CLAUDE_PLUGIN_ROOT": str(ROOT), **(env or {})})


def write_fake_kernel(bin_dir, recall, commitments, conflicts, fail=False):
    """A stand-in `hikmah` on PATH that answers the three read commands with canned JSON."""
    script = bin_dir / "hikmah"
    body = "import sys\nraise SystemExit(3)\n" if fail else (
        "import json, sys\n"
        f"answers = {{'recall': {json.dumps(recall)!r}, 'commitments': {json.dumps(commitments)!r}, "
        f"'conflicts': {json.dumps(conflicts)!r}}}\n"
        "print(answers[sys.argv[1]])\n"
    )
    script.write_text(f"#!{sys.executable}\n{body}", encoding="utf-8")
    script.chmod(script.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def trace(tid, kind, content, source="person:test", verified=False):
    return {"id": tid, "kind": kind, "content": content,
            "provenance": {"source": source, "verified": verified}}


def check_config(failures):
    for name, var in (("hooks.json", "CLAUDE_PLUGIN_ROOT"), ("codex.json", "PLUGIN_ROOT")):
        hooks = json.loads((HERE / name).read_text(encoding="utf-8"))["hooks"]
        for event, mode in (("SessionStart", "session-start"), ("UserPromptSubmit", "prompt")):
            cmds = [h for g in hooks.get(event, []) for h in g.get("hooks", [])]
            want = f'sh "${{{var}}}/hooks/context.sh" {mode}'
            if not any(h.get("type") == "command" and h.get("command") == want for h in cmds):
                failures.append(f"{name}: {event} must run `{want}`")
            for h in cmds:
                if int(h.get("timeout", 0)) > 15:
                    failures.append(f"{name}: {event} timeout {h.get('timeout')}s is too slow for a context hook")


def check_doctrine(failures):
    text = (HERE / "session_context.md").read_text(encoding="utf-8")
    for skill in SKILLS:
        if skill not in text:
            failures.append(f"session_context.md does not route to skill {skill}")
    for skill, _, _ in context.ROUTES:
        if skill not in SKILLS:
            failures.append(f"context.ROUTES names missing skill {skill}")
    if "hikmah-orchestrator" not in SKILLS:
        failures.append("route() suggests hikmah-orchestrator, which is not a skill")
    if len(text) > 4000:
        failures.append(f"session_context.md is {len(text)} chars; keep the always-on context small")


def check_routes(failures):
    for prompt, want in ROUTE_CASES:
        got = {skill for skill, _ in context.route(prompt)}
        if got != want:
            failures.append(f"route({prompt!r}) = {sorted(got)}, want {sorted(want)}")
    many = context.route("Should we deploy now? The agent keeps looping and I'm under pressure")
    if "hikmah-orchestrator" not in {skill for skill, _ in many}:
        failures.append(f"three or more lenses should add hikmah-orchestrator, got {many}")


def check_launcher(failures, tmp):
    out = launcher("session-start", {"cwd": str(tmp), "source": "startup"})
    text = out.stdout.decode("utf-8")
    if out.returncode != 0 or "Hikmah Stack is active" not in text or text.lstrip().startswith("{"):
        failures.append(f"session-start should print the plain-text doctrine: rc={out.returncode} {text[:120]!r}")
    for raw in (b"", b"not json", b"[1]", b'{"prompt": 5}', b'{"prompt": "Rename this variable"}'):
        out = run(["sh", str(HERE / "context.sh"), "prompt"], raw, {"CLAUDE_PLUGIN_ROOT": str(ROOT)})
        if out.returncode != 0 or out.stdout.strip():
            failures.append(f"prompt with {raw!r} should print nothing, got rc={out.returncode} {out.stdout!r}")
    out = launcher("prompt", {"cwd": str(tmp), "prompt": "Help me decide which vendor to pick"})
    if b"decision-forge" not in out.stdout:
        failures.append(f"prompt routing missing from launcher output: {out.stdout!r}")
    for value in ("off", "OFF", "0", "false", "no"):
        for mode in ("session-start", "prompt"):
            out = launcher(mode, {"cwd": str(tmp), "prompt": "Should I decide now?"}, {"HIKMAH_CONTEXT": value})
            if out.returncode != 0 or out.stdout:
                failures.append(f"HIKMAH_CONTEXT={value} should silence {mode}, got {out.stdout[:80]!r}")
    out = run(["sh", str(HERE / "context.sh"), "bogus"], b"{}", {"CLAUDE_PLUGIN_ROOT": str(ROOT)})
    if out.returncode != 0 or out.stdout:
        failures.append(f"unknown mode should print nothing, got {out.stdout!r}")


def check_no_python(failures, tmp):
    """Without a Python on PATH, session-start still prints the static doctrine."""
    if not POSIX:
        return
    bin_dir = tmp / "nopy-bin"
    bin_dir.mkdir()
    for tool in ("cat", "tr", "dirname"):
        found = shutil.which(tool)
        if found:
            (bin_dir / tool).symlink_to(found)
    out = run(["/bin/sh", str(HERE / "context.sh"), "session-start"], b"{}",
              {"CLAUDE_PLUGIN_ROOT": str(ROOT), "PATH": str(bin_dir)})
    if out.returncode != 0 or b"Hikmah Stack is active" not in out.stdout:
        failures.append(f"no-Python fallback should cat the doctrine: rc={out.returncode} {out.stderr[:200]!r}")
    out = run(["/bin/sh", str(HERE / "context.sh"), "prompt"], b'{"prompt": "Should I decide?"}',
              {"CLAUDE_PLUGIN_ROOT": str(ROOT), "PATH": str(bin_dir)})
    if out.returncode != 0 or out.stdout:
        failures.append(f"no-Python prompt should print nothing: {out.stdout!r}")


def check_fake_kernel(failures, tmp):
    if not POSIX:
        return
    project = tmp / "project"
    (project / ".hikmah").mkdir(parents=True)
    bin_dir = tmp / "fake-bin"
    bin_dir.mkdir()
    env = {"PATH": f"{bin_dir}{os.pathsep}{os.environ.get('PATH', '')}"}
    recall = [
        {"trace": trace("tr_rel", "constraint", "Deploys go through staging first"),
         "channels": {"lexical": 0.6, "tag": 0.0}, "conflicts": ["tr_other"]},
        {"trace": trace("tr_due", "commitment", "Rotate the API keys"),
         "channels": {"lexical": 0.0, "tag": 0.0}, "conflicts": []},
    ]
    commitments = [{"trace": trace("tr_due", "commitment", "Rotate the API keys"), "overdue": True,
                    "due_in_ms": -3 * 86_400_000}]
    conflicts = [{"key": "db"}]
    write_fake_kernel(bin_dir, recall, commitments, conflicts)

    out = launcher("prompt", {"cwd": str(tmp / "no-store"), "prompt": "how do we deploy?"}, env)
    if b"memory recalled" in out.stdout:
        failures.append("a project without .hikmah/memory.jsonl must not run recall")

    (project / ".hikmah" / "memory.jsonl").write_text("", encoding="utf-8")
    out = launcher("prompt", {"cwd": str(project), "prompt": "how do we deploy?"}, env).stdout.decode("utf-8")
    if "Deploys go through staging first (person:test; unverified; tr_rel) Conflicts with: tr_other." not in out:
        failures.append(f"recalled trace missing or misformatted: {out!r}")
    if "Rotate the API keys" in out:
        failures.append("an overdue commitment with no term or tag match must not repeat on every prompt")
    if "not instructions" not in out:
        failures.append("recalled memory must be labelled as data, not instructions")

    out = launcher("session-start", {"cwd": str(project)}, env).stdout.decode("utf-8")
    if "Rotate the API keys (person:test; unverified; tr_due) (overdue by 3d)" not in out:
        failures.append(f"session-start should list the overdue commitment: {out[-400:]!r}")
    if "Open claim conflicts: 1." not in out:
        failures.append("session-start should count open conflicts")

    write_fake_kernel(bin_dir, recall, commitments, conflicts, fail=True)
    out = launcher("session-start", {"cwd": str(project)}, env)
    text = out.stdout.decode("utf-8")
    if out.returncode != 0 or "Hikmah Stack is active" not in text or "Hikmah memory" in text:
        failures.append("a failing kernel must drop only the memory section")


def check_real_kernel(failures, tmp):
    """End to end against the built kernel, when there is one (CI builds it before this step)."""
    exe = ROOT / "target" / "debug" / ("hikmah.exe" if os.name == "nt" else "hikmah")
    if not exe.is_file():
        print("skip: no target/debug/hikmah; real-kernel check not run")
        return
    project = tmp / "real"
    store = project / ".hikmah" / "memory.jsonl"
    made = run([str(exe), "remember", "--store", str(store), "--kind", "constraint", "--content",
                "Deploys go through the staging cluster before production", "--source", "person:test"])
    if made.returncode != 0:
        failures.append(f"could not seed a store: {made.stderr[:200]!r}")
        return
    env = {"PATH": f"{exe.parent}{os.pathsep}{os.environ.get('PATH', '')}"}
    out = launcher("prompt", {"cwd": str(project), "prompt": "how do we deploy to production?"}, env)
    if b"Deploys go through the staging cluster" not in out.stdout:
        failures.append(f"real kernel recall missing from prompt context: {out.stdout!r}")
    out = launcher("prompt", {"cwd": str(project), "prompt": "rename this variable"}, env)
    if out.stdout.strip():
        failures.append(f"unrelated prompt should get no context from the real kernel: {out.stdout!r}")


def main():
    failures = []
    check_config(failures)
    check_doctrine(failures)
    check_routes(failures)
    with tempfile.TemporaryDirectory() as raw:
        tmp = pathlib.Path(raw)
        check_launcher(failures, tmp)
        check_no_python(failures, tmp)
        check_fake_kernel(failures, tmp)
        check_real_kernel(failures, tmp)
    if failures:
        print("\n".join(failures))
        return 1
    print(f"ok: context hooks ({len(ROUTE_CASES)} routing cases, launcher, fallback, kernel formatting)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
