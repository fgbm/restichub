---
title: Job signing key compromise
description: What an operator does when the job signing key leaked.
---

The control plane signs each job with the current job signing key. A person who holds this key can make jobs that the agents accept. Use this procedure when the key, a backup of the control plane that holds it, or the key encryption key (KEK) leaked.

## Before you start

- Learn the time of the leak. Read the audit log of the platform for the actions `signing_key_rotate` and `signing_key_retire`.
- Check that the control plane is reachable by the agents. The procedure sends the signed update over the open streams.
- If the KEK leaked, replace the KEK first. The new KEK wraps all keys again.

## Procedure

Read the identifier of the current key. Run the command on a host that has the configuration file and the KEK of the control plane:

```sh
restichub-cp show-signing-key --config /etc/restichub/cp.toml
```

The command prints the identifier and does not change anything. Use this identifier as `<current-key-id>` below.

Run one of the two commands.

In the multi tenant mode, a Platform Admin calls the API:

```sh
curl -X POST https://<control-plane>/api/v1/platform/signing-keys/compromise \
  -H "Authorization: Bearer <platform-admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"expected_current_key_id": "<current-key-id>"}'
```

The request names the key that you believe is current. If another call changed the current key first, the control plane answers HTTP 409 and changes nothing. A second call with the same key identifier therefore retires nothing.

In every mode, run the command on a host that has the configuration file and the KEK of the control plane:

```sh
restichub-cp compromise-signing-key --config /etc/restichub/cp.toml --expected-key-id <current-key-id>
```

The argument `--expected-key-id` is required. If another call changed the current key first, the command stops and changes nothing. The command prints the identifier of each key that it retired for a compromise.

## What the command does

1. The next key becomes the current key.
2. A new next key is created.
3. The old current key retires at once with the reason `compromise`. A key that waited in its overlap retires with it.
4. The control plane writes the audit records `signing_key_compromise` and `signing_key_retire`.
5. Each control plane instance sends a signed update to its open streams within 2 seconds. The signature comes from the new current key, which the agents already hold. The update names the retired keys as revoked. The agents refuse each job of the old key from then on.

If the next key leaked too, read the new current key identifier with `show-signing-key` and run the command a second time with it.

A job in the status `sent` can hold a signature of a retired key. The control plane signs it again with the current key before it sends the job again. The agent never gets the old signature.

## Agents that were offline

An agent that was at most one rotation behind holds the new current key as its next key. When it connects, it gets the re-sync chain. The chain does not carry the compromised key, but it carries the step that the new current key signed. The agent verifies this step with the key that it already holds, learns the new next key and revokes the compromised key. It needs no new enrollment and it accepts the next job.

An agent that is two or more rotations behind, for example after two compromises, does not hold the new current key. It cannot verify the chain and it does not run a job. Register such a host again with a new enrollment token. See [Register an agent](/restichub/operations/register-an-agent/).

## Result

- The old key signs no job that an agent accepts.
- The installation has a new current key and a new next key.
- The audit log has one record for the compromise.
