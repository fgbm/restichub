# ADR-0015: Pluggable job log storage

- Status: accepted
- Date: 2026-10-12
- Requirements: FR-LG-01 to FR-LG-04

## Context

Job logs can be large. A small installation wants everything in PostgreSQL. A large installation wants logs in object storage. Apache Airflow solves this with remote logging: the connection and the retention are separate settings.

## Decision

- A `LogStore` trait hides the storage. Backends: `database` (default, MVP) and `s3` (1.0).
- The `s3` backend uses the `object_store` crate (Apache Arrow). It also supports Azure, GCS and local files.
- The connection settings and the retention settings are separate. Retention works the same way for all backends and can be different for each tenant.
- Job metadata is always in the database.
- Object keys: `<tenant>/<job>/<step>`.

## Consequences

- A small installation needs only PostgreSQL.
- A change of the backend does not make old logs unavailable. The control plane reads each log from the backend that wrote it.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Only the database | Database growth on large installations. |
| Only S3 | A mandatory extra component for small installations. |
| Own S3 client | `object_store` is mature and supports more backends. |
