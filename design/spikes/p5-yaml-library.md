# Spike P5: YAML library

- Status: done
- Date: 2026-10-12
- Requirements: FR-PF-07
- Decision record: [ADR-0019](../adr/0019-yaml-library.md)

## Question

Which maintained YAML library replaces the archived `serde_yaml`? The control plane uses the library to import and export profiles (FR-PF-07). The input comes from users and the control plane must treat it as untrusted.

The owner archived the dtolnay repository of `serde_yaml`. The last release is 0.9.34, from 2024-03-25. The owner does not name a replacement [1][20].

## Criteria

Each criterion has an ID. The recommendation uses these IDs.

| ID | Criterion | Why it matters |
|---|---|---|
| C1 | Maintenance: recent release, active repository, more than one release in 2026. | The control plane must receive fixes. |
| C2 | No `unsafe` code in the library and no C dependency. | The library parses untrusted input. |
| C3 | No open RustSec advisory for the crate or its parser. | The `cargo deny` check reports advisories. |
| C4 | `serde` `Serialize` and `Deserialize` support. | FR-PF-07 needs import and export. |
| C5 | Limits for alias expansion, nesting depth and document size. | Alias bombs and deep nesting must not exhaust memory or stack. |
| C6 | YAML 1.2 support. | Profiles must stay portable to other tools. |
| C7 | License allowed by `deny.toml`. | The build must pass `cargo deny check licenses`. |

## Comparison table

Data come from crates.io, GitHub and the RustSec advisory database on 2026-10-12 [1]–[13], [28]. "Recent downloads" means the last 90 days, as the crates.io API reports them.

### Project health

| Crate | Latest version (date) | Recent downloads | Maintainer | Repository state |
|---|---|---|---|---|
| `serde_yaml` | 0.9.34+deprecated (2024-03-25) | 96.1 M | dtolnay | Archived 2024-03-25 |
| `serde_yml` | 0.0.13 (2026-05-27) | 5.4 M | sebastienrousseau | Archived. Crate is a deprecated shim for `noyalib`. |
| `serde_norway` | 0.9.42 (2024-12-21) | 6.4 M | cafkafk | Open. Last push 2025-08-04. |
| `serde_yaml_ng` | 0.10.0 (2024-05-26) | 9.8 M | acatton | Open. Last push 2025-09-14. |
| `serde_yaml_bw` | 2.5.8 (2026-09-07) | 0.15 M | bourumir-wyngs | Open. Last push 2026-09-07. |
| `serde-saphyr` | 1.3.0 (2026-09-16) | 7.3 M | bourumir-wyngs | Open. Last push 2026-10-11. 37 versions. |
| `saphyr` | 0.2.1 (2026-10-12) | 0.9 M | Ethiraric, davvid | Open. Last push 2026-10-12. |
| `saphyr-parser` | 0.2.1 (2026-10-12) | 4.5 M | Ethiraric, davvid | Same repository as `saphyr`. |
| `yaml-rust2` | 0.13.0 (2026-09-11) | 18.4 M | Ethiraric, davvid | Open. Last push 2026-09-11. |
| `noyalib` | 0.0.57 (2026-10-09) | 0.7 M | sebastienrousseau | Open. First release 2026-05-10. 10 stars. |

### Security and features

| Crate | Unsafe code or C parser (C2) | RustSec (C3) | `serde` (C4) | Limits (C5) | YAML (C6) | License (C7) |
|---|---|---|---|---|---|---|
| `serde_yaml` | `unsafe-libyaml` | `unsafe-libyaml` unmaintained | Both | Fixed recursion limit | 1.1 | MIT OR Apache-2.0 |
| `serde_yml` 0.0.13 | None (forwards to `noyalib`) | RUSTSEC-2025-0068, all versions up to 0.0.12 | Both | Same as `noyalib` | 1.2 (via `noyalib`) | MIT OR Apache-2.0 |
| `serde_norway` | `unsafe-libyaml-norway` | None for the crate. Unverified for its fork of `unsafe-libyaml`. | Both | Fixed repetition and recursion limits | 1.1 (inferred from libyaml, unverified) | MIT OR Apache-2.0 |
| `serde_yaml_ng` | `unsafe-libyaml`, 59 `unsafe` hits in own source | RUSTSEC-2024-0449 (`unsafe-libyaml` unmaintained) | Both | Fixed repetition and recursion limits | 1.1 (README) | MIT (0.10.0) |
| `serde_yaml_bw` | `unsafe-libyaml-norway` plus `granit-parser` pre-check | None found | Both | Configurable `Budget` | 1.1 and 1.2 floats (README) | MIT OR Apache-2.0 |
| `serde-saphyr` | None. `#![forbid(unsafe_code)]`. Parser `granit-parser` also forbids it. | None found | Both through `from_str` and `to_string` | Configurable `Budget` (21 fields) | 1.2 (parser), with optional 1.1 boolean forms | MIT OR Apache-2.0 |
| `saphyr` | None. No `unsafe` keyword in source. | None found | No `serde` | Not applicable | 1.2 | MIT OR Apache-2.0 |
| `saphyr-parser` | None. No `unsafe` keyword in source. | None found | No `serde` | Not found in the README (unverified) | 1.2 | MIT OR Apache-2.0 |
| `yaml-rust2` | None. No `unsafe` keyword in source. | None found | No `serde` | Not found in the README (unverified) | 1.2 | MIT OR Apache-2.0 |
| `noyalib` | None. Workspace forbids it. | RUSTSEC-2026-0333, patched in 0.0.53 | Both | Configurable `ParserConfig` limits | 1.2, opt-in 1.1 resolver | MIT OR Apache-2.0 |

## Findings

### The `serde_yaml` family

1. `serde_yaml` has no maintainer. The 0.9.34 release note says the author does not plan more versions [20].
2. `serde_yaml_ng` and `serde_norway` fork `serde_yaml`. Both still depend on the C-to-Rust translation `unsafe-libyaml` or its fork [8][3][12].
3. RUSTSEC-2024-0449 marks `unsafe-libyaml` as unmaintained. The advisory names `saphyr`, `serde-saphyr` and `noyalib` as safe Rust alternatives [15].
4. `serde_yaml_ng` has had no release since 2024-05-26. Its license field changed from `MIT OR Apache-2.0` to `MIT` for 0.10.0. Both pass `deny.toml`.
5. `serde_yaml_bw` still uses `unsafe-libyaml-norway`. Its README recommends `serde-saphyr` over its own crate [24].

### `serde_yml`

1. RUSTSEC-2025-0068 marks all versions up to 0.0.12 as unsound. `serde_yml::ser::Serializer.emitter` can cause a segmentation fault [14].
2. David Tolnay traced the undefined behavior to commits in `serde_yml` and in `libyml`. The code he forked from, `serde_yaml` and `unsafe-libyaml`, does not have the flaw [19].
3. RUSTSEC-2025-0067 marks the `libyml` crate as unsound for a related reason [16].
4. The issue reporter also wrote that the author "seems to act in malicious intent using AI". The issue gives no evidence for this claim. We do not rely on it. Unverified.
5. Release 0.0.13 of 2026-05-27 replaces the code with a shim for `noyalib` and tells users to migrate [2][29]. A shim from the same author does not give an independent review.

### `noyalib`

1. `noyalib` is a pure Rust YAML 1.2 library with `serde` support and parser budgets [10][30].
2. The version is 0.0.57. The crate has 55 published versions in five months. The author does not promise API stability yet. The repository has 10 stars.
3. RUSTSEC-2026-0333 (2026-10-06) shows that the typed deserialization path ignored some budgets before 0.0.53. The default depth, size and alias caps still applied [17].
4. The crate is a credible later candidate. The project age and the author history of `serde_yml` do not meet C1 for a control plane today.

### Parser crates without `serde`

1. `saphyr`, `saphyr-parser` and `yaml-rust2` have no `serde` support [25][26]. The `saphyr` README announces `saphyr-serde` as "soon-to-be" [25].
2. Use of these crates needs our own `serde` layer for profiles. That work has no value for the project and adds a security risk. We reject this option.

### `serde-saphyr`

1. `serde-saphyr` deserializes directly into Rust types. It has `from_str`, `from_reader` and `to_string`. It builds on `granit-parser` and not on `saphyr` [21][27].
2. Both crates forbid `unsafe` code. The default features pull 18 other runtime crates. Some of them, such as `smallvec` and `encoding_rs`, contain `unsafe` code. The forbid attribute does not cover them.
3. The `Budget` type has 21 limits. Examples: `max_depth` 64, `max_nodes` 250 000, `max_aliases` 50 000, `max_total_scalar_bytes` 64 MiB, `max_reader_input_bytes` 256 MiB. The author calls these defaults "intentionally permissive" [22].
4. The library does not build objects from tags. This removes the known YAML remote code execution class [21].
5. The repository has 5 open issues and a push on 2026-10-11. The author wrote 771 of about 850 commits. This is a single-maintainer risk [28].
6. The first release is from 2025-09-27 and 1.0.0 is from 2026-07-31. Versions 1.1, 1.2 and 1.3 came between 2026-08-15 and 2026-09-16.
7. The `Deserializer` type is available only inside a closure. We need only `from_str_with_options` and `to_string`, so this limit has no effect.
8. The crate needs Rust 1.89. The project uses Rust 1.96 (`rust-toolchain.toml`).

### Local experiment

We built a probe program with `serde-saphyr` 1.3.0, `serde_yaml_ng` 0.10.0, `serde_norway` 0.9.42 and `noyalib` 0.0.57 on Rust 1.96.1 (debug build, 2026-10-12).

| Test | `serde-saphyr` | `serde_yaml_ng` | `serde_norway` | `noyalib` |
|---|---|---|---|---|
| Alias bomb: 10 levels, 9 aliases each | Error: node budget, 439 ms | Error: repetition limit, 31 ms | Error: repetition limit, 29 ms | Error: alias limit, 228 ms |
| 100 000 nested flow sequences | Error: recursion limit, 1.6 ms | Error: recursion limit, 1.6 s | Not run | Error: recursion limit, 7.5 ms |
| 5 000 nested block sequences | Error: depth budget, 81 ms | Not run | Not run | Not run |
| Profile round trip (struct, map, list) | Equal | Not run | Not run | Not run |
| `enabled: yes` into `bool` | `true` | Error | Not run | Error |
| String `"yes"` and `"1.10"` on export | Quoted | Not run | Not run | Not run |

All four libraries reject the hostile inputs without a crash or a panic. `serde-saphyr` gives the clearest error text and the most limits that the caller can configure. The experiment is a smoke test. It is not a fuzz test.

`cargo deny check` with the repository `deny.toml` on a crate that depends on `serde-saphyr` 1.3.0 reports: advisories ok, bans ok, licenses ok, sources ok.

## Recommendation

Use `serde-saphyr` for profile import and export in the control plane. It meets C1 to C7:

- It forbids `unsafe` code and has no C dependency (C2).
- It has no RustSec advisory (C3).
- It has configurable resource limits for aliases, nesting and size (C5).
- It supports the YAML 1.2 parser from `granit-parser` and the merge keys of YAML 1.1 (C6).
- It passes `cargo deny` with the present `deny.toml` (C7).

Apply these rules:

1. Put all YAML calls in one module of the control plane. No other module imports `serde-saphyr`.
2. Call `from_str_with_options` with limits that the module defines. Tighten the defaults for profile files. Limit the size of the request body before parsing.
3. Write tests that refer to FR-PF-07 for the alias bomb, deep nesting and a large document.
4. Pin the minor version in `Cargo.toml`. Review each update with `cargo deny` and the release notes.
5. If the maintainer stops work, replace the module. Candidates are `saphyr-serde` and `noyalib` 1.0, after a new spike.

Risks that remain:

- One maintainer wrote most of `serde-saphyr` and `granit-parser`. The ADR records this risk. The single module limits the cost of a replacement.
- The library is young. Fuzz the import path in the test bench before release 1.0.

## Sources

All sources opened on 2026-10-12.

1. crates.io API, `serde_yaml`: https://crates.io/api/v1/crates/serde_yaml
2. crates.io API, `serde_yml`: https://crates.io/api/v1/crates/serde_yml
3. crates.io API, `serde_norway`: https://crates.io/api/v1/crates/serde_norway
4. crates.io API, `serde-saphyr`: https://crates.io/api/v1/crates/serde-saphyr
5. crates.io API, `saphyr`: https://crates.io/api/v1/crates/saphyr
6. crates.io API, `saphyr-parser`: https://crates.io/api/v1/crates/saphyr-parser
7. crates.io API, `yaml-rust2`: https://crates.io/api/v1/crates/yaml-rust2
8. crates.io API, `serde_yaml_ng`: https://crates.io/api/v1/crates/serde_yaml_ng
9. crates.io API, `serde_yaml_bw`: https://crates.io/api/v1/crates/serde_yaml_bw
10. crates.io API, `noyalib`: https://crates.io/api/v1/crates/noyalib
11. crates.io API, `granit-parser`: https://crates.io/api/v1/crates/granit-parser
12. crates.io API, `unsafe-libyaml` and `unsafe-libyaml-norway`: https://crates.io/api/v1/crates/unsafe-libyaml and https://crates.io/api/v1/crates/unsafe-libyaml-norway
13. RustSec advisory database (clone with commit of 2026-10-12): https://github.com/rustsec/advisory-db
14. RUSTSEC-2025-0068, `serde_yml`: https://github.com/rustsec/advisory-db/blob/main/crates/serde_yml/RUSTSEC-2025-0068.md
15. RUSTSEC-2024-0449, `unsafe-libyaml`: https://github.com/rustsec/advisory-db/blob/main/crates/unsafe-libyaml/RUSTSEC-2024-0449.md
16. RUSTSEC-2025-0067, `libyml`: https://github.com/rustsec/advisory-db/blob/main/crates/libyml/RUSTSEC-2025-0067.md
17. RUSTSEC-2026-0333, `noyalib`: https://github.com/rustsec/advisory-db/blob/main/crates/noyalib/RUSTSEC-2026-0333.md
18. RUSTSEC-2018-0005, `serde_yaml` (fixed in 0.8.4): https://github.com/rustsec/advisory-db/blob/main/crates/serde_yaml/RUSTSEC-2018-0005.md
19. Issue "serde_yml is unsound", comments of David Tolnay and others: https://github.com/rustsec/advisory-db/issues/2395
20. `serde_yaml` 0.9.34 release note: https://github.com/dtolnay/serde-yaml/releases/tag/0.9.34
21. `serde-saphyr` README: https://github.com/bourumir-wyngs/serde-saphyr
22. `serde-saphyr` `Budget` documentation: https://docs.rs/serde-saphyr/latest/serde_saphyr/budget/struct.Budget.html
23. `serde_yaml_ng` README: https://github.com/acatton/serde-yaml-ng
24. `serde_yaml_bw` README: https://github.com/bourumir-wyngs/serde-yaml-bw
25. Saphyr README: https://github.com/saphyr-rs/saphyr
26. `yaml-rust2` README: https://github.com/Ethiraric/yaml-rust2
27. `granit-parser` README: https://github.com/bourumir-wyngs/granit-parser
28. GitHub API for repository state, contributors and releases: https://api.github.com/repos/bourumir-wyngs/serde-saphyr and the same path for the other repositories
29. `serde_yml` README: https://github.com/sebastienrousseau/serde_yml
30. `noyalib` README: https://github.com/sebastienrousseau/noyalib
31. Local probe program, `cargo tree` and `cargo deny check` run, 2026-10-12. Not published. Source in `/tmp/opencode/p5/probe`.
32. Repository `deny.toml`: [deny.toml](../../deny.toml)
