# hikmah — Python bridge

Stdlib-only Python client for the Hikmah deterministic memory kernel. It shells
out to the `hikmah` JSON CLI and returns parsed dicts — no native bindings, no
dependencies beyond the standard library.

## Setup

Build the binary once, then point the client at it:

```bash
cargo build --release -p hikmah-kernel   # produces target/release/hikmah
export HIKMAH_BIN=$PWD/target/release/hikmah
```

The binary is located via `binary=...`, else `HIKMAH_BIN`, else `PATH`.

## Usage

```python
from hikmah import Client, HikmahError

db = Client(store=".hikmah/memory.jsonl")  # store= optional; defaults to the CLI default
db.init()

trace = db.remember("Deploys freeze on Fridays", kind="belief",
                    source="human:juber", tags=("ops",))["trace"]
print(trace["id"])

for hit in db.recall("friday deploys"):
    print(hit["trace_id"], hit["score"], hit["trace"]["content"])

print(db.verify_ledger()["ok"])   # True iff the hash chain verifies
print(db.get(trace["id"])["content"])
print(db.conflicts())             # unresolved claim conflicts
```

Failures raise `HikmahError` with the CLI's stderr attached.

## Tests

```bash
cd python && python3 -m unittest discover -s tests
```

Tests run against the locally built debug binary (`HIKMAH_BIN` or
`<repo>/target/debug/hikmah`).
