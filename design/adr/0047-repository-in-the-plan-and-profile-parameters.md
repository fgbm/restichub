# ADR-0047: The repository in the plan, and profile parameters

- Status: accepted
- Date: 2026-10-18
- Requirements: FR-PF-01, FR-PF-03, FR-PF-09 to FR-PF-12, FR-SC-01, FR-RP-01, FR-RP-08, AC-RP-06, AC-RP-07, AC-RP-12, AC-RP-13, AC-PF-01, AC-PF-06 to AC-PF-10
- Related: [ADR-0034](0034-repository-maintenance-schedule.md) (retention isolation), [ADR-0037](0037-server-scripts-on-trusted-hosts.md) (secret script variables), [ADR-0046](0046-local-repository-bound-to-its-host.md) (the repository host)

## Context

The profile holds the repository (FR-PF-01). A plan binds a profile and a host (FR-SC-01). A local repository is bound to one host (ADR-0046). Thus a profile with a local repository fits the plans of that host only. The operator makes one profile for each host.

The repository in the profile has two more costs. A delete of a repository needs a change of each profile that names it. A change of the repository in a profile moves every plan of the profile, and the user interface does not tell the operator.

A value differs from one host to another: a mount point in a path, a database name, a password for a script. FR-PF-03 gives the host name, the operating system, the agent name, the agent labels and the date. The control plane derives all of them from the agent. A profile has no value that a person sets. Thus an operator that needs such a value copies the profile for each host.

## Decision

### The plan holds the repository

- A profile says what to back up and how to back it up. It holds the paths, the excludes, the tags, the backup options, the scripts, the retention, the check options and the limits. A profile has no repository.
- A plan binds a profile, a host and a repository with a schedule (FR-SC-01).
- The plans of a repository block its delete. The control plane refuses the delete with `409 repository_in_use` and the list of the plans. A bulk move of the plans to another repository is a separate issue after 1.0 (issue #284).
- The retention isolation of ADR-0034 stays. The control plane adds the tag `restichub-profile-<profile id>` to each backup of a plan.
- The retention task of a repository gives one `forget` step for each distinct profile of the plans of the repository. The step takes the agent facts from the plans of this profile in this repository only.
- The check task reads the largest `read_data_subset` among the profiles of the plans of the repository.
- Two plans of one profile on one host in one repository with the same paths share one retention. A parameter in the path separates them (FR-PF-10).
- The control plane checks the rule of ADR-0046 when it saves a plan. A plan of a local repository on another host is refused with the reason `not_repository_host`.
- The data migration gives each plan the repository of its profile.
- The API break comes before v0.1.0. `repository_id` leaves the profile requests and views. It is required in the plan requests.

### Profile parameters

- A profile declares parameters (FR-PF-09). A parameter has a name, a kind, a required flag, a default value and a description.
- The name matches `^[a-z][a-z0-9_]{0,63}$` and is unique in the profile.
- The kind is `text` or `secret`. Only a `text` parameter has a default value. A default secret would give one password to every host.
- A host holds parameter values by name, and every profile of the host reads them. A plan can override a value.
- The control plane resolves a value in this order: the plan, then the host, then the default value.
- A `secret` value is a reference to a secret of the purpose `hook_secret`. The control plane stores no secret text in the value.
- A `text` value expands as `{{param.<name>}}` in the fields that expand `{{hostname}}` today: the paths, the excludes, the exclude files and the tags.
- A parameter does not expand in the retention and check fields. One maintenance step serves every host, as with `retention.filter.hosts` (ADR-0034).
- A script environment variable of the kind "profile parameter" names a parameter. The agent gives the value of the parameter to the script. A stored plain value does not expand, so a stored script does not change.
- A `secret` parameter never goes into a template field. Restic arguments are visible in the process list (FR-AG-06).
- Each saved plan has a value for each required parameter (FR-PF-11). The control plane refuses these writes:
  - A plan save without a value: the code `parameter_missing`, with the names of the parameters.
  - A profile save that leaves a plan without a required value: the code `parameter_missing_in_plans`, with the plans.
  - A change of the host values that leaves a plan without a required value: the same code, with the plans.
  - A value of the wrong kind counts as missing: the code `parameter_kind_mismatch`.
- The job builder checks the values again. It refuses the job with `parameter_missing`, and the job ends with the result `config_error`.
- The replace of a host record (FR-ID-11) moves the host values with the plans and the labels.
- The control plane refuses the delete of a secret that a host value or a plan value references.
- A Viewer does not see `text` values on hosts and plans, as with the string script variables (FR-HK-12).
- The control plane sends a `text` value in a script variable as a masked value, and a `secret` value as the value of its secret. The agent masks both in the logs.
- A preview, a job detail and an audit diff mask the parameter values. The audit keeps the names and the identifiers of the secrets only (FR-PF-12).
- A value in a path, an exclude or a tag is part of the restic command and of the snapshot. Each role that sees jobs and snapshots sees it, and the user interface says so.
- Only an Admin sets, changes or removes a `secret` value on a host or in a plan: it chooses the credential that a script receives. An Operator sets the `text` values.
- `ScriptEnv.masked_value` is a new field, and an agent that does not know it rejects the message. The agent announces the capability `script-masked-value` in `AgentHello.capabilities`, and the control plane keeps the capabilities of the live connection only (ADR-0034). The builder sends such a value as `secret_value` to an agent without the capability: an older agent masks a secret value the same way.
- Labels stay separate. The enrollment sets the labels, and a plan selector uses them (FR-SC-11).

## Consequences

- A profile with a local repository serves the plans of one host only. A profile for many hosts has no local repository.
- A delete of a repository needs no change of a profile. The plans block the delete instead.
- A plan records its repository. A change of the profile moves no plan to another repository.
- The API changes before v0.1.0. A profile request with `repository_id` gets a validation error. A plan request without `repository_id` gets `400` with the reason `empty` on the field.
- The migration moves the repository of each profile to its plans. A profile without a plan loses its repository, and the plan that a person adds later selects the repository.
- One profile serves many hosts, and a host sets a value one time. The operator copies no profile for a password or a mount point.
- A plan save fails while a host value is missing. The plan page names the parameters.
- The retention task of a repository has one `forget` step for each distinct profile of its plans. A repository with many plans of few profiles keeps a short job.
- A `text` value in a path or a tag is in the snapshot of the repository. The documentation tells the operator to keep a credential out of a path.
- A host parameter value lives in the control plane database. A host with a secret value holds a reference only.
- Code follow-up, not part of this change: the migration, the API, the user interface, the job builder and the tests.

## Alternatives

| Alternative | Why we did not select it |
|---|---|
| Keep the repository in the profile | A profile with a local repository fits one host only. A delete needs a change of each profile. A profile change moves the plans without a notice. |
| Profile inheritance with `extends` for the values of each host | It needs N profiles for N hosts, and the values stay in the profile text. |
| Agent labels as the values | A label is set at the enrollment and a person cannot change it. A label holds no secret. It selects hosts (FR-SC-11). |
| Values on the plan only | The password repeats for each plan of a host. A label selector (FR-SC-11, 1.0) needs a value for a host that the plan does not name. |
| A plan tag instead of the profile tag | It changes ADR-0034. A snapshot of a deleted plan loses its retention. |
| A parameter of the kind repository | The plan holds the repository. The parameter adds a second source of truth. |
