# ADR-0014: Apache License 2.0 and GitHub

- Status: accepted
- Date: 2026-10-12
- Requirements: D1, NFR-OSS-01, NFR-SEC-06

## Context

ResticHub is an open source product for companies and service providers. Similar tools use GPL-3.0 (Backrest), AGPL-3.0 (Zerobyte), a non-commercial license (Fulgurite) or Apache-2.0 (Arkeep, Pluton). Free Authenticode signing for open source (SignPath Foundation) needs a public build system.

## Decision

- The license is the Apache License 2.0.
- Contributors sign off commits with the Developer Certificate of Origin. There is no Contributor License Agreement.
- The main repository is on GitHub. CI uses GitHub Actions. Mirrors to other hosts are permitted.

## Consequences

- Companies can adopt and extend ResticHub with few legal obstacles.
- Closed forks are possible.
- GitHub Actions gives Windows arm64 runners and works with SignPath Foundation.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| AGPL-3.0 | Stops closed SaaS forks, but makes adoption in companies harder. |
| GitLab as the main host | Smaller open source audience. SignPath Foundation needs a supported public CI. |
