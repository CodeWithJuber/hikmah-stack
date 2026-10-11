"""Python bridge for the Hikmah deterministic memory kernel.

Stdlib-only: it shells out to the `hikmah` JSON CLI and returns parsed dicts,
so the author's Python stack can use the kernel without native bindings.

The binary is located via an explicit ``binary=`` argument, else the
``HIKMAH_BIN`` environment variable, else ``PATH``. A missing binary raises
``HikmahError`` with a clear message instead of failing obscurely.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess

__version__ = "3.1.0"


class HikmahError(Exception):
    """The `hikmah` CLI failed, timed out, or returned something that is not JSON."""


def _locate_binary(explicit: str | None) -> str:
    if explicit:
        return explicit
    env = os.environ.get("HIKMAH_BIN")
    if env:
        return env
    found = shutil.which("hikmah")
    if found:
        return found
    raise HikmahError(
        "hikmah binary not found: set the HIKMAH_BIN environment variable, "
        "put `hikmah` on PATH (e.g. `cargo build --release -p hikmah-kernel`), "
        "or pass binary=... to Client."
    )


class Client:
    """Thin wrapper around the `hikmah` JSON CLI. Every method returns parsed JSON."""

    def __init__(
        self,
        binary: str | None = None,
        store: str | None = None,
        policy: str | None = None,
    ) -> None:
        self.binary = _locate_binary(binary)
        self.store = store
        self.policy = policy

    def _run(self, *args: str) -> str:
        cmd = [self.binary]
        if self.policy:
            cmd += ["--policy", self.policy]
        cmd += list(args)
        try:
            proc = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        except FileNotFoundError as exc:
            raise HikmahError(f"hikmah binary not executable at {self.binary!r}: {exc}")
        except subprocess.TimeoutExpired:
            raise HikmahError(f"hikmah timed out after 120s: {' '.join(cmd)}")
        if proc.returncode != 0:
            detail = (proc.stderr or proc.stdout).strip()
            raise HikmahError(f"hikmah exited {proc.returncode}: {detail[-2000:]}")
        return proc.stdout

    def _json(self, *args: str):
        out = self._run(*args)
        try:
            return json.loads(out)
        except json.JSONDecodeError as exc:
            raise HikmahError(f"hikmah returned non-JSON output: {exc}\n{out[:2000]}")

    def _store_args(self) -> list:
        return ["--store", self.store] if self.store else []

    def init(self) -> dict:
        """Create an empty memory store."""
        return self._json("init", *self._store_args())

    def remember(
        self,
        content: str,
        *,
        kind: str = "belief",
        source: str = "unknown",
        tags: tuple = (),
        salience: float | None = None,
        confidence: float | None = None,
        privacy: str | None = None,
        claim_key: str | None = None,
        claim_value: str | None = None,
        supersedes: str | None = None,
        deadline: str | None = None,
        locator: str | None = None,
        authority: float | None = None,
        verified: bool = False,
    ) -> dict:
        """Append a trace. Returns ``{"conflicts": [...], "trace": {...}}``."""
        args = ["remember", *self._store_args(), "--kind", kind, "--content", content,
                "--source", source]
        for tag in tags:
            args += ["--tag", tag]
        optional = {
            "--salience": salience,
            "--confidence": confidence,
            "--privacy": privacy,
            "--claim-key": claim_key,
            "--claim-value": claim_value,
            "--supersedes": supersedes,
            "--deadline": deadline,
            "--locator": locator,
            "--authority": authority,
        }
        for flag, value in optional.items():
            if value is not None:
                args += [flag, str(value)]
        if verified:
            args.append("--verified")
        return self._json(*args)

    def recall(
        self,
        query: str,
        *,
        tags: tuple = (),
        kinds: tuple = (),
        limit: int = 8,
        include_superseded: bool = False,
    ) -> list:
        """Contextual recall. Returns a list of hits, each with a ``trace_id``."""
        args = ["recall", *self._store_args(), "--query", query, "--limit", str(limit)]
        for tag in tags:
            args += ["--tag", tag]
        for kind in kinds:
            args += ["--kind", kind]
        if include_superseded:
            args.append("--include-superseded")
        return self._json(*args)

    def conflicts(self) -> list:
        """List unresolved conflicts: active traces whose claims disagree."""
        return self._json("conflicts", *self._store_args())

    def verify_ledger(self) -> dict:
        """Verify the hash chain and head file. ``ok`` is true iff ``errors`` is empty."""
        return self._json("verify-ledger", *self._store_args())

    def get(self, trace_id: str) -> dict:
        """Fetch one trace by id. Raises ``HikmahError`` when the id is unknown."""
        return self._json("get", *self._store_args(), trace_id)
