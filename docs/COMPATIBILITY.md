# Compatibility

## What is portable

The `skills/*/SKILL.md` layer is intentionally host-light. Any agent that implements Agent Skills or can load equivalent instruction bundles can reuse the core concepts.

## OpenAI

OpenAI plugins can package skills, MCP servers, and hooks. Hikmah Stack includes `.codex-plugin/plugin.json` and a development marketplace entry. Public OpenAI publication is a platform review/submission process, separate from making the GitHub repository public.

Current reference:
- https://developers.openai.com/plugins/concepts/plugins
- https://developers.openai.com/plugins/build/plugins

## Claude Code

The repository keeps a Claude Code manifest, marketplace metadata, a read-only orchestrator subagent, and a prompt-based completion review hook.

## Kimi

Kimi plugins bundle a root `kimi.plugin.json` manifest with a `skills/` directory. Hikmah Stack keeps the manifest at the repository root pointing at `./skills/`, so no repackaging or path rewriting is needed: the repo-level `docs/`, `playbooks/`, and `lenses/` paths referenced inside skills resolve in place. The manifest's `skillInstructions` tell the host when to route to each of the six skills.

## OpenClaw

OpenClaw supports compatible plugin bundles from several ecosystems and detects this repository through its existing `.codex-plugin/plugin.json` marker. Installing that bundle loads the manifest's `./skills/` root through OpenClaw's normal skill loader, preserving the same Codex, Claude Code, and Kimi manifests rather than introducing a competing native manifest.

```bash
openclaw plugins install git:github.com/CodeWithJuber/hikmah-stack --accept-capabilities
openclaw plugins inspect hikmah-stack
openclaw gateway restart
openclaw skills check
```

Review third-party source before accepting capabilities. A local packaging smoke test with OpenClaw 2026.8.2 identified the repository as `Format: bundle`, `Bundle format: codex`, and discovered all six skills as eligible.

The supported boundary is deliberately narrow:

- the portable `skills/` content is loaded;
- `hooks/codex.json` is detected as a declared bundle capability but is not executed because it is not an OpenClaw hook pack (`HOOK.md` plus `handler.ts` or `handler.js`);
- Claude `hooks/hooks.json` remains detect-only in OpenClaw;
- the Rust kernel is not automatically built, started, or exposed as tools;
- no native OpenClaw plugin, MCP server, model provider, or autonomous runtime is claimed.

Do not enable or translate either existing hook merely to make it run under another host. A future OpenClaw hook should be added only with an explicit event contract, tests, and an honest security review.

Current references:
- https://docs.openclaw.ai/plugins/bundles
- https://docs.openclaw.ai/cli/plugins
- https://docs.openclaw.ai/tools/skills

## Other agents

MCP's official documentation describes Agent Skills as portable instruction sets and documents manual installation for multiple agents. For a skills-only package such as Hikmah Stack, copying the required skill directories is often enough. If future capabilities require tools or external systems, an MCP server can provide a standardized integration layer.

References:
- https://modelcontextprotocol.io/docs/develop/build-with-agent-skills
- https://modelcontextprotocol.io/docs/getting-started/intro

## Important distinction: model vs host

A raw model is a prediction/reasoning engine. A host decides which files become context, which tools exist, whether hooks run, what permissions apply, and how plugins are installed. There is therefore no single repository format that can force itself into every model. Portability is achieved by standardizing the capability layer and adding thin adapters for runtimes.
