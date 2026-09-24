# Security Policy

Hikmah Stack is primarily an instruction package. It intentionally ships no credentials, remote network service, or privileged MCP server.

## Trust boundary

Plugin hooks are executable behavior and deserve separate review.

- **Claude Code** runs two Stop hooks: the command hook `sh "${CLAUDE_PLUGIN_ROOT}/hooks/truth_gate.sh"` and a prompt-based review. **Codex** runs the command hook only. Codex asks the user to trust plugin hooks before running them.
- `hooks/truth_gate.sh` runs, in order: the plugin's `bin/hikmah`, a `hikmah` found on `PATH`, then `python3 hooks/truth_gate.py`, and finally allows. A `hikmah` on `PATH` is trusted as the user's own install; pin it or ship `bin/hikmah` if that matters in your environment.
- The launcher **never compiles code** at stop time. Earlier versions fell back to `cargo run`, which reads cargo and rustup configuration (for example `.cargo/config.toml` runners or `rust-toolchain.toml`) from the user's project directory, so an untrusted repository could run code at the end of every turn. That path is removed.
- The launcher always exits 0 with a JSON object. A missing, stale, or failing binary allows the stop rather than blocking or looping the host.
- **Network:** nothing leaves the machine unless a decision engine is explicitly selected (`--engine jev`, or `HIKMAH_HOOK_ENGINE=jev` for the hook) and `TYPESAFE_API_KEY` is set. The last assistant message (up to 8,000 characters) is then sent to TypeSafe. A request whose text (state, instructions, options, or labels) matches common credential shapes is refused before any call. The key is never logged. Build with `--no-default-features` to remove network code entirely.

## Reporting

Do not publish exploitable security details before maintainers have had a reasonable chance to investigate. Open a private GitHub security advisory when the repository supports it, or contact the maintainer through the GitHub profile associated with this project.

## Secrets

Never commit API keys, tokens, passwords, private keys, session cookies, production database URLs, or customer data. CI validates structure but is not a substitute for dedicated secret scanning.


## Cognitive memory security

Persistent memory creates additional threats: poisoning, scope bleed, stale/superseded activation, secret retention, and provenance laundering. The reference TraceWeave store rejects `sensitive` persistence by default. Deployments that need sensitive durable memory should provide encrypted storage and deletion semantics appropriate to their environment.

The hash chain covers the exact bytes of every record. On its own it detects a record edited, reordered, injected, or removed from the middle of the chain. The chain is **not keyed**: anyone who can write the ledger file can compute valid hashes, so the chain alone cannot tell a cut-off tail, a wholesale re-chain, or a record added at the end from a legitimate ledger. The `<store>.head` file adds detection of those three: truncation, rewrites, and records appended without a head update. Neither the chain nor the head file detects an append or a re-chain by someone who can also update or delete the head file, which sits next to the ledger with the same permissions. Only a head hash pinned outside the writer's reach (`hikmah verify-ledger --expect-head`) detects that.

Records that follow the head file without a head update are treated as unacknowledged. They come from a direct append by another program, an older binary, or a crash between the record write and the head update. `hikmah verify-ledger` fails and lists them under `unacknowledged`, and writes are refused until a person inspects them and runs `hikmah verify-ledger --accept-tail`. Writes are also refused while the ledger and its head file disagree (records removed or rewritten) until a person runs `hikmah verify-ledger --reset-head` after a deliberate repair. So the next ordinary write can neither approve a forged append nor hide a truncation. Inside a detected AI agent session (see below) the CLI refuses both `--accept-tail` and `--reset-head`: an agent that reads a refusal and runs the command it names must not be the one to approve a record it may have forged. Like the agent-session guard, this is not authentication. Reads (`recall`, `conflicts`, `calibration`) still include unacknowledged records; only verification and writes flag them. The chain does not encrypt content and is not a substitute for access control.

**Follow-up, not implemented: a keyed chain.** A v3 record could hash with `blake3::keyed_hash` and a key file (for example one named by `HIKMAH_LEDGER_KEY_FILE`, mode 0600). That only helps against writers who cannot read the key. `hikmah` runs as the same OS user as the agent that calls it, and that user can read its own 0600 file, so a useful key must be held by another principal, such as a separate account, an OS keychain that prompts, or a hardware token. The design also needs a rule against unkeyed records after keyed ones and a migration path for existing v1/v2 ledgers. It is the same missing piece as authenticated attestation (below), so it is left for that work rather than shipped as a key the agent can read.

`purge` is a tombstone: the content leaves recall but stays in the append-only ledger. Model-authored traces (`model:` sources) cannot be marked verified, cannot supersede other traces, and cannot resolve predictions. `--source` and `--verified` are otherwise what the caller claims. Inside a detected AI agent session (`CLAUDECODE`, `CLAUDE_CODE_*`, `CODEX_*`, `CURSOR_*`, `GEMINI_CLI`, `AI_AGENT`), the CLI stamps each `remember`, `predict`, and `outcome` with the locator `agent-session:<host>:<id>`, refuses `--verified`, and refuses `verify-ledger --accept-tail` and `--reset-head`; the kernel refuses any verified trace carrying that locator. This stops an agent from verifying its own memory, or approving records appended behind the ledger's back, by default. It is not authentication: detection reads environment variables, and a process that clears them passes as a person. A variable a person's own shell or IDE terminal sets can also be taken for an agent; the refusal names the variables, and `docs/MEMORY.md` ("Agent sessions") says how to proceed. Real verification needs authenticated principals, such as a signed `hikmah attest` with a key the agent cannot read, which this reference store does not implement.
