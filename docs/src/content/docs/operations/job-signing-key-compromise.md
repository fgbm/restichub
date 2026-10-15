---
title: Job signing key compromise
description: What an operator does when the job signing key leaked.
---

The control plane signs each job with the current job signing key. A person who holds this key can make jobs that the agents accept. Use this procedure when the key, a backup of the control plane that holds it, or the key encryption key (KEK) leaked (T13, ADR-0030).

## Before you start

- Learn the time of the leak. Read the audit log of the platform for the actions `signing_key_rotate` and `signing_key_retire`.
- Check that the control plane is reachable by the agents. The procedure sends the signed update over the open streams.
- If the KEK leaked, replace the KEK first. The new KEK wraps all keys again.

## Procedure

Run one of the two commands.

In the multi tenant mode, a Platform Admin calls the API:

```sh
curl -X POST https://<control-plane>/api/v1/platform/signing-keys/compromise \
  -H "Authorization: Bearer <platform-admin-token>"
```

In every mode, run the command on a host that has the configuration file and the KEK of the control plane:

```sh
restichub-cp compromise-signing-key --config /etc/restichub/cp.toml
```

The command prints the identifier of each key that it retired for a compromise.

## What the command does

1. The next key becomes the current key.
2. A new next key is created.
3. The old current key retires at once with the reason `compromise`. A key that waited in its overlap retires with it.
4. The control plane writes the audit records `signing_key_compromise` and `signing_key_retire`.
5. Each control plane instance sends a signed update to its open streams within 2 seconds. The signature comes from the new current key, which the agents already hold. The update names the retired keys as revoked. The agents refuse each job of the old key from then on.

If the next key leaked too, run the command a second time.

## Agents that were offline

An agent that was offline does not hold the new current key. The re-sync chain never carries a compromised key, so the agent cannot verify the update. The agent rejects each job with the reason "unknown key". Register such a host again with a new enrollment token. See [Register an agent](register-an-agent.md).

## Result

- The old key signs no job that an agent accepts.
- The installation has a new current key and a new next key.
- The audit log has one record for the compromise.
