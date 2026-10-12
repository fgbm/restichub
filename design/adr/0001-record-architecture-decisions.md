# ADR-0001: Record architecture decisions

- Status: accepted
- Date: 2026-10-12
- Requirements: NFR-OSS-01

## Context

ResticHub is an open source project. New contributors must understand why the project made each important decision. Discussions in chats and issues are hard to find later.

## Decision

We record each architecture decision as a Markdown file in `design/adr/`. We use the template in `0000-template.md`. We do not change accepted records. A new record supersedes an old record.

## Consequences

- The reasons for decisions stay in the repository, next to the code.
- A change of an important decision needs a pull request with a new record.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Wiki pages | They are separate from the code and have no review process. |
| No records | The reasons get lost. |
