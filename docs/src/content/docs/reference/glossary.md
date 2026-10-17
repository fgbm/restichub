---
title: Glossary
description: The terms that ResticHub uses, with one meaning for each term.
---

The documentation and the web UI use these terms. One term has one meaning.

| Term | Meaning |
|---|---|
| agent | The ResticHub program on a host. It runs restic and sends the result. |
| control plane | The ResticHub server. It holds the schedule, the secrets and the audit log. |
| host | A computer with an agent. |
| tenant | The unit of isolation in one installation. A tenant owns hosts, repositories, plans, profiles, secrets and members. The web UI calls it an organization. |
| repository | A restic repository. |
| profile | The definition of a backup: paths, exclusions, options and retention. |
| plan | A profile that runs on a host with a schedule. |
| job | One run of a plan, or one manual operation. |
| step | One command in a job. |
| snapshot | A restic snapshot. |
| backup | The data that restic saves. |
| maintenance agent | The agent that runs `forget`, `prune` and `check` for a repository. |
| repository host | The host that holds the path of a local repository. It is the maintenance agent of the repository, and every job of the repository runs on it. |
| exclusive operation | An operation that needs the whole repository: `init`, `check`, `forget`, `prune` and `unlock`. |
| enrollment | The first registration of an agent. |
| enrollment token | A one-time value that lets an agent enroll. |
| agent listener | The separate port of the control plane. Agents connect to it with mTLS. |
| revoke | Make the certificate of an agent not valid. |
| lease | The time in which an agent must confirm that it accepted a job. |
| server script | A script that you write in the web UI. The control plane signs it, and an agent runs it. |
| trust mode | A local setting of a host that allows server scripts. It is off by default. |
| KEK | The key encryption key. It protects all secrets in the database. |
| API token | A secret value that a script sends to call the API. |
