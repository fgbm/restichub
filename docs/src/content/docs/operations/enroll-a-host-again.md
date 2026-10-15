---
title: Enroll a host again and find duplicate hosts
description: Keep the plans of a host when the agent enrolls again, and merge two records of the same computer.
---

A computer can enroll twice. For example, a person installs the agent again and uses a new enrollment token. Then the Hosts screen shows two records for one computer. This page tells how to prevent a second record, and how to merge two records.

## Hosts that wait for a connection

An agent that enrolled but did not open a stream yet has the status "Waiting for connection". The host is not online and not offline. The dashboard counts it apart. The status changes when the agent connects for the first time.

## Enroll a host again

Use this procedure when the agent lost its identity, or when you must move the agent to a new installation. The host keeps its identifier, plans, maintenance host references and labels (FR-ID-01).

1. Open the Hosts screen as an Admin.
2. Open the menu of the host and select "Re-register".
3. Select "Create token". The screen shows the token one time.
4. Run the shown command on the computer of the host: `restichub-agent register --force --url <url> --token <token>`.

The token is bound to the host. The command prints that it replaced the host. The control plane then refuses the old certificate at once, and it closes the open stream of the old agent. The audit log gets the action `agent_reenroll`.

A token that is bound to a host appears in the list "Pending registrations" with the name of the host. The token expires after one hour by default. A revoked host cannot enroll again.

## Possible duplicates

The agent sends a stable identifier of the computer when it enrolls: the `MachineGuid` on Windows and the file `/etc/machine-id` on Linux. The agent does not write the identifier in a log. The control plane keeps only a hash of it. The hash depends on the tenant, so a tenant cannot learn that a computer is enrolled in another tenant.

When an enrollment token that is not bound to a host enrolls a computer with a known identifier, both records show "Possible duplicate of <hostname>".

To merge the records:

1. Open the menu of the host that you keep.
2. Select "Replace the old record" and confirm.

The control plane moves the plans, the maintenance host references and the labels of the old record to the host that you keep. It then revokes the old record. All changes happen in one transaction. The audit log gets the action `agent_replace`. Both records must be active.

## Pending registrations

The list "Pending registrations" refreshes every 10 seconds. A token that a host used leaves the list. The "Add host" dialog shows "Host registered: <hostname>" when a host uses the token that the dialog created.

You cannot cancel a token in these cases. The control plane gives HTTP 409 with a typed code:

| Code | Meaning |
|---|---|
| `token_used` | A host already used the token. |
| `token_already_revoked` | An Admin cancelled the token before. |
| `token_expired` | The token expired. |
