# Contributing

Thanks for improving Hikmah Stack.

## Before a pull request

1. Keep the portable core under `skills/` vendor-light.
2. Put host-specific behavior in the appropriate adapter or hook file.
3. Add or update evidence notes for empirical factual claims.
4. Run `cargo test --workspace` and `cargo run -p hikmah-kernel -- validate --root .`.
5. Inspect the diff for secrets, generated junk, unsupported completion claims, and accidental breaking renames.

## Skill quality bar

A skill should state when it triggers, what problem it solves, boundaries, the workflow, and what successful output looks like. Prefer concrete controls and decision-changing guidance over motivational prose.

## Evidence policy

Prefer primary research or first-party technical documentation. Record the date and limitation. Do not smuggle a historical benchmark into a universal claim.


## Cognitive kernel changes

Kernel changes should include a test for the invariant they alter. If proposing a graph, embedding model, neural module, external database, or MCP server, include the measured failure in the current design and the evaluation that would justify the added complexity.
