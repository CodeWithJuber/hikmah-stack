#!/usr/bin/env python3
from __future__ import annotations
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXPECTED_VERSION = "2.0.0"
EXPECTED_SKILLS = {"operator-core", "agent-radar", "decision-forge", "ship-guard", "hikmah-orchestrator"}


def fail(msg: str) -> None:
    raise SystemExit(f"ERROR: {msg}")


def load_json(rel: str):
    path = ROOT / rel
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        fail(f"invalid JSON in {rel}: {exc}")


def parse_skill(path: Path):
    text = path.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        fail(f"missing YAML frontmatter: {path.relative_to(ROOT)}")
    end = text.find("\n---", 4)
    if end < 0:
        fail(f"unterminated frontmatter: {path.relative_to(ROOT)}")
    front = text[4:end]
    name_m = re.search(r"(?m)^name:\s*([a-z0-9-]+)\s*$", front)
    version_m = re.search(r'(?m)^\s*version:\s*["\']?([^"\'\n]+)', front)
    if not name_m:
        fail(f"skill missing name: {path.relative_to(ROOT)}")
    return name_m.group(1), (version_m.group(1).strip() if version_m else None), text


def main() -> None:
    required = [
        ".codex-plugin/plugin.json", ".claude-plugin/plugin.json",
        ".agents/plugins/marketplace.json", ".claude-plugin/marketplace.json",
        "hooks/hooks.json", "hooks/codex.json", "hooks/truth_gate.py",
        "README.md", "LICENSE", "SECURITY.md", "CONTRIBUTING.md",
        "docs/ARCHITECTURE.md", "docs/COMPATIBILITY.md", "docs/ETHICS.md", "docs/EVIDENCE.md",
    ]
    for rel in required:
        if not (ROOT / rel).exists():
            fail(f"missing required file: {rel}")

    codex = load_json(".codex-plugin/plugin.json")
    claude = load_json(".claude-plugin/plugin.json")
    load_json(".agents/plugins/marketplace.json")
    load_json(".claude-plugin/marketplace.json")
    codex_hooks = load_json("hooks/codex.json")
    claude_hooks = load_json("hooks/hooks.json")

    for label, manifest in (("codex", codex), ("claude", claude)):
        if manifest.get("name") != "hikmah-stack":
            fail(f"{label} manifest name mismatch")
        if manifest.get("version") != EXPECTED_VERSION:
            fail(f"{label} manifest version mismatch")
        if manifest.get("license") != "MIT":
            fail(f"{label} manifest license must be MIT")
        if manifest.get("author", {}).get("name") != "Juber Shaikh":
            fail(f"{label} manifest author mismatch")

    if codex.get("skills") != "./skills/":
        fail("Codex manifest must point to ./skills/")
    if codex.get("hooks") != "./hooks/codex.json":
        fail("Codex manifest must use the command-hook adapter")

    handler_types = [h.get("type") for group in codex_hooks.get("hooks", {}).values() for entry in group for h in entry.get("hooks", [])]
    if not handler_types or any(t != "command" for t in handler_types):
        fail("Codex hooks must use command handlers only")
    if not claude_hooks.get("hooks", {}).get("Stop"):
        fail("Claude Stop quality hook missing")

    found = set()
    for skill_md in sorted((ROOT / "skills").glob("*/SKILL.md")):
        name, version, text = parse_skill(skill_md)
        if name != skill_md.parent.name:
            fail(f"skill name/path mismatch: {skill_md.parent.name} vs {name}")
        if name in found:
            fail(f"duplicate skill name: {name}")
        found.add(name)
        if version != EXPECTED_VERSION:
            fail(f"skill version mismatch in {name}: {version}")
        if len(text) < 250:
            fail(f"skill appears suspiciously short: {name}")

    if found != EXPECTED_SKILLS:
        fail(f"skills mismatch; expected {sorted(EXPECTED_SKILLS)}, got {sorted(found)}")

    forbidden = ["sk_test_", "sk_live_", "BEGIN PRIVATE KEY", "ghp_", "github_pat_"]
    for path in ROOT.rglob("*"):
        if path.is_file() and ".git" not in path.parts:
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for token in forbidden:
                if token in text and path.name not in {"validate.py"}:
                    fail(f"possible secret marker {token!r} in {path.relative_to(ROOT)}")

    print(f"OK: Hikmah Stack {EXPECTED_VERSION}; {len(found)} skills; manifests/hooks/docs validated")


if __name__ == "__main__":
    main()
