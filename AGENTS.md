# Agent instructions for this repository

Preserve the portable-core / host-adapter architecture. `skills/` is the source of truth. Do not add a dependency, MCP server, hook, network call, or privileged action merely to make the project look more sophisticated.

Before declaring a change complete:
1. Run `python3 scripts/validate.py`.
2. Inspect changed files.
3. Verify any factual compatibility claim against current primary documentation.
4. State limitations rather than inventing support that was not tested.
