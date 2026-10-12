# Crates

| Crate | Type | Contents |
|---|---|---|
| `restichub-proto` | library | Protocol Buffers contract between the agent and the control plane (ADR-0008) |
| `restichub-common` | library | Shared types: identifiers, job statuses, restic exit codes |
| `restichub-db` | library | Database pool, migrations and readiness check (ADR-0023) |
| `restichub-secrets` | library | Secret values and secret sources (ADR-0023) |
| `restichub-compiler` | library | Profile compiler: profile to steps (FR-PF-04) |
| `restichub-agent` | binary | Agent (FR-AG-*) |
| `restichub-cp` | binary | Control plane (ADR-0007) |
| `restichub-emulator` | binary | Agent emulator for load tests (bench S2) |
