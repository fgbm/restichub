# ADR-0055: The environment of the server in the About ResticHub answer

- Status: accepted
- Date: 2026-10-28
- Requirements: none. The owner set the environment facts of the About screen on 2026-10-28.
- Supersedes: decision 5 of ADR-0054, and only its clause about an environment value. The other decisions of ADR-0054 stay in force.

## Context

A person who asks support for help gives the build of the control plane. The version and the revision are often enough. A problem can depend on the system of the server. An example is a package of the operating system, a kernel module or a PostgreSQL server of another major version. For such a problem support needs more than the build. Support asks for these facts, and a person collects them by hand from the host.

The About screen is the one place where every signed-in role reads the facts of the build (ADR-0054). The environment of the server belongs to the same answer, so one copy of the support text is enough for support.

The tenancy mode stays out of the body (ADR-0054, decision 3). `GET /api/v1/instance` reports it to an unauthenticated client already, and the UI reads that route at load.

## Decision

1. `GET /api/v1/about` carries the object `environment`. The object holds these fields:
   - `os`: the operating system family of the process, for example `linux` or `windows`.
   - `os_version`: the distribution on Linux, read from `PRETTY_NAME` of `/etc/os-release`. The field is absent on another system. When `PRETTY_NAME` is absent, the field holds `NAME` and `VERSION`.
   - `kernel`: the release of the running kernel on Linux, read from `/proc/sys/kernel/osrelease`. The field is absent on another system.
   - `arch`: the architecture of the process, for example `x86_64`.
   - `container`: true when the process finds a container marker of Docker or Podman, or a container runtime in its cgroup path. False on another system, because the process reports no container state there.
   - `database`: the server version of PostgreSQL, read with `SHOW server_version` without the suffix of the package builder. The route does not call `version()`, because that function adds the compiler and the build options.
2. Every signed-in role reads `environment`, also in the platform mode and in a session without an organization. The field does not depend on a role or on a tenant.
3. The body holds no host name, no IP address, no file path, no environment variable, no database name, no user name and no connection string. The module that reads the environment keeps these values out of the object. A fact that the system does not expose stays absent. The route invents no value.
4. The support text of the screen holds the version, the revision, the protocol, the range of the agents and the environment in one copy. The page adds the tenancy mode from the existing instance store to the same text.

## Consequences

- Support gets the system of the server in the same copy as the build. A person does not collect the facts from the host.
- The body adds no field for a role. The role matrix of AC-AU-02 does not change.
- The database version costs one `SHOW server_version` on the pool for each read of the route. The other facts are read from files one time in the life of the process.
- The answer tells a signed-in user the distribution, the kernel and the architecture of the server.

## Accepted risk

An attacker who holds a signed-in account learns the distribution, the kernel version, the architecture, the container state and the PostgreSQL version of the server. These facts help to select a known weakness of one of these packages. The owner accepted this risk on 2026-10-28:
- The route needs a session, and support needs the facts from every role, so a narrower role boundary would break the goal of the screen.
- The version and the revision of the build are public in `GET /api/v1/openapi.json` (T37). The environment narrows the same public information.
- An installation that refuses this disclosure runs the control plane in a container. The value of `os_version` and of `kernel` then describes the container, and not the host (ADR-0055, `container`).

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Show the environment to the Platform Admin only | A Viewer and an Operator ask support too, and the environment is a fact of the build in the same way as the version. |
| Return `version()` of PostgreSQL | The value holds the compiler and the build options. It is long and it says more than support needs. |
| Read the distribution from the host of the agent | The answer is about the control plane. The agent reports its own facts. |
| Add the tenancy mode to the body | `GET /api/v1/instance` is public and reports it (ADR-0054, decision 3). |
| Return the whole content of `/etc/os-release` | The file holds an identifier of the installation (`ID`, `HOME_URL`) and can hold a host name in `VARIANT`. |
