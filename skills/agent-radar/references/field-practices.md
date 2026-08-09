# Field Practices — Agent Radar

Operator-facing verbs for the AI age.

## Part I — Machine Failure Modes

### The Loop
- Write the smallest executable version of the task before asking the agent anything.
- When the agent repeats a failed approach twice, stop prompting and change state: rewind, reset, restart clean.
- Set a repetition tripwire: same action three times means locked; halt the session.
- Write the definition-of-done before the final stretch; forbid scope additions once in sight.
- Gate every investigation with: what will I do differently given each possible answer?

### Blind Judgment
- Stop feeding more input to a misjudging system; examine the judging layer.
- Write evaluation criteria before generation, not after.
- Run at least two review passes with different instruments.
- When evaluating a new tool, list its effects, not its category.
- Keep one personal standard you never outsource: some outputs you reject on sight.

### Missing Common Sense
- Test outputs against the physical and practical world, not against other text.
- Convert instructions into checks the model must pass, not phrases it can parrot.
- Before reviewing output, write down what you're hoping it says; discount accordingly.
- Ground every consequential AI output in one real-world observation before shipping.
- Keep integration human: read the source, not only the summary.

### Recombination
- Label your own work honestly: new combination, or new category?
- Attach a falsifiable benchmark to every capability claim you make or buy.
- Challenge "it's just autocomplete" by attempting replication at equal quality.
- Hunt for false fusions in your domain; the split is the opportunity.
- Separate generation from verification in every pipeline.

### Training-Data Dependency
- Audit training data and conventions by grounding, not by lineage.
- When better guidance arrives, update — even when it invalidates your history.
- Pre-register kill criteria for every model, approach, and belief you adopt.
- Feed systems verified external input: real tests, real users, real measurements.
- Invest in judgment the way you invest in compute — deliberately and recurrently.

### Confident Guessing
- Tag every AI output: known, inferred, or guessed. No tag, no trust.
- Never let a confidence score substitute for verification.
- Open the source: every citation, every package, every quote.
- Before any verdict, ask: have I encompassed this, or am I guessing?
- Ship "unknown" as a first-class output; refuse filled-in guesses.

## Part II — Workflow Failure Modes

### Reinventing the Wheel
- Inventory what the repo already carries before any generation task.
- Search the registry for a maintained package before writing any new capability.
- Route unknowns to maintainers, documentation, and domain experts before prompting.
- Write reuse-first into the instruction file.
- Import the community standard over the local custom.

### Over-Complication
- Review every pipeline with the 2 a.m. test: could a tired teammate reconstruct it?
- Count steps, files, and tokens per shipped result monthly; cut whatever grows.
- Verify momentous outputs in pairs or solo inside a protected hour — never in a crowd.
- Budget tokens, abstractions, and infrastructure at the standing middle.
- Calibrate the workflow to this quarter's real capacity.

### The Memory Hole
- Externalize every decision, TODO, and constraint at each handoff.
- Decide the degradation ladder before the journey: canonical doc, working notes, committed artifacts, escrowed state.
- Pin standing instructions adjacent to the reward surface: merge button, deploy script, demo checklist.
- Checkpoint before compaction; diff the summary against the session.
- Keep canonical state under your control; agents work on copies.

## Part III — Relationship Failure Modes

### The Flattery Engine
- Discount unsolicited agreement; weight a tool's dissent three times higher than its praise.
- Strip the compliments from the transcript before judging the answer.
- Inspect structure, not surface: sources, logic, and what the output refuses to say.
- Log praise-to-performance: how often was it flattering, and how often was it right?
- Probe with a deliberately wrong premise; watch whether the tool follows you off the cliff.

### Slop Economics
- Judge every artifact at six-month distance: still referenced, still running, still standing?
- Define good before opening the firehose; never let volume set the standard.
- Label machine involvement wherever margin depends on hiding it.
- Test output at arrival-distance: could someone act on it without asking you anything?
- Run the five-layer audit quarterly on your own output.

### The Black Box
- Contract explanation rights before deployment; opacity needs a declassification date.
- Run discrepancy audits: rephrase, repeat, cross-examine across sessions; log every contradiction.
- Build verification and rollback that never require internals.
- Prefer vendors whose top layer bears tamper-cost.
- Demand creator-side artifacts: evals, training provenance, changelogs.

### Unwanted Costs
- Attach an exception clause to every autonomous plan — caps, exits, tripwires.
- Run postmortems in the order: loss seen → blame burned out → transgression named → substitute specified.
- Calibrate spend to the standing middle: neither starved nor flooded.
- Circulate what works: fixes, prompts, evals. Track what returns.
- Meter haste: add friction exactly where error compounds — production, payments, deletions.

### Values & Trust
- Test for re-ranked priorities: does the agent optimize what it measures over what it was told?
- Keep a named human bearer of the trust on every deployment; no orphaned autonomy.
- Encode invariants in permissions and code, never in prompts alone.
- Write the mandate; bound the action space to it; name the watcher for after the session.
- Keep the agent's constitution to kernel size: three commands, three prohibitions.

## Part IV — The Human's Answer

### What Remains for Humans
- Choose what to start, where to aim, when to stop; delegate none of the three.
- Do one meaningful task unassisted daily; skill is interest on striving.
- Write the one-sentence purpose before the prompt.
- Schedule learning reps the machine cannot do for you: read the paper, debug the failure.
- Audit quarterly what the tool's help cost your own capability.
