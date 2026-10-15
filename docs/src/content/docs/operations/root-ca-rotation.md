---
title: Root CA rotation
description: How an operator replaces the installation root certificate authority.
---

:::caution
No command does this procedure. The agent keeps the root certificate that it pinned at enrollment, and no function updates it. Register every agent again after a root CA change.
:::

The installation has one root certificate authority (CA). The root CA signs one issuing CA. The agents trust the root CA. A rotation of the root CA changes the trust of every agent.

## When to rotate the root CA

Rotate the root CA in these cases:

- The root certificate expires soon. The root CA lives for 10 years.
- The root key is possibly compromised.

The control plane rotates the issuing CA by itself. Do not rotate the issuing CA by hand.

## Before you start

- Plan a maintenance window. During the rotation every agent must trust the new root CA.
- Keep the old root certificate. An agent can still present an old chain.
- Copy the control plane database and the key encryption key (KEK).

## Procedure

1. Generate the new root CA with a lifetime of 10 years.
2. Generate a new issuing CA and sign it with the new root CA.
3. Add the new root certificate to the trust store of every agent.
4. Wait until every agent reports the new trust store.
5. Replace the root CA row in the control plane database.
6. Restart the control plane. At start the control plane checks the root CA.
7. Remove the old root certificate from the trust store of every agent.
8. Remove the old root CA row.

## Result

- The agents trust the new root CA.
- The control plane signs the agent certificates with the new issuing CA.

A start with an expired root CA refuses with a message. Follow this procedure then.
