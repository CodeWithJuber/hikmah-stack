# Security Policy

Hikmah Stack is primarily an instruction package. It intentionally ships no credentials, remote network service, or privileged MCP server.

## Trust boundary

Plugin hooks are executable/runtime behavior and deserve separate review. The Codex hook prefers the Rust `hikmah hook` runtime and keeps a small Python implementation only as a zero-install compatibility fallback. The Claude completion hook is prompt-based. Review both before enabling them in a sensitive environment.

## Reporting

Do not publish exploitable security details before maintainers have had a reasonable chance to investigate. Open a private GitHub security advisory when the repository supports it, or contact the maintainer through the GitHub profile associated with this project.

## Secrets

Never commit API keys, tokens, passwords, private keys, session cookies, production database URLs, or customer data. CI validates structure but is not a substitute for dedicated secret scanning.


## Cognitive memory security

Persistent memory creates additional threats: poisoning, scope bleed, stale/superseded activation, secret retention, and provenance laundering. The reference TraceWeave store rejects `sensitive` persistence by default. Deployments that need sensitive durable memory should provide encrypted storage and deletion semantics appropriate to their environment.

The hash chain detects ledger tampering; it does not encrypt content and is not a substitute for access control.
