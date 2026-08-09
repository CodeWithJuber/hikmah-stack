# Research Notes: Human Memory and Alternative Model Architectures

Verified: **2026-08-09**. These notes inform design hypotheses; they are not claims that Hikmah reproduces human cognition.

## Dynamic/selective memory representations

Tomé et al., *Nature Neuroscience* (2024), combined computational and mouse experiments and reported that memory engram composition/selectivity changes with consolidation, with inhibitory activity/plasticity important for selectivity.

Source: https://www.nature.com/articles/s41593-023-01551-w

**Design lesson:** durable memory should support changing accessibility and explicit correction rather than freezing the first representation forever.

## Replay and distributed activation in humans

Huang et al., *Nature Communications* (2024), used simultaneous EEG-fMRI and reported fast sequential replay associated with hippocampal and medial-prefrontal activity/connectivity during mental simulation.

Source: https://www.nature.com/articles/s41467-024-51582-5

**Design lesson:** offline/reflective replay is a plausible inspiration for consolidation and cross-context learning, but software replay must remain evidence-preserving.

## Temporal structure in human hippocampal-entorhinal neurons

Tacikowski et al., *Nature* (2024), reported single-neuron representations related to temporal structure and replay in human hippocampal/entorhinal recordings.

Source: https://www.nature.com/articles/s41586-024-07973-1

**Design lesson:** temporal order and sequence deserve first-class representation; not every relation needs a static semantic edge.

## Memory construction and consolidation model

Spens & Burgess, *Nature Human Behaviour* (2024), proposed a computational account in which hippocampal replay and generative systems interact to reconstruct experiences and schemas.

Source: https://www.nature.com/articles/s41562-023-01799-z

**Design lesson:** recall should not be treated as byte-perfect playback. Software, however, can preserve provenance even when summaries/reconstructions are generated.

## Replay and planning

Jensen, Hennequin & Mattar, *Nature Neuroscience* (2024), presented a recurrent planning model connecting replay to planning behavior.

Source: https://www.nature.com/articles/s41593-024-01675-7

**Design lesson:** recall and planning can be coupled, but Hikmah keeps the memory ledger independent from the planner so a bad plan cannot rewrite its own evidence.

## Alternative learned sequence models

Sarrof, Veitsman & Hahn, NeurIPS 2024, analyzed linear state-space models and found strengths and limitations distinct from transformers on formal-language tasks.

Source: https://proceedings.neurips.cc/paper_files/paper/2024/hash/485e0981e81766248b61fd1ec43c118f-Abstract-Conference.html

**Design lesson:** “non-transformer” does not mean “solved.” Different learned architectures have different representational tradeoffs. The co-model contract should outlive them.

## Test-time neural memory

Behrouz, Zhong & Mirrokni introduced Titans (2024/2025), a neural long-term memory approach that learns at test time and was evaluated on long-context tasks.

Source: https://arxiv.org/abs/2501.00663

Later independent analysis reported promising memory effects alongside reproducibility and chunking limitations.

Source: https://arxiv.org/abs/2510.09551

**Design lesson:** learned memory can become an optional proposal/retrieval channel, but durable factual memory still needs provenance, conflict handling, and policy outside the learned weights.
