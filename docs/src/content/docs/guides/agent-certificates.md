---
title: Agent certificates and enrollment
description: How the agent renews its certificate, how to enroll a host again with its plans, and how to merge two records of one computer.
---

Each agent has a client certificate for the mTLS stream to the control plane. The agent renews it by itself. When the agent has no valid identity, or when its certificate expired, enroll the host again. The host keeps its plans, its labels and the references to its maintenance host.

## Hosts that wait for a connection

An agent that enrolled but did not open a stream has the status "Waiting for connection". The host is not online and not offline. The Overview counts it apart as "Hosts waiting for connection". The status changes to "Online" when the agent connects for the first time.

A host stays in this status when the service does not start or cannot reach the control plane. Run `status` on the host to read the last error (see [Register an agent](/restichub/reference/agent-commands/#troubleshooting)).

## Lifetime and rotation window

- The default lifetime is 30 days. Set `RESTICHUB_AGENT_CERT_LIFETIME_SECS` (or `agent_cert_lifetime_secs` in the configuration file) to change it. The value is in seconds. The lowest value is 600 and the highest value is 7 776 000 (90 days).
- The agent asks for a new certificate when two thirds of the lifetime have passed. With the default lifetime this is day 20.
- The control plane accepts a request after half of the lifetime. The gap between the half and the two thirds absorbs a difference between the clocks of the two sides.
- The control plane signs at most one rotation for each agent in 10 minutes, or in a quarter of the lifetime when that is shorter. The registry holds the time of the last request, so a reconnect or another control plane instance does not reset the wait. A refused request starts the wait too. A certificate that the agent did not confirm blocks the next one in the same way.
- An agent that finds its certificate past the due time at a connection waits a random time of up to 60 seconds before it asks. After a refusal it asks again after 11 minutes.
- A new certificate starts 5 minutes before the time of issue, so an agent with a slow clock accepts it. The start never comes before the start of the issuing CA.

## What happens in a rotation

1. The agent makes a new key pair. The new private key stays in the memory of the agent until the answer arrives.
2. The agent sends a certificate signing request (CSR) over the open stream.
3. The control plane checks the request. The agent must be active, and it must present its enrolled certificate. The request must hold a new public key. The control plane signs a certificate for the identity of the presented certificate, never for an identity in the request.
4. The control plane keeps the new certificate as pending and writes the audit record `agent_certificate_rotate`.
5. The agent checks the answer against the root CA that it pinned at enrollment. It stores the new key, certificate and chain in the protected configuration directory. Then it sends the confirmation.
6. The control plane makes the new certificate the enrolled one. It refuses the old certificate serial from this moment.

The old certificate stays valid until step 6. An agent that connects with the new certificate before it sent the confirmation also makes the new certificate the enrolled one.

The open stream keeps running during the rotation. A job that runs on the agent continues. The agent uses the new certificate at its next connection.

## Files on the agent

The agent writes the new files with the same owner-only rules as at enrollment. It writes them in the same directory as the rest of its state. A power loss in the middle of a rotation does not break the identity. At its next start the agent finishes the rotation or drops it. The key always matches the certificate.

## What the Hosts screen shows

The Hosts screen and the agents API show the certificate expiry and the time of the last rotation of each host. A host with an expiry in less than 3 days needs attention: its agent did not rotate.

## A certificate that a rotation replaced

After a rotation, the old certificate must not connect. If a connection presents it, the control plane refuses the connection. It writes the audit record `agent_superseded_certificate_used` at most once an hour, and the Hosts screen shows a warning for the host. This can mean a copy of the old private key. Check the host. If the host is not safe, revoke the agent and register it again.

## When the agent did not rotate

The agent rotates only over an open stream. If the agent stays offline until its certificate expires, it cannot connect and it cannot rotate. The log of the agent then shows this message:

```text
the agent certificate is expired and cannot rotate; register the agent again with a new enrollment token
```

An expired certificate cannot rotate. Enroll the host again (see [Enroll a host again](#enroll-a-host-again)). The host keeps its plans and labels.

## Enroll a host again

Use this procedure when the agent lost its identity, or when you must move the agent to a new installation. The host keeps its identifier, plans, maintenance host references and labels.

1. Open the Hosts screen as an Admin.
2. Open the menu "…" of the host row and select "Re-register". The host page has the same button in the section "Dangerous actions".
3. Select "Create token". The screen shows the token one time.
4. Run the shown command as `root` or Administrator on the computer of the host. The installer adds no link on `PATH`, so use the full path of the agent. You can run it from any folder.
   - Linux: `sudo /opt/restichub-agent/bin/restichub-agent register --force --url <url> --token <token>`
   - Windows, in an elevated PowerShell: `& "$env:ProgramFiles\ResticHub\Agent\restichub-agent.exe" register --force --url <url> --token <token>`

The token is bound to the host. The command prints that it replaced the host record. The control plane then refuses the old certificate at once, and it closes the open stream of the old agent. The audit log gets the action `agent_reenroll`.

A token that is bound to a host appears in the list "Pending registrations" with the text "Re-registers the host" and the host name. The token takes over the identity of the host. Therefore it expires after one hour at most. Set [`RESTICHUB_REENROLL_TOKEN_MAX_TTL_SECS`](/restichub/reference/environment-variables/) to change the limit. A new token for the same host cancels the earlier unused tokens of that host, and a successful registration cancels the others. A revoked host cannot enroll again, and the audit log records the attempt.

The host shows "Waiting for connection" until the new agent connects. It raises no "Host offline" alert in this time.

## Possible duplicates

The agent sends a digest of a stable identifier of the computer when it enrolls: the `MachineGuid` on Windows and the file `/etc/machine-id` on Linux. The raw identifier stays on the computer. The control plane keeps only a keyed hash of the digest. The key comes from the KEK and the tenant. A tenant cannot learn that a computer is enrolled in another tenant. A database dump does not confirm that a computer is enrolled. Virtual machines that were cloned from one image can share an identifier. The warning is a hint, and you decide.

An enrollment token that is not bound to a host can enroll a computer with a known identifier. Then both records show the status "Possible duplicate" in the list of hosts. The page of the newer record shows the banner "This record may duplicate the older record "<hostname>". The old record was last seen <time>."

To merge the records:

1. Open the menu "…" of the newer record, or open the page of the newer record. Only the newer record offers the replacement.
2. Select "Replace the old record". On the page, the button is in the banner. The dialog shows the last contact of both records.
3. Select "Replace".

The control plane moves the plans, the maintenance host references, the queued jobs and the labels of the old record to the newer record. It then revokes the old record and cancels its running jobs, as for a revoke. The audit log gets the actions `agent_replace` and `agent_revoke`. The screen shows "The old record is replaced" and the number of plans that moved.

The control plane replaces a record only when all these rules are true:

- Both records are active.
- Both records report the same computer identifier.
- The record that you open is newer than the old record.
- The old record had no sign of life in the last 5 minutes. Otherwise the control plane refuses the replacement with HTTP 409 `agent_recently_online`. Set [`RESTICHUB_REPLACE_ONLINE_WINDOW_SECS`](/restichub/reference/environment-variables/) to change the window.

For a pair that is not a possible duplicate, the control plane answers with HTTP 404.

## Pending registrations

The list "Pending registrations" shows the enrollment tokens that wait for a host. Each row shows who created the token, the creation time and the expiry time. Select "Cancel" to cancel a token. The list refreshes every 10 seconds. A token that a host used leaves the list. The "Add host" dialog shows "Host registered: <hostname>" when a host uses the token that the dialog created.

You cannot cancel a token in these cases. The control plane gives HTTP 409 with a typed code:

| Code | Meaning |
|---|---|
| `token_used` | A host already used the token. |
| `token_already_revoked` | An Admin cancelled the token before. |
| `token_expired` | The token expired. |

## Related pages

- [Register a host](/restichub/getting-started/add-a-host/)
- [Revoke an agent](/restichub/guides/agent-revocation/)
- [Environment variables](/restichub/reference/environment-variables/)
