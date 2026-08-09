# Governance

Hikmah Stack is maintainer-led and evidence-aware.

## Decision policy

The maintainer may merge changes that improve correctness, portability, safety, clarity, or downstream usefulness. Changes that materially alter an operating principle should explain the failure mode they solve and the tradeoff they introduce.

## Evidence changes

Empirical claims require a dated source, study/context scope, and at least one limitation. Primary sources are preferred. Dated measurements must not be promoted into timeless laws.

## Compatibility changes

Host-specific behavior belongs in adapters. Avoid contaminating portable skills with vendor-specific commands unless the skill is explicitly host-scoped.

## Breaking changes

Rename or remove public skills only in a major release, with migration notes in `CHANGELOG.md`.
