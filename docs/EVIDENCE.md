# Evidence Notes

Verified: **2026-08-09**. These are snapshots from specific studies or surveys, not universal constants. The plugin should prefer the operational lesson over repeating a number without its scope.

## Package hallucinations in code-generating LLMs

A USENIX Security 2025 study evaluated 16 code-generating LLMs across 576,000 generated code samples. It reported average hallucinated-package rates of at least **5.2% for commercial models** and **21.7% for open-source models** in its tested settings, with 205,474 unique hallucinated package names.

Source: https://www.usenix.org/conference/usenixsecurity25/presentation/spracklen

**Use carefully:** This does not mean a fixed percentage of every package recommendation from every current model is hallucinated. Models, prompts, languages, and tool grounding differ. The durable rule is: verify package existence and provenance before installation.

## AI assistance and experienced open-source developer productivity

METR's July 2025 randomized controlled trial found that experienced open-source developers in its sample took **19% longer** on assigned issues when allowed to use early-2025 AI tools, while after the study they believed AI had sped them up by about **20%**.

Source: https://metr.org/blog/2025-07-10-early-2025-ai-experienced-os-dev-study/

In February 2026, METR explicitly cautioned against treating that result as a timeless estimate: its newer experiment suffered selection and measurement problems, and METR said it was likely developers were more sped up by newer tools, but the new data was too biased to estimate the effect reliably.

Source: https://metr.org/blog/2026-02-24-uplift-update/

**Use carefully:** The durable rule is to measure local productivity and rework rather than infer current ROI from one historical study.

## "Workslop" survey snapshot

BetterUp Labs, in partnership with the Stanford Social Media Lab, reported from a September 2025 survey of U.S. desk workers that **40%** had received AI-generated low-substance work in the prior month, with an estimated **$186 monthly cost per employee** from time spent resolving such incidents.

Source: https://www.betterup.com/workslop

**Use carefully:** This is survey-based organizational research, not a universal causal estimate. The durable rule is to evaluate downstream recipient cost, not merely generation speed.

## Evidence maintenance rule

When adding a statistic:

1. Link the primary study or first-party research page.
2. State the population, date, and study design.
3. State at least one limitation or non-generalization.
4. Never turn a dated measurement into a universal law.
5. Re-check time-sensitive claims before a release.

## Cognitive architecture research

Human-memory and alternative-model research used to motivate TraceWeave design is maintained separately in [RESEARCH.md](RESEARCH.md), with explicit engineering lessons and limitations.

## Staleness updates (audited 2026-08-13)

Applying rule 5 of the maintenance rule above ("re-check time-sensitive claims before a release").

### Package hallucination — denominator footnote and a 2026 replication

The 5.2% / 21.7% figures above are at the **code-sample** level (576,000 generated samples). The same
Spracklen et al. study also reports a **package-mention** level aggregate: those samples contained
2.23 million package recommendations, of which 440,445 (**19.7%**) were hallucinated, with 205,474
unique fabricated names. A separate sub-experiment (500 prompts x 10 reruns) found 43% of hallucinated
names recurred in all ten reruns — the property that makes slopsquatting farmable. Both framings are
correct; cite the denominator you mean.

An independent 2026 replication on five frontier models (Claude Sonnet 4.6, Claude Haiku 4.5,
GPT-5.4-mini, Gemini 2.5 Pro, DeepSeek V3.2) reports overall rates compressed to **4.62%-6.10%**.

Source: https://arxiv.org/abs/2605.17062

**Use carefully:** that replication is a **single-author, non-peer-reviewed preprint** whose author
flags an uncontrolled training-data-contamination confound. Treat it as a caution against reusing the
2024-cohort percentages as current, not as a settled replacement. The durable rule is unchanged:
verify package existence and provenance before installation.

### AI-hallucinated citations in court decisions

Databases tracking court decisions involving AI-hallucinated citations are **continuously updated**, so
any hardcoded count is stale by construction. A direct check on 2026-08-13 returned 1,870 cases.
Always state the count with the date you checked it, or cite the tracker without a number.

Source: https://www.damiencharlotin.com/hallucinations/
