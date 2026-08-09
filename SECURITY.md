# Security Policy

Hikmah Stack is primarily an instruction package. It intentionally ships no credentials, remote network service, or privileged MCP server.

## Trust boundary

Plugin hooks are executable/runtime behavior and deserve separate review. The Codex hook is a small local Python script that reads Stop-event JSON from stdin and emits only a continuation decision. The Claude completion hook is prompt-based. Review both before enabling them in a sensitive environment.

## Reporting

Do not publish exploitable security details before maintainers have had a reasonable chance to investigate. Open a private GitHub security advisory when the repository supports it, or contact the maintainer through the GitHub profile associated with this project.

## Secrets

Never commit API keys, tokens, passwords, private keys, session cookies, production database URLs, or customer data. CI validates structure but is not a substitute for dedicated secret scanning.
