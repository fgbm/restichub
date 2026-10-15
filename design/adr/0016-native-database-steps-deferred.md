# ADR-0016: Native database steps are deferred

- Status: accepted
- Date: 2026-10-12
- Requirements: D4, section 5 of the requirements

## Context

A native MSSQL step needs streaming through the SQL Server Virtual Device Interface (VDI), a COM interface. No Rust binding for VDI was found. Full support also needs DIFF and LOG backups, LSN chains and point-in-time restore. This work is large and specific to one database.

## Decision

Native database steps are not in the MVP and not in 1.0. Application consistency comes from scripts on trusted hosts, for example a dump before the backup.

## Consequences

- The MVP scope stays small.
- Users of MSSQL use scripts with `sqlcmd` or other tools.
- The step model (typed `oneof`) permits new step types later without a protocol break.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Native MSSQL step in the MVP | Large scope and technical risk (VDI binding). |
