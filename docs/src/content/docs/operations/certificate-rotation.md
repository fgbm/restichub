---
title: Agent certificate rotation
description: How an agent renews its certificate, and what to do when a certificate expired.
---

Each agent has a client certificate for the mTLS stream to the control plane (ADR-0026). The certificate has a limited lifetime. The agent renews it by itself, so an operator does nothing in normal use.

## Lifetime and rotation window

- The default lifetime is 30 days. Set `RESTICHUB_AGENT_CERT_LIFETIME_SECS` (or `agent_cert_lifetime_secs` in the configuration file) to change it. The value is in seconds. The lowest value is 600 and the highest value is 7 776 000 (90 days).
- The agent asks for a new certificate when two thirds of the lifetime have passed. With the default lifetime this is day 20.
- The control plane accepts a request after half of the lifetime. The gap between the half and the two thirds absorbs a difference between the clocks of the two sides.
- A stream gets a second signed rotation only after a quarter of the lifetime has passed, and at most 30 seconds.

## What happens in a rotation

1. The agent makes a new key pair. The new private key stays in the memory of the agent until the answer arrives.
2. The agent sends a certificate signing request (CSR) over the open stream.
3. The control plane checks the request. The agent must be active, and it must present its enrolled certificate. The request must hold a new public key. The control plane signs a certificate for the identity of the presented certificate, never for an identity in the request.
4. The control plane keeps the new certificate as pending and writes the audit record `agent_certificate_rotate`.
5. The agent checks the answer against the root CA that it pinned at enrollment. It stores the new key, the new certificate and the new chain in the protected configuration directory, and it sends the confirmation.
6. The control plane makes the new certificate the enrolled one. It refuses the old certificate serial from this moment.

The old certificate stays valid until step 6. An agent that connects with the new certificate before it sent the confirmation also makes the new certificate the enrolled one.

The open stream keeps running during the rotation. A job that runs on the agent continues. The agent uses the new certificate at its next connection.

## Files on the agent

The agent writes the new files with the same owner-only rules as at enrollment. It writes them in the same directory as the rest of its state. A power loss in the middle of a rotation does not break the identity. At its next start the agent finishes the rotation or drops it. The key always matches the certificate.

## What the Hosts screen shows

The Hosts screen and the agents API show the certificate expiry and the time of the last rotation of each host. A host with an expiry in less than 3 days needs attention: its agent did not rotate.

## When the agent did not rotate

The agent rotates only over an open stream. If the agent stays offline until its certificate expires, it cannot connect and it cannot rotate. The log of the agent then shows this message:

```text
the agent certificate is expired and cannot rotate; register the agent again with a new enrollment token
```

## Register an agent again after an expired certificate

The control plane refuses a connection with an expired certificate. Register the host again:

1. Make a new enrollment token in the Web UI. Use the same labels as before.
2. Run the registration command on the host with `--force`:

   ```sh
   restichub-agent register --url https://<control-plane> --token <token> --force
   ```

3. Restart the agent service.

The registration makes a new agent identity. The old host entry stays in the list with its old certificate. Revoke the old entry after the new host appears.

## Related pages

- [Register an agent](register-an-agent.md)
- [Agent revocation](agent-revocation.md)
