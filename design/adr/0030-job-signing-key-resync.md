# ADR-0030: Re-sync of the job signing key set

- Status: accepted
- Date: 2026-10-13
- Requirements: FR-PR-11

## Context

ADR-0026 fixes the job signing key rotation. The control plane sends the key set to an agent over the stream. An agent that was offline longer than one rotation does not hold the current key. The agent cannot verify a signed key update, because it does not hold the signer key.

## Decision

- The control plane keeps every retired key row. It therefore holds the whole chain of keys.
- A re-sync sends the chain of `KeyUpdate` messages, one step per rotation. Each message is signed by the key that the agent already holds. The agent reaches the current key in a few steps.
- A rotation refuses while an earlier rotation still holds its overlap. One overlap row exists at a time, so the chain stays short.
- A key that the control plane retired for a compromise is never part of the chain. The chain carries the keys of a normal rotation only. An agent that needs a compromised key re-enrolls instead.
- The control plane signs each chain step once, at the rotation that creates the key, and stores the step in the key row (2026-10-14). The delivery reads the stored steps, so it needs no private key of a retired row. It skips a step that does not decode, and it always delivers the current step. The chain carries the newest 64 steps.
- The row of a key holds the reason for its retirement: `rotation` or `compromise` (2026-10-14). A key whose reason is `compromise` is not part of the chain.
- The delivery of the chain and the acknowledgement are issue #50.
- The enrollment delivers the current key and the next key (2026-10-14). An agent that enrolls holds the current key at once, so it verifies a job before the first rotation.

## Consequences

- An agent more than one rotation behind reaches the current key without a re-enrollment.
- The retired rows stay in the database. The chain is built from them.
- A compromise uses the enrollment path, because the chain must not carry a compromised key.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| A re-enrollment for every lagging agent | A re-enrollment needs a token and an operator action. The chain uses the retained rows and needs neither. |
| The chain also carries a compromised key | A compromised key must not sign a later update. The agent would then trust the attacker. |
