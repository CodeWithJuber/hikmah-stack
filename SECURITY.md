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

The hash chain covers the exact bytes of every record, so an edited, injected, reordered, or deleted record is detected. Truncation of the tail and a wholesale re-chain are detected only through the `<store>.head` file or a head hash pinned outside the writer's reach (`hikmah verify-ledger --expect-head`). Writes are refused while the ledger and its head file disagree, so an ordinary write cannot hide a truncation; `hikmah verify-ledger --reset-head` accepts the current state after a deliberate repair. Anyone who can rewrite both the ledger and its head can still rewrite history. The chain does not encrypt content and is not a substitute for access control.

`purge` is a tombstone: the content leaves recall but stays in the append-only ledger. Model-authored traces (`model:` sources) cannot be marked verified, cannot supersede other traces, and cannot resolve predictions. The CLI still lets any caller claim `--verified` for non-model sources; real verification needs authenticated principals, which this reference store does not implement.
