---
title: Enroll a host again and find duplicate hosts
description: Keep the plans of a host when the agent enrolls again, and merge two records of the same computer.
---

A computer can enroll twice. For example, a person installs the agent again and uses a new enrollment token. Then the Hosts screen shows two records for one computer. This page tells how to prevent a second record, and how to merge two records.

## Hosts that wait for a connection

An agent that enrolled but did not open a stream yet has the status "Waiting for connection". The host is not online and not offline. The Overview counts it apart as "Hosts waiting for connection". The status changes to "Online" when the agent connects for the first time.

A host stays in this status when the service does not start or cannot reach the control plane. Run `restichub-agent status` on the host to read the last error (see [Register an agent](/restichub/reference/agent-commands/#troubleshooting)).

## Enroll a host again

Use this procedure when the agent lost its identity, or when you must move the agent to a new installation. The host keeps its identifier, plans, maintenance host references and labels.

1. Open the Hosts screen as an Admin.
2. Open the menu "…" of the host row and select "Re-register". The host page has the same button in the section "Dangerous actions".
3. Select "Create token". The screen shows the token one time.
4. Run the shown command as `root` or Administrator on the computer of the host: `restichub-agent register --force --url <url> --token <token>`. You can run it from any folder.

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
