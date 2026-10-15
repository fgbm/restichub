# Instructions for coding agents

Read this file before you change the repository.

## Sources of truth

1. [design/requirements.md](design/requirements.md): requirements with IDs.
2. [design/acceptance-criteria.md](design/acceptance-criteria.md): acceptance criteria with IDs.
3. [design/adr/](design/adr/): accepted decisions. Do not act against an accepted ADR. To change a decision, propose a new ADR.
4. [design/threat-model.md](design/threat-model.md): security assumptions.
5. [design/use-cases.md](design/use-cases.md): acceptance use cases for the manual acceptance.

## Rules

- Write all repository text in English by ASD-STE100 ([design/style/ste-en.md](design/style/ste-en.md)). Code comments and doc comments are outside this scope; the Code rules below apply to them. Exception: Russian documentation in `docs/src/content/docs/ru/` obeys [design/style/ru-technical.md](design/style/ru-technical.md).
- Use the terms from the project glossary. Do not use synonyms.
- Each test that covers an acceptance criterion refers to its ID, for example `ac_ag_05_exit_code_mapping`.
- Never put a long-lived secret in logs, process arguments or files on the agent. A one-time enrollment token is the only exception: the `register` command takes it on the command line (ADR-0032).
- The agent has no scheduling logic (ADR-0002). Do not add timers that start jobs on the agent.
- A script from the CP runs only on a host with the local trust mode, which is off by default (ADR-0037). Do not add another step type that runs an arbitrary command. The allow-list hooks of ADR-0004 are removed (ADR-0040).
- Steps are typed messages. Do not add free-form argument maps to the protocol (FR-PR-01).
- Each database table has `tenant_id` (ADR-0006).

## Code rules

- A comment explains only what the code cannot say. It gives a non-obvious reason, a security invariant, or a constraint with its FR, AC or ADR ID.
- Do not restate a name or the code in a comment. Do not narrate steps. Do not refer to a review finding or the change history.
- A doc comment is for the public library API only. One sentence is enough, unless the contract is subtle.
- Do not add a doc comment to a field or a function whose name says everything.
- Code comments use plain short English, not STE. The first line of a doc comment never restates the item name.
- An OpenAPI field description is only where it adds information: units, format, allowed values or invariants.
- Use `?` with a `From` conversion for an error. Do not add a `map_err` chain that only wraps into an internal error.
- Never drop the cause with `map_err(|_| ...)`.
- A closed set (role, status, purpose, action, error code) is an enum, not a string. A string literal of these values appears only in SQL and migrations.
- One source of truth for each route. Pass 2 introduces it. Until then, follow the existing table.
- When the test database variable is set, a setup failure panics. It never skips. Use `expect` in tests.
- A test helper does not live in a production module. Split a test file above about 500 lines by topic.
- Do not add a dependency until the code uses it. `cargo machete` runs in CI.
- Keep a file focused. Split a module above about 800 lines.
- `scripts/check-comments.sh` runs in CI.

## Commands

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
vale --minAlertLevel=error README.md CONTRIBUTING.md SECURITY.md design docs/src
(cd docs && npm ci && npm run build)
```

## Commits

Use Conventional Commits and sign off each commit (`git commit -s`).
