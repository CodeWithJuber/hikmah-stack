# Hikmah Stack

**Judgment infrastructure for agentic AI.**

Hikmah Stack is an open-source capability pack for AI agents that need better decision structure, stronger evidence discipline, clearer AI-failure diagnosis, and more reliable delivery. The core is portable `SKILL.md` content. Host-specific manifests adapt that same core to **ChatGPT/Codex** and **Claude Code**.

> Version **2.0.0** · MIT License · Maintained by [Juber Shaikh](https://github.com/CodeWithJuber)

## Why this is not “Claude-only”

A language model by itself does not install a repository, execute hooks, or discover local tools. Those extension behaviors belong to the **host/runtime** around the model. Hikmah Stack therefore separates the portable brain from the host adapter:

```text
                 ┌──────────────────────────────┐
                 │        Hikmah Stack          │
                 │  portable SKILL.md knowledge │
                 └──────────────┬───────────────┘
                                │
              ┌─────────────────┼─────────────────┐
              │                 │                 │
      ChatGPT / Codex      Claude Code      Other skill-aware
      .codex-plugin/       .claude-plugin/  agents / MCP hosts
```

OpenAI's current plugin architecture supports plugins containing skills, MCP servers, and hooks, with public plugins shared across ChatGPT and Codex. MCP and Agent Skills provide a broader portability layer for other compatible hosts. See [Compatibility](docs/COMPATIBILITY.md).

## The capability stack

| Capability | Job |
|---|---|
| **Operator Core** (`operator-core`) | Human judgment, leadership, pressure, ethics, communication, recovery, stewardship |
| **Agent Radar** (`agent-radar`) | Detect hallucination, loops, sycophancy, context loss, slop, opacity, cost leakage, brittle automation |
| **Decision Forge** (`decision-forge`) | Compare options, classify uncertainty, test reversibility, weigh risk, fairness, commitments, action |
| **Ship Guard** (`ship-guard`) | Acceptance criteria, verification, logging, rollback, handoffs, completion discipline |
| **Hikmah Orchestrator** (`hikmah-orchestrator`) | Route across the other capabilities and synthesize one coherent plan |
| **Truth Gate** | Conservative completion hygiene. Host-specific implementation, never a fake “fact checker” |

## Design doctrine

1. **Evidence before confidence.** Consequential claims need proportionate checking.
2. **Justice and trust are decision gates.** Optimize without quietly transferring unfair cost or breaking commitments.
3. **Uncertainty is data.** Mark verified evidence, inference, and unknowns separately.
4. **Human accountability remains named.** AI may assist judgment; it does not inherit moral or professional responsibility.
5. **Action beats framework theater.** Every framework should change a choice, control, or deliverable.
6. **Durability beats generation volume.** Judge work by downstream usefulness and rework avoided.
7. **Consult knowledge before improvising.** Current primary sources and qualified specialists outrank confident memory.

## Install / test

### ChatGPT + Codex

The repository includes the required `.codex-plugin/plugin.json` plus a repo-scoped marketplace file for development.

```bash
codex plugin marketplace add CodeWithJuber/hikmah-stack
```

Public-directory publication is a separate OpenAI review/submission step after repository release.

### Claude Code

```text
/plugin marketplace add CodeWithJuber/hikmah-stack
/plugin install hikmah-stack@hikmah-stack
/reload-plugins
```

Local development:

```bash
claude --plugin-dir .
```

### Other Agent Skills clients

Copy the desired folder from `skills/` into your agent's supported skills directory, or use the repository as a source if the host supports Git-backed skills. The skill content intentionally avoids depending on Claude-specific tool names.

## Example requests

- “Use Decision Forge to compare these three architecture options.”
- “Run Agent Radar on this coding session and tell me why it keeps looping.”
- “Apply Ship Guard before I call this migration production-ready.”
- “Use Operator Core for this team conflict, but separate values from facts.”
- “Use Hikmah Orchestrator across this entire situation and give me a single action plan.”

## Safety boundaries

Hikmah Stack is decision-support infrastructure, not a substitute for current medical, legal, financial, security, religious, or other qualified professional guidance. Higher stakes require stronger evidence and appropriate human review. The project does not claim its principles are infallible, and it must not label a claim “verified” merely because a model generated it.

## Repository map

```text
.codex-plugin/             OpenAI ChatGPT/Codex manifest
.claude-plugin/            Claude Code manifest + marketplace
.agents/plugins/           OpenAI repo marketplace for local testing
skills/                    Portable capability core
agents/                    Claude-specific subagent adapter
hooks/                     Claude prompt hook + Codex command hook
scripts/                   Validation tooling
docs/                      Architecture, compatibility, ethics, evidence
.github/                    CI and contribution templates
```

## Validate

```bash
python3 scripts/validate.py
python3 hooks/truth_gate.py <<<'{"stop_hook_active":false,"last_assistant_message":"Done. TODO: upload it later."}'
```

CI runs the same structural checks on every push and pull request.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md), [GOVERNANCE.md](GOVERNANCE.md), and [SECURITY.md](SECURITY.md). New empirical claims belong in [docs/EVIDENCE.md](docs/EVIDENCE.md) with date, source, scope, and limitations.

## License

MIT. See [LICENSE](LICENSE).
