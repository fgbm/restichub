---
title: Job signing key recovery
description: How an operator recovers a control plane start with every job signing key retired.
---

:::caution
ResticHub is in the design phase. This procedure is planned for 1.0. It is not implemented yet.
:::

The control plane signs each job with the current job signing key. The key set holds a current key and a next key. The control plane retires an old key after all known online agents acknowledged the new set, or at most 7 days after the rotation starts.

A start with every key retired refuses with the message "every stored job signing key is retired; the installation has no active key set". The control plane does not replace the revoked keys by itself, because a silent new set would hide a possible compromise.

## When this happens

- Every row of the `signing_keys` table holds a retirement time.
- No row is the current key and no row is the next key.

## Before you start

- Copy the control plane database.
- Confirm that no key retired for a compromise. A compromise needs a key rotation and an agent re-enrollment. Ask the security owner.

## Procedure

1. Stop the control plane.
2. Connect to the database as the owner role.
3. Remove every row of the `signing_keys` table.
4. Start the control plane. It creates a new current key and a new next key.
5. Every agent re-enrolls. The enrollment gives the agent the new key set.

## Result

- The installation has a new key set.
- The control plane signs the jobs with the new current key.
- An agent that did not re-enroll yet rejects a job with the reason "unknown key".
