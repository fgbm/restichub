# Instructions for coding agents

Read this file before you change the repository.

## Sources of truth

1. [design/requirements.md](design/requirements.md): requirements with IDs.
2. [design/acceptance-criteria.md](design/acceptance-criteria.md): acceptance criteria with IDs.
3. [design/adr/](design/adr/): accepted decisions. Do not act against an accepted ADR. To change a decision, propose a new ADR.
4. [design/threat-model.md](design/threat-model.md): security assumptions.

## Rules

- Write all repository text in English by ASD-STE100 ([design/style/ste-en.md](design/style/ste-en.md)). Exception: Russian documentation in `docs/src/content/docs/ru/` obeys [design/style/ru-technical.md](design/style/ru-technical.md).
- Use the terms from the project glossary. Do not use synonyms.
- Each test that covers an acceptance criterion refers to its ID, for example `ac_ag_05_exit_code_mapping`.
- Never put secrets in logs, process arguments or files on the agent.
- The agent has no scheduling logic (ADR-0002). Do not add timers that start jobs on the agent.
- Hooks run only from the agent allow-list (ADR-0004). Do not add a step type that runs an arbitrary command.
- Steps are typed messages. Do not add free-form argument maps to the protocol (FR-PR-01).
- Each database table has `tenant_id` (ADR-0006).

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
