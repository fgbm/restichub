---
title: Alerts and notification channels
description: How the control plane raises alerts, which kinds exist, and how to send them by email or by webhook.
---

The control plane watches each organization and raises an alert when a backup needs attention. It sends a message to each enabled channel of the organization. When the condition clears, it sends a recovery message.

## Alert kinds

An Admin switches each kind on or off in **Settings**, **Alert rules**.

| Kind | Name in the web interface | Raised when | Default |
|---|---|---|---|
| `rpo_violated` | Backup overdue | A plan has no successful backup within its recovery point objective (RPO). | On |
| `job_failed` | Job failed | The latest finished job of a plan or a task failed. | On |
| `job_dead_letter` | Job gave up | The latest finished job stayed locked after every retry. | On |
| `job_warnings` | Job with warnings | The latest finished job succeeded, but restic reported warnings. | Off |
| `agent_offline` | Host offline | A host did not contact the control plane for longer than the limit. | On, 30 minutes |
| `stale_lock` | Stale repository lock | A job waits on a stale repository lock for longer than the limit. | On, 60 minutes |

An alert stays open while its condition holds. The control plane sends one message when it opens the alert. It sends no repeat while the alert is open. A job alert resolves when the next job of the same plan succeeds.

A failure that finished more than 24 hours before the first evaluation raises no alert. A cancelled job and an expired job never raise an alert. A disabled plan and a plan of a revoked host raise no RPO alert.

When an alert resolves because its plan, host or rule is gone, the control plane sends no recovery message.

## Recovery point objective

Each plan has an optional RPO from 15 minutes to 400 days. Set it in the plan form, in hours. The default is the schedule interval plus a margin. The margin is one tenth of the interval, at least 15 minutes and at most 2 hours.

| Schedule | Default RPO |
|---|---|
| Every hour | 75 minutes |
| Every day | 26 hours |
| Every week | 170 hours |

The age of a plan is the time since its last successful backup. A plan without a success counts from its creation. The Overview shows the state of each plan:

- `OK`: the age is below 75 % of the RPO.
- `At risk`: the age is 75 % of the RPO or more.
- `Violated`: the age is above the RPO.

## Delivery

A loop in each control plane instance evaluates the rules every 30 seconds. A message therefore reaches a channel well inside 5 minutes of a violation.

The control plane stores each message in a queue before it sends it. A failed send repeats after 30 seconds, 2 minutes, 10 minutes and 1 hour. After the fifth failure, the control plane drops the message and keeps the error text. The queue holds at most 1000 pending messages for an organization.

Two control plane instances send each message once. A crash between the send and the record can repeat a send, so a receiver must accept a repeat. The webhook payload holds the field `id` for this purpose.

## Email channels

An Admin adds an email channel in **Settings**, **Channels**. The channel holds 1 to 20 recipients. The installation needs an SMTP server. See [Configure alerts](/restichub/operations/alert-configuration/). Without it, an email channel cannot send, and the test button shows the error `smtp_not_configured`.

The message is plain text with the subject `[ResticHub] <alert> - <plan> (<organization>)`. It holds the organization, the host, the plan, the repository, the error text and a link to the job.

## Webhook channels

A webhook channel sends a signed `POST` with a JSON body. The URL must use `https`, unless the installation allows `http`.

When you create a webhook, the control plane generates a signing secret. The dialog shows the secret one time. Copy it to the receiver. To get a new secret, choose **Create new signing secret** in the edit dialog. The path and the query of the URL stay hidden after you save, because they often hold a secret.

### Payload

```json
{
  "schema_version": 1,
  "id": "0198a2a8-6a52-7f5e-9a4d-4a3c9f5a7e10",
  "event": "alert.opened",
  "sent_at": "2026-10-15T08:00:00Z",
  "tenant": { "id": "0198a2a0-0000-7000-8000-000000000001", "name": "acme" },
  "alert": {
    "id": "0198a2a8-6a50-7a3b-8c2d-1b2c3d4e5f60",
    "kind": "job_failed",
    "status": "open",
    "opened_at": "2026-10-15T08:00:00Z",
    "resolved_at": null,
    "agent": { "id": "…", "name": "win-fs01" },
    "plan": { "id": "…", "name": "nightly-web" },
    "repository": { "id": "…", "name": "primary-s3" },
    "job": { "id": "…" },
    "message": "The backup job failed (failed): repository password=<redacted> rejected",
    "params": { "job_kind": "backup", "error_code": "failed" },
    "url": "https://backup.example.com/jobs/…"
  }
}
```

The field `event` is `alert.opened`, `alert.resolved` or `test`. A test message has `"alert": null`. The text in `message` is scrubbed and holds at most 500 characters. The control plane hides a password, a token, a key and the user in a URL.

The language of `message` follows `notifications.language` of the installation, so a receiver must not parse it. A receiver that decides what to do must read `alert.kind` and `alert.params`. The fields of `params` are typed values, such as `elapsed_seconds`, `rpo_seconds`, `threshold_minutes`, `job_kind` and `error_code`. A field is absent when it does not apply. A new version can add a kind or a field, so a receiver must ignore the values that it does not know.

### Signature

Each request holds these headers:

| Header | Value |
|---|---|
| `X-ResticHub-Timestamp` | The send time in seconds since the Unix epoch. |
| `X-ResticHub-Signature` | `sha256=` and the hexadecimal HMAC-SHA256 of the timestamp, a dot and the raw body. |
| `X-ResticHub-Delivery` | The identifier of the delivery. It equals `id` in the body. |

The key of the HMAC is the signing secret. To check a request:

1. Read the raw body. Do not parse it first.
2. Compute `HMAC-SHA256(secret, timestamp + "." + body)` and write it in hexadecimal.
3. Compare the result with the header in constant time.
4. Reject a request whose timestamp is older than 5 minutes.
5. Ignore a request whose `X-ResticHub-Delivery` you already processed.

### Address policy

The control plane refuses to send to a private network. It resolves the host name, checks every address, and connects to the checked address itself. It refuses:

- loopback addresses, such as `127.0.0.1` and `::1`;
- private addresses, such as `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` and `fc00::/7`;
- link-local addresses, such as `169.254.0.0/16`, which includes the cloud metadata address `169.254.169.254`, and `fe80::/10`;
- carrier-grade NAT, multicast, unspecified and documentation addresses;
- an IPv6 address that wraps one of these IPv4 addresses.

A host name that resolves to one forbidden address among others is refused too. The control plane never follows a redirect. An answer other than 2xx is a failure. A send takes at most 10 seconds.

To reach a private receiver, the administrator adds its network to the allow-list. See [Configure alerts](/restichub/operations/alert-configuration/).

## Audit

The audit log holds the records `channel_create`, `channel_update`, `channel_delete`, `alert_rule_update` and `alert_sent`. A record holds no secret and no URL path. A record `alert_sent` holds no address.
