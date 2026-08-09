# Co-Model Architecture

A **co-model** is a second cognitive system that supervises and complements a generative model. Hikmah's co-model is deliberately useful even with no neural network attached.

## Division of labor

### Generative proposal engine
Good at:
- language;
- hypothesis generation;
- compression and reframing;
- code/text proposals;
- interpreting unstructured inputs.

Not trusted to own:
- durable truth;
- provenance;
- irreversible authorization;
- commitment state;
- completion claims;
- deletion policy;
- self-evaluation as the sole judge.

### Hikmah kernel
Owns:
- memory trace lifecycle;
- hash-chained event history;
- structured claim conflicts;
- bounded contextual recall;
- deterministic decision scoring;
- parallel challenge lanes;
- completion hygiene;
- policy and evaluation records.

## The local-model roadmap

The kernel exposes a `ProposalEngine` trait. This lets us benchmark radically different proposal mechanisms behind one cognitive contract:

1. no-model deterministic baseline;
2. retrieval + rules + search;
3. small local language model;
4. state-space/recurrent model;
5. experimental non-attention architecture;
6. future learned memory module;
7. hybrid ensemble.

We do **not** declare one architecture “best” before measuring it. State-space models, recurrent test-time memory, and other recent approaches are valuable research inputs, but they remain learned models with their own limitations. Hikmah's innovation is to make memory integrity and judgment architecture independent of that competition.

## Non-neural reasoning path

A fully local no-neural configuration can perform:

- trace encoding;
- structured claim conflict detection;
- contextual recall;
- commitment recall;
- evidence/risk/human-impact/delivery lanes;
- weighted multi-criteria decisions;
- deterministic policies;
- bounded symbolic planning with Branch Loom;
- verification gates;
- outcome logging;
- future rule/search modules.

Natural-language generation can be delegated to any attached model without giving that model custody of memory or policy.

## Parallel cognition

Hikmah does not ask one monolithic model to “think harder.” Independent lanes inspect different failure surfaces in parallel:

```text
                    +-> Evidence lane ------+
Input + Recall -----+-> Memory lane --------+
                    +-> Risk lane ----------+--> Arbiter --> action state
                    +-> Human-impact lane --+
                    +-> Delivery lane ------+
```

This improves debuggability: a blocked action says **which lane blocked it and why**.
