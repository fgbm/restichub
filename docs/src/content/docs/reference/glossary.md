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
| profile | The definition of a backup: paths, exclusions, options, retention and scripts. A profile declares parameters and holds no repository. |
| plan | A profile, a host and a repository with a schedule. |
| profile parameter | A named value that a profile declares. A host sets the value for all its profiles, and a plan can override it. |
| parameter value | The value of a profile parameter on a host or on a plan. |
| template variable | A placeholder such as `{{hostname}}` or `{{param.<name>}}` in a path, an exclude or a tag. The control plane replaces it with a value for each host. |
| job | One run of a plan, or one manual operation. |
| step | One command in a job. |
| snapshot | A restic snapshot. |
| backup | The data that restic saves. |
| maintenance host | The host that runs `forget`, `prune` and `check` for a repository. |
| repository host | The host that holds the path of a local repository. It is the maintenance host of the repository, and every job of the repository runs on it. |
| exclusive operation | An operation that needs the whole repository: `init`, `check`, `forget`, `prune` and `unlock`. |
| restore directory | A directory on a host. The agent writes the files of a restore only below it. A host has one directory by default, and an administrator can add more. The API reports the directories in the field `restore_roots`. |
| enrollment | The first registration of an agent. The web interface and the agent command say "register". |
| enrollment token | A one-time value that lets an agent enroll. |
| agent listener | The separate port of the control plane. Agents connect to it with mTLS. |
| revoke | Make the certificate of an agent not valid. |
| lease | The time in which an agent must confirm that it accepted a job. |
| server script | A script that you write in the web UI. The control plane signs it, and an agent runs it. |
| trust mode | A local setting of a host that allows server scripts. It is off by default. |
| KEK | The key encryption key. It protects all secrets in the database. |
| API token | A secret value that a script sends to call the API. |
