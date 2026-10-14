# Awesome-list submission drafts

Prepared 2026-10-14. **Do not auto-submit** — these are drafts for a human to
review and submit (external PRs need the maintainer's own account).

## 1. st-365dev/awesome-agent-memory

- **List:** https://github.com/st-365dev/awesome-agent-memory
- **Section:** Open-source frameworks and engines
- **Rules (from their CONTRIBUTING):** one tool per pull request, alphabetical
  within each section, factual, no marketing language.
- **How to submit:** fork the repo, add the line below in alphabetical position
  within the section, open a PR titled `Add Hikmah Stack`.

**Entry line (Markdown):**

```markdown
- [Hikmah Stack](https://github.com/CodeWithJuber/hikmah-stack) - Deterministic Rust memory kernel for AI agents: typed traces with provenance, contradiction detection, append-only hash-chained ledger, and completion-claim screening. Installs via `cargo install hikmah-kernel`.
```

**Suggested PR body:**

```text
## Add Hikmah Stack

Hikmah Stack is an open-source (MIT) deterministic memory kernel for AI
agents, written in Rust. It stores typed memory traces with provenance,
confidence, and privacy class; detects contradictions between claims without
silently resolving them; keeps an append-only hash-chained ledger verifiable
with `hikmah verify-ledger`; and screens completion claims at host stop time
via a Truth Gate hook.

- Repo: https://github.com/CodeWithJuber/hikmah-stack
- Install: `cargo install hikmah-kernel` (published on crates.io)
- Tests: 180 unit/integration tests, CI on every push

Entry placed alphabetically in "Open-source frameworks and engines".
```

## 2. ARUNAGIRINATHAN-K/awesome-ai-agents-2026

- **Registry:** https://github.com/ARUNAGIRINATHAN-K/awesome-ai-agents-2026
- **How to submit:** open an issue using their `submit_agent.yml` template
  (or the `/submit` portal), which asks for ~10 structured fields. Draft
  values below (descriptions kept to one factual sentence each, no raw
  Markdown links inside fields per their validation rules).

**Draft field values:**

| Field | Value |
|---|---|
| Project name | Hikmah Stack |
| Repository URL | https://github.com/CodeWithJuber/hikmah-stack |
| Category | Agent Memory / Infrastructure |
| One-line description | Deterministic Rust memory kernel for AI agents with provenance-stamped traces, contradiction detection, and an auditable append-only ledger |
| Language | Rust |
| License | MIT |
| Install command | `cargo install hikmah-kernel` |
| Key differentiator | Model-authored traces can never be marked verified; contradictions stay visible instead of being silently resolved |
| Maturity | Working CLI + library, 180 tests, CI-green; v3.1.0 on crates.io |
| Homepage/docs | https://github.com/CodeWithJuber/hikmah-stack |

**Notes for the submitter:**
- If the registry asks for a tier/emerging tag: the project is small
  (1 star at time of writing) but fully working with published artifacts —
  tag honestly, don't inflate.
- Alternative list with overlapping scope: `caramaschiHG/awesome-ai-agents-2026`
  (PR-based, "Agent Infrastructure" style sections) — same entry line works.
