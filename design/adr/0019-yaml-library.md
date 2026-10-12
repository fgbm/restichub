# ADR-0019: serde-saphyr for YAML profile import and export

- Status: proposed
- Date: 2026-10-12
- Requirements: FR-PF-07

## Context

FR-PF-07 needs import and export of profiles in YAML. The control plane reads YAML from users, so the control plane must not trust the input. A hostile file can use alias bombs, deep nesting or a very large size to exhaust memory or stack.

The owner archived `serde_yaml` and deprecated it on 2024-03-25. Its forks `serde_yaml_ng` and `serde_norway` still use the translated C library `unsafe-libyaml`. RUSTSEC-2024-0449 marks `unsafe-libyaml` as unmaintained. RUSTSEC-2025-0068 marks `serde_yml` as unsound.

Spike P5 compared ten crates. See [the spike report](../spikes/p5-yaml-library.md). The parser crates `saphyr`, `saphyr-parser` and `yaml-rust2` have no `serde` support. The `deny.toml` file allows the license of each candidate.

## Decision

- We use `serde-saphyr` for all YAML import and export of profiles. The crate forbids `unsafe` code, uses a pure Rust parser (`granit-parser`) and has configurable resource limits.
- One module of the control plane owns all YAML calls. No other module imports `serde-saphyr`.
- The module calls `from_str_with_options` with limits that we choose for profile files. The API layer limits the size of the request body before parsing.
- We pin the minor version of `serde-saphyr`. Each update passes `cargo deny` and a review of the release notes.
- Tests for the alias bomb, deep nesting and a large document refer to FR-PF-07.
- If the maintainer stops work, we replace the module. We then run a new spike. Candidates: `saphyr-serde` and `noyalib` 1.0.

## Consequences

- The YAML path has no `unsafe` code in the parser and the `serde` layer, and no C dependency.
- Limits for aliases, nesting depth and size are part of the API of the library. We do not write them ourselves.
- The default limits are permissive. We must tighten them and test them. The tests are an extra task for the implementation.
- When the target type is `bool`, the library reads the YAML 1.1 forms `yes` and `on`. On export, it quotes strings such as `"yes"` and `"1.10"`.
- One maintainer wrote most of the code of `serde-saphyr` and `granit-parser`. This is a project risk. The single module limits the cost of a replacement.
- The crate is young: first release 2025-09-27, version 1.0.0 on 2026-07-31. We must fuzz the import path before release 1.0.
- The crate needs Rust 1.89. The project uses Rust 1.96.
- The API differs from `serde_yaml`. This has no cost, because the project has no existing YAML code.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| `serde_yaml` 0.9.34 | The owner archived it and marked it deprecated. It uses `unsafe-libyaml`, which has no maintainer. |
| `serde_yml` | RUSTSEC-2025-0068 marks versions up to 0.0.12 as unsound. Version 0.0.13 is a deprecated shim for `noyalib`. |
| `serde_yaml_ng` | It uses `unsafe-libyaml` and has no release since 2024-05-26. It supports YAML 1.1 only. |
| `serde_norway` | It uses `unsafe-libyaml-norway`, a fork of the same C translation. It has no release since 2024-12-21. |
| `serde_yaml_bw` | It still uses `unsafe-libyaml-norway`. Its README recommends `serde-saphyr`. |
| `noyalib` | Version 0.0.57 is pre-1.0 and 5 months old. RUSTSEC-2026-0333 shows that typed parsing ignored some budgets before 0.0.53. We review it again at 1.0. |
| `saphyr` or `yaml-rust2` with own `serde` layer | These crates have no `serde` support. A layer of our own adds code and risk with no gain. |
