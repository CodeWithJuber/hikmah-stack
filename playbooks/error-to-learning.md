# Playbook: Error -> Durable Learning

A system that merely apologizes does not learn.

1. Capture the failed expectation and observed outcome as separate traces.
2. Identify the earliest point where the failure could have been detected.
3. Classify the failure: knowledge, memory, reasoning, tool, execution, verification, policy, or handoff.
4. Create the smallest durable correction: test, rule, memory correction, acceptance criterion, or playbook update.
5. Re-run the original case.
6. Add a nearby counterexample so the fix does not overfit one incident.
7. Record whether the correction actually reduced recurrence.
