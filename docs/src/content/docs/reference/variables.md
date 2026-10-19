---
title: Template variables
description: The variables that a profile field accepts, the date format, and where each variable applies.
---

A profile field can hold a variable. The control plane replaces the variable with a value for each host when it builds a job. A variable has the form `{{name}}`. The name has no space before `{{` and no modifier.

```text
/srv/{{hostname}}/data
/var/backups/{{param.pg_dir}}/{{date:%Y-%m-%d}}
```

The expansion runs one time. A substituted value is not expanded again.

## Host variables

| Variable | Value | Example |
|---|---|---|
| `hostname` | The host name of the agent. | `db-01.example.com` |
| `os` | The operating system of the agent. | `linux`, `windows` or `macos` |
| `agent.name` | The name of the agent in ResticHub. | `db-01` |
| `agent.label.<key>` | The value of the agent label `<key>`. The enrollment sets the label. | `{{agent.label.env}}` gives `prod` |
| `date` | The date of the run, in the format `%Y-%m-%d`. | `2026-10-14` |
| `date:<format>` | The date of the run in your format. | `{{date:%Y%m%d}}` gives `20261014` |

The label must exist on the host of the plan. The control plane refuses the plan or the job when the host does not set the label.

### The date format

| Directive | Value | Example |
|---|---|---|
| `%Y` | The year, four digits. | `2026` |
| `%m` | The month, two digits. | `10` |
| `%d` | The day, two digits. | `14` |
| `%H` | The hour, two digits, 24-hour clock. | `09` |
| `%M` | The minute, two digits. | `42` |
| `%S` | The second, two digits. | `00` |
| `%%` | A percent sign. | `%` |

Any other directive is an error. Text without `%` stays as it is, so `{{date:%Y-%m}}` gives `2026-10`.

## Profile parameters

`{{param.<name>}}` gives the value of a declared parameter. The control plane resolves the value in this order: the plan, the host, the default value. See [Profile parameters](/restichub/concepts/profile-parameters/).

A parameter of the type "Secret" never expands. The restic command line is visible in the process list.

## Where a variable applies

| Field | Host variables | Profile parameters |
|---|---|---|
| Paths, each system | yes | yes |
| Exclusions | yes | yes |
| Exclude file on the host | yes | yes |
| Tags | yes | yes |
| Retention rules: keep within, keep tags, filter tags, filter paths | yes | no |
| Retention rules: filter hosts | no | no |
| Repository check: read-data subset | yes | no |

A parameter does not expand in the retention or the check, because one maintenance step serves every host of the repository. The control plane refuses it with `parameter_not_allowed`. The filter hosts accept no variable, because the step of one host has no single host name.

A value in a path, an exclusion or a tag is part of the restic command and of the snapshot. Each role that sees jobs and snapshots sees it.

## The list in the profile editor

The fields for the paths, the exclusions, the exclude file and the tags open a list when you type `{{`. The list holds two groups:

- "Profile parameters": each declared parameter with its type and its description.
- "Host variables": the variables of the table above, with an example for each.

The text after `{{` filters the list. The footer counts the rows. These keys work in the list:

| Key | Action |
|---|---|
| `↑`, `↓` | Move the highlight. A secret stays dimmed and the arrows skip it. |
| Enter | Insert the whole variable with its closing braces. |
| Esc | Close the list and keep the text. |

A secret parameter in the list gives the reason why it cannot go into a path or a tag. In the tags field, Enter belongs to the list while the list is open. The second Enter adds the chip.

A name that is not declared gives the hint "The parameter … is not declared." under the field, with the button "Declare". The button adds a row of the type "Text" with "Required" on.

## Errors

| Error | Cause |
|---|---|
| `unknown variable` | The name is not in the tables above. |
| `agent label … is missing` | No host of the plans sets the label. |
| `invalid date format` | The format holds a directive other than the table above. |
| `unterminated variable` | The closing `}}` is missing. |
| `the parameter … has no value` | No source gives a value to the parameter. |
| `the parameter … is a secret and cannot be expanded` | A secret parameter stands in a path, an exclude or a tag. |
| `the parameter … cannot expand in this field` | A parameter stands in a retention or check field. |

## Related pages

- [Profiles](/restichub/concepts/profiles/)
- [Profile parameters](/restichub/concepts/profile-parameters/)
- [Set parameter values](/restichub/guides/profile-parameters/)
