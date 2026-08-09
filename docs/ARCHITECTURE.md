# Architecture

Hikmah Stack uses a **portable-core / host-adapter** architecture.

## Portable core

`skills/` is the source of truth. Each capability is a self-contained `SKILL.md` with optional references. It should remain useful even if every host-specific manifest is removed.

## Host adapters

- `.codex-plugin/plugin.json` packages the skills for ChatGPT/Codex and points Codex to its command-based hook.
- `.claude-plugin/plugin.json` exposes the same skill tree to Claude Code; `agents/hikmah-orchestrator.md` is a Claude-specific convenience adapter.
- `.agents/plugins/marketplace.json` exists for repo-scoped OpenAI development/testing.
- `.claude-plugin/marketplace.json` exists for Claude marketplace installation and legacy-compatible local discovery.

## Why hooks differ

Hook semantics are runtime-specific. The Claude adapter uses a narrow prompt-based Stop review. Current Codex hook execution supports command handlers, so Codex uses `hooks/codex.json` and `hooks/truth_gate.py` instead. The Python gate is intentionally conservative and does not pretend to fact-check model output.

## Why there is no MCP server yet

The current product is instruction-heavy and does not need an external service or privileged action surface. Adding an MCP server solely to look “more agentic” would increase attack surface and maintenance without improving the core use case. Add MCP only when Hikmah Stack needs controlled tools, external state, authentication, or observable server-side behavior.
