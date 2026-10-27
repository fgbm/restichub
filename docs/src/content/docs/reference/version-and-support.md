---
title: Version and support
description: How to read the version of the control plane, and which facts to send to support.
---

The About ResticHub screen shows the version of the control plane and the versions of the protocol. Give these facts to support with your question. Every signed-in user reads the screen. This includes an installation with one organization and an account without an organization.

## Read the version

1. Select your account menu in the top bar.
2. Select "About ResticHub".

The item in the account menu shows the short version in its hint. The screen shows these facts:

| Fact     | Meaning                                                                                                  |
| -------- | -------------------------------------------------------------------------------------------------------- |
| Version  | The version of the control plane. A pre-release build shows the badge "Pre-release".                    |
| Revision | The commit of the build. A build from the source code shows `dev`.                                       |
| Protocol | The version of the protocol between the control plane and the agents.                                   |
| Agents   | The oldest agent version that the control plane accepts. A newer minor version works too.               |

## Send the facts to support

The block "Support information" holds one line with the version, the revision, the protocol and the range of the agents. Select "Copy". Paste the text into your support request.

## The facts of the installation

A Platform Admin sees two more facts:

- Organizations: "One organization" or "Several organizations". This is the tenancy mode of the installation (ADR-0024).
- Database schema level: the version of the last database migration. Compare it with the version in the release notes when an update fails.

These two facts are absent for the other roles.

## Links

The screen links these pages:

- The [release notes](https://github.com/fgbm/restichub/releases) of ResticHub.
- The [documentation](https://fgbm.github.io/restichub/).
- The [agent versions](https://fgbm.github.io/restichub/reference/agent-version-compatibility/) that this control plane accepts.
- The [Apache-2.0 license](https://github.com/fgbm/restichub/blob/main/LICENSE).
