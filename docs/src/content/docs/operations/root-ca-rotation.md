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

- Plan a maintenance window. Every agent must register again, so no agent runs a job until you finish.
- Copy the control plane database and the key encryption key (KEK).
- Make one enrollment token for each host. See [Register an agent](/restichub/getting-started/add-a-host/).

## Procedure

1. Stop the control plane.
2. Connect to the database as the owner role.
3. Remove every row of the `ca_keys` table.
4. Start the control plane. It creates a new root CA and a new issuing CA.
5. Register each agent again with `--force` and a new enrollment token. The agent pins the new root CA.
6. Revoke the old host entries after the new entries appear. See [Agent revocation](/restichub/guides/agent-revocation/).

## Result

- The agents trust the new root CA.
- The control plane signs the agent certificates with the new issuing CA.
- A start with an expired root CA refuses with a message. Follow this procedure then.
