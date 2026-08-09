# Lens: Model Independence

Whenever a design says “the model will remember/decide/check itself,” ask:

1. What state must survive a model swap?
2. What must be deterministic or auditable outside learned weights?
3. Which outputs are proposals versus authoritative state transitions?
4. Can a weaker/local/offline model still operate the core workflow?
5. Can two different models be compared behind the same interface?
6. What failure becomes invisible if the same model generates and judges the answer?

If a capability is essential to identity, memory integrity, policy, or audit, prefer a host/kernel mechanism over hidden model state.
