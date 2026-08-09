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

## Other agents

MCP's official documentation describes Agent Skills as portable instruction sets and documents manual installation for multiple agents. For a skills-only package such as Hikmah Stack, copying the required skill directories is often enough. If future capabilities require tools or external systems, an MCP server can provide a standardized integration layer.

References:
- https://modelcontextprotocol.io/docs/develop/build-with-agent-skills
- https://modelcontextprotocol.io/docs/getting-started/intro

## Important distinction: model vs host

A raw model is a prediction/reasoning engine. A host decides which files become context, which tools exist, whether hooks run, what permissions apply, and how plugins are installed. There is therefore no single repository format that can force itself into every model. Portability is achieved by standardizing the capability layer and adding thin adapters for runtimes.
