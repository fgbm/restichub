---
title: Version and support
description: How to read the version of the control plane, and which facts to send to support.
---

The About ResticHub screen shows the version of the control plane, the versions of the protocol and the environment of the server. Give these facts to support with your question. Every signed-in user reads the screen. This includes an installation with one organization and an account without an organization.

## Read the version

1. Select your account menu in the top bar.
2. Select "About ResticHub".

The item in the account menu shows the short version below its label. The screen shows these facts:

| Fact     | Meaning                                                                                                  |
| -------- | -------------------------------------------------------------------------------------------------------- |
| Version  | The version of the control plane, with the pre-release part of a pre-release build. That build shows the badge "Pre-release". |
| Revision | The commit of the build. A build from the source code shows `dev`.                                       |
| Protocol | The version of the protocol between the control plane and the agents.                                   |
| Agents   | The oldest agent version that the control plane accepts, and its major version. An agent of another major version is refused. |

## Server environment

The screen shows the environment of the server (ADR-0055):

| Fact            | Meaning                                                                                                         |
| --------------- | --------------------------------------------------------------------------------------------------------------- |
| Operating system | The operating system family of the process, for example `linux`.                                              |
| Distribution    | The distribution of the process, for example `Ubuntu 24.04.1 LTS` or `Debian GNU/Linux 12 (bookworm)`. Linux only. |
| Kernel          | The release of the running kernel. Linux only.                                                                  |
| Architecture    | The architecture of the process, for example `x86_64`.                                                          |
| Container       | `Yes` when the control plane runs in a container. `Not set` when the process cannot tell a container from a host, also on a host. |
| PostgreSQL      | The server version of the database, for example `16.4`.                                                         |

The distribution stays out when its name holds an unusual character, has the shape of a file path, or is very long. An example of a path shape is a name that starts with `/` or holds `//`.

A fact that the process does not report shows the text "Not set". Thus the kernel and the distribution show "Not set" on Windows. The container shows "Not set" when the process cannot tell a container from a host, also on a host. The PostgreSQL version shows "Not set" when the database stops answering after you sign in. When the database is down, the sign-in check fails and the screen shows the load error, as every other screen does.

## Send the facts to support

The block "Support information" holds one text with the version, the revision, the protocol, the range of the agents and the environment of the server. Select "Copy". Paste the text into your support request. One copy is enough for support.

## The facts of the installation

A Platform Admin sees two more facts:

- Organizations: "One organization" or "Several organizations". This is the tenancy mode of the installation (ADR-0024). The support text holds the mode for every role, because `GET /api/v1/instance` reports it without a sign-in.
- Database schema level: the version of the last database migration. Compare it with the version in the release notes when an update fails.

The panel is absent for the other roles.

## Links

The screen links these pages:

- The [release notes](https://github.com/fgbm/restichub/releases) of ResticHub.
- The [documentation](https://fgbm.github.io/restichub/).
- The [agent versions](https://fgbm.github.io/restichub/reference/agent-version-compatibility/) that this control plane accepts.
- The [Apache-2.0 license](https://github.com/fgbm/restichub/blob/main/LICENSE).
