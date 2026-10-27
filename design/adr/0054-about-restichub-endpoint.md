# ADR-0054: The About ResticHub endpoint and its role boundary

- Status: accepted
- Date: 2026-10-27
- Requirements: none. The owner set the screen in issue #417.

## Context

A person who asks support for help must give the exact build of the control plane. Today the only machine-readable copy of the version is `info.version` of the OpenAPI document at `GET /api/v1/openapi.json`. That document is public and holds no role boundary.

The web UI must not parse the version string. The string holds a package version, an optional pre-release part and a build revision. A parser in the UI would repeat the rule of the build script and drift from it.

An operator also needs two facts of the installation when an update fails. They are the tenancy mode (ADR-0024) and the version of the last applied database migration. The tenancy mode is already public on `GET /api/v1/instance`, and the UI reads that route at load.

## Decision

1. We add `GET /api/v1/about`. The operation is `about_read` and the minimum role is Viewer. Its class is the class of the account operations. Thus every authenticated role reads it, also in single mode and in a session without an organization. The platform tenant passes the account operations only (ADR-0029).
2. The body is a parsed structure. It holds the package version with its pre-release part, a `prerelease` flag, the revision and the protocol major and minor versions. It also holds the agent protocol range: the major version and the oldest accepted minor version. The range follows the N-1 rule of FR-PR-09. It states no upper minor bound, because a newer minor version works.
3. The body does not repeat the tenancy mode. `GET /api/v1/instance` reports it to an unauthenticated client already. A second copy behind a role boundary would promise a privacy that the endpoint does not give.
4. The database schema level is the `max(version)` of `_sqlx_migrations`. The body holds it only for an account with the Platform Admin right. The field is absent for every other role, and absent while the table holds no migration row. The UI shows the absent value as "Not set".
5. The body holds no secret, no environment value and no file path. The version is the same string as `info.version` of the public OpenAPI document. The route therefore adds no disclosure beyond that document.

## Consequences

- Support gets one line that a person copies from the screen. The line holds the version with the revision, the protocol and the agent range.
- The UI never parses a version string. A change of the pre-release format changes only the server.
- A role below the Platform Admin does not learn the state of the database. A binary that does not match the schema is an operator problem.
- The version and the revision stay public: an unauthenticated client reads them from the OpenAPI document. The threat model accepts that (T37).
- One more route needs the isolation fixture of AC-TN-01 and the OpenAPI document. One more operation joins the role matrix of AC-AU-02.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Return one formatted string, for example `0.2.0-rc.2+3b21922` | The UI would parse it to place the "Pre-release" badge, and two parsers drift. |
| Repeat the tenancy mode in the body | `GET /api/v1/instance` is public and reports it, so the copy protects nothing. |
| Make the route public like the OpenAPI document | The screen is behind the sign-in. A public route would put the schema level outside the platform right or split it into a second route. |
| Show the facts of the installation to every role | The schema level tells the state of the whole installation, and the role matrix gives that state to the Platform Admin (ADR-0029). |
