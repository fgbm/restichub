---
title: About ResticHub
description: What ResticHub is, the problem that it solves, why it exists and how it compares with similar tools.
---

ResticHub is a central control plane for [restic](https://restic.net) backups on many hosts. The control plane keeps the profiles, the schedules and the secrets. A thin agent on each host runs restic when the control plane tells it to.

ResticHub is pre-release. The MVP is in acceptance testing on real hosts. The interfaces can still change.

## The problem and who has it

restic is a good backup program for one host. It encrypts and deduplicates the data, and it supports many storage backends. But restic has no server. Each host keeps its own configuration, its own schedule and its own repository password.

This is a problem for a small team of administrators with tens or hundreds of Windows and Linux hosts:

- The configuration of each host is different, and nobody sees all of it in one place.
- The repository passwords and the storage credentials are in files on each host.
- Nobody knows which host did not make a backup last night, until a restore fails.
- Two hosts can run `prune` on one repository at the same time, or a stale lock stops the backups for days.
- No log records who changed a backup, who ran a restore or who got a secret.

ResticHub is for these teams. It is also for teams that already use restic with scripts or with resticprofile, and want one place to control it.

## Why ResticHub exists

The project started from a list of functions that the team needs and that no restic tool gives together:

- One server that makes all decisions about when a job runs. The hosts keep no schedules.
- Secrets that stay on the server. A host gets a repository password for one job only and does not store it.
- Windows and Volume Shadow Copy (VSS) as primary use cases, not as an addition.
- Organizations with separate data, and roles for the people in them.
- A dashboard that compares the age of the last good backup of each plan with its recovery point objective (RPO). An alert when the age is too large.
- An audit log of each change, each run, each restore and each delivery of a secret.
- A control plane that cannot run an arbitrary command on a host, unless the administrator of that host permits it.

The other tools do some of these functions. The [comparison](#comparison) shows which.

## How it works

The control plane is a server with PostgreSQL, a web interface and a REST API. An agent runs as a service on each host and opens one outbound connection to the control plane, with mutual TLS. The agent does not listen on a port. When a plan is due, the control plane signs the job and sends it to the agent with the secrets for that job. The agent verifies the signature, runs restic, streams the progress and sends the result. The agent writes the data directly to the repository, so the backup data does not go through the control plane. A script from the control plane runs only on a host where a local administrator turned on the trust mode. Read [Concepts](/restichub/concepts/architecture/) for the terms. Read [Security for operators](/restichub/operations/security-for-operators/) for the security model.

## What ResticHub is not

- It is not a new backup format. restic does the deduplication, the encryption and the storage. A repository stays a normal restic repository that you can open without ResticHub.
- It does not make disk images, bare-metal recovery or hypervisor-level backups of virtual machines. It backs up files.
- It has no native database steps, for example for Microsoft SQL Server or PostgreSQL. Use a script on a trusted host to make a database dump before the backup.
- It does not mount a repository as a file system.
- It has no high availability in the MVP. If the control plane stops, no jobs start until it runs again.
- It does not protect a repository from a host that is compromised during a job. The agent has the repository password for the duration of the job. Use storage controls, for example bucket versioning and a separate copy that the hosts cannot write.

## Comparison

The table compares ResticHub with six tools on the functions that define ResticHub. The facts come from the documentation and the repository of each tool. Date of the check: 2026-10-17. A dash means that the documentation of the tool does not tell, or that the function does not apply. It does not mean "no".

Three of the tools use restic: [Backrest](https://github.com/garethgeorge/backrest), [resticprofile](https://github.com/creativeprojects/resticprofile) and [Arkeep](https://github.com/arkeep-io/arkeep). Three of the tools use their own format: [Kopia](https://kopia.io) with its [repository server](https://kopia.io/docs/repository-server/), [UrBackup](https://www.urbackup.org) and [Bareos](https://www.bareos.com).

<div style="overflow-x: auto">

| | ResticHub | Backrest | resticprofile | Arkeep | Kopia | UrBackup | Bareos |
|---|---|---|---|---|---|---|---|
| Backup engine | restic | restic | restic | restic, rclone | Kopia | UrBackup, files and images | Bareos |
| Central control of many hosts | Yes | Optional [multihost sync](https://garethgeorge.github.io/backrest/docs/multihost) between full instances | No, one configuration file on each host | Yes | Optional server mode | Yes | Yes |
| Where the schedule runs | Control plane | Each instance ([issue #68](https://github.com/garethgeorge/backrest/issues/68)) | Scheduler of the operating system | Server | Policy, which the Kopia process on the host runs ([FAQ](https://kopia.io/docs/faqs/)) | Server | Director |
| Connection direction | Agent to control plane, outbound only | Instance to instance | Not applicable | Agent to server, outbound only | Host to repository server | LAN: server finds the hosts. Internet: host to server ([manual](https://www.urbackup.org/administration_manual.html)) | Director to file daemon by default. Client-initiated connection is optional ([network setup](https://docs.bareos.org/TasksAndConcepts/NetworkSetup.html)) |
| Storage credentials on the host | Only in memory during a job | In the local configuration | In the local configuration | - | Not necessary with the repository server | Not necessary, the server keeps the storage | Not necessary, the storage daemon keeps the storage |
| Windows VSS | Yes, profile option | - | - | - | Yes, [policy option](https://kopia.io/docs/reference/command-line/common/policy-set) | Yes ([features](https://www.urbackup.org/features.html)) | Yes ([Windows](https://docs.bareos.org/TasksAndConcepts/TheWindowsVersionOfBareos.html)) |
| Organizations and roles | Organizations; Admin, Operator, Viewer | Permissions for each paired instance | Not applicable | Local accounts and OpenID Connect | Access rules for each `user@host` | - | - |
| Audit log | Yes | - | Not applicable | Yes | - | - | - |
| Alerts | Failed job, RPO violation, offline host, stale lock; email, webhook | Discord, Slack, Shoutrrr, Gotify, Healthchecks | Status file, Prometheus, HTTP hooks | Email, webhook, Healthchecks | - | Email when a host has no backup for a set time | - |
| License | Apache 2.0 | GPL 3.0 | GPL 3.0 | Apache 2.0 | Apache 2.0 | AGPL 3.0 or later | AGPL 3.0 |
| Latest release on the date of the check | Pre-release | [1.14.1](https://github.com/garethgeorge/backrest/releases/tag/v1.14.1), 2026-07-12 | [0.33.1](https://github.com/creativeprojects/resticprofile/releases/tag/v0.33.1), 2026-04-15 | [0.9.0](https://github.com/arkeep-io/arkeep/releases/tag/v0.9.0), 2026-10-12 | - | - | - |

</div>

restic itself can make a VSS snapshot on Windows with the `--use-fs-snapshot` option. The restic-based tools can pass restic options. The table shows a dash for them because their documentation does not show VSS as a separate function.

The most similar tool is Arkeep. It also has a server that schedules each job and agents that connect outbound with mutual TLS. The differences are in the focus. Arkeep finds Docker containers and volumes, supports OpenID Connect and SFTP today, and plans inventory and update orchestration. ResticHub puts the focus on Windows hosts, organizations with roles, secrets for one job only, signed jobs and the trust mode for scripts. Arkeep sends an anonymous daily ping by default, and you can turn it off ([README](https://github.com/arkeep-io/arkeep)). ResticHub sends no telemetry.

## When to choose another tool

- **One host or a small number of personal hosts.** Backrest gives a web interface for restic on each host. Many people use it: the repository had 7.5 thousand stars on the date of the check.
- **Configuration as files in Git, without a server.** resticprofile keeps the profiles in a file and uses the scheduler of the operating system.
- **A released product today, with Docker volume discovery, OpenID Connect or SFTP repositories.** Arkeep has these functions now. In ResticHub, OpenID Connect and SFTP come in version 1.0.
- **Hosts must never know the storage credentials.** The Kopia repository server keeps the storage credentials on the server. In ResticHub, the agent gets the repository password for the duration of a job.
- **Disk images and bare-metal recovery.** UrBackup makes image backups of Windows and Linux volumes.
- **A classic backup system with its own storage daemons and a backup catalog.** Bareos has a director, file daemons and storage daemons, and a catalog of all backups.
- **macOS hosts now.** The ResticHub agent for macOS comes in version 1.0.
- **High availability of the server.** The ResticHub MVP has one control plane instance.
