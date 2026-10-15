---
title: Configure alerts
description: The installation settings for the public origin, the SMTP server and the webhook allow-list.
---

The alerts need four installation settings. They belong to the installation, not to an organization. Set them in the configuration file or in environment variables. An environment variable overrides the file.

## Public origin

The public origin is the base URL of the links in a message.

| File key | Environment variable | Meaning |
|---|---|---|
| `public_origin` | `RESTICHUB_PUBLIC_ORIGIN` | The scheme, the host and the port of the web interface, for example `https://backup.example.com`. |

The value has no user, no path and no query. Without it, a message holds no link.

## Language of messages

The emails and the webhook messages use one language for the whole installation. The web interface shows each alert in the language of the reader.

| File key | Environment variable | Meaning |
|---|---|---|
| `notifications.language` | `RESTICHUB_NOTIFICATIONS_LANGUAGE` | `en` (the default) or `ru`. |

A webhook message also holds two fields for a receiver that builds its own text. The field `params` holds the typed values. The field `error_text` holds the scrubbed error text of a job.

## SMTP server

An email channel sends through one SMTP server. Without a server, email channels cannot send.

| File key | Environment variable | Meaning |
|---|---|---|
| `smtp.host` | `RESTICHUB_SMTP_HOST` | The host name of the server. Setting it turns email on. |
| `smtp.port` | `RESTICHUB_SMTP_PORT` | The port. The default is 587 for `starttls`, 465 for `tls` and 25 for `none`. |
| `smtp.security` | `RESTICHUB_SMTP_SECURITY` | `starttls` (default), `tls` or `none`. |
| `smtp.username` | `RESTICHUB_SMTP_USERNAME` | The user name. Without it, the control plane does not sign in. |
| `smtp.password_file` | `RESTICHUB_SMTP_PASSWORD_FILE` | The path of a file that holds the password. |
| — | `RESTICHUB_SMTP_PASSWORD` | The password. Use it instead of the file. Set not both. |
| `smtp.from` | `RESTICHUB_SMTP_FROM` | The sender address. It is required when the host is set. |

Rules for the password:

- The password never goes to the database or to the audit log.
- Use the file for a production installation. The file must be a regular file that only its owner can read.
- The control plane signs in with `AUTH PLAIN` and only over TLS. With `none` and a password, the control plane refuses to start.
- When the server refuses a part of the recipients and accepts the others, the control plane sends the message. The delivery keeps a note with the number of refused recipients.
- The control plane checks the certificate of the server against the system roots and the bundled public roots.

Example:

```toml
public_origin = "https://backup.example.com"

[smtp]
host = "smtp.example.com"
security = "starttls"
username = "restichub"
password_file = "/run/secrets/smtp-password"
from = "restichub@example.com"
```

## Webhook allow-list

By default, a webhook uses `https` and a public address. The administrator can relax two rules.

| File key | Environment variable | Meaning |
|---|---|---|
| `webhook.allow_http` | `RESTICHUB_WEBHOOK_ALLOW_HTTP` | Allows the scheme `http`. The default is `false`. |
| `webhook.allow_networks` | `RESTICHUB_WEBHOOK_ALLOW_NETWORKS` | Networks in CIDR form that the address check admits. In the variable, separate the entries with commas. |

The address check always refuses the loopback, private, link-local, shared, reserved and multicast ranges. It also refuses the ranges `2001::/23` (which holds Teredo), `64:ff9b:1::/48` and `192.88.99.0/24`. The test button of a channel is limited to 6 tests per minute for each user and 20 per minute for each organization.

A network in the allow-list applies to every organization of the installation. Add the smallest network that holds the receiver.

```toml
[webhook]
allow_networks = ["10.20.30.0/24"]
```

> **Warning:** The allow-list lets an Admin of any organization make the control plane send requests into that network. Add a network only when you trust every Admin of the installation.
