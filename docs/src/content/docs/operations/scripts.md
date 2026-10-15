---
title: Server scripts
description: How to run scripts before and after a backup, how a host administrator turns the trust mode on, and what risk this mode accepts.
---

A server script is a script that you write in the web interface. The control plane sends it with the job. The agent runs it on the host before or after the backup. Each script is a step of the job with its own status, duration and log.

A host runs server scripts only when its administrator turns on the local trust mode. The mode is off by default (ADR-0037). A host without the mode can still run [allow-list hooks](hooks/).

## Risk

:::caution
A server script runs as root on Linux and as LocalSystem on Windows. Turn on the trust mode only on a host that you want the control plane to control.
:::

On a host with the trust mode, a person who controls the control plane can run any code on that host. The same is true for a person who controls the account of an Admin. The control plane has these protections:

- The trust mode is local. Only root or an Administrator can turn it on. A job cannot change it.
- The control plane signs each job. The signature covers the text of every script.
- Only an Admin can change a script or a template. An Operator and a Viewer can read them.
- The audit log records each change of a script and each job that carries scripts. Each record holds the SHA-256 hash of the script text.
- A secret reaches a script only in an environment variable. It never appears in the text, in an argument or in the log.

## Turn the trust mode on

1. Log in to the host as root or as an Administrator.
2. Run the command:

   ```sh
   sudo restichub-agent trust-server-scripts --enable
   ```

   On Windows, start an elevated PowerShell and run `restichub-agent.exe trust-server-scripts --enable`.
3. Open **Hosts** in the web interface. The column **Server scripts** shows **Enabled** after the agent connects again. The agent sends the state each time it connects, so the column does not change until the next connection. To see the new state at once, restart the agent service.

To turn the mode off, run the command with `--disable`. The agent reads the setting for each job, so the change applies to the next job.

The command writes a small file next to the configuration directory of the agent.

| System | File |
|---|---|
| Linux | `/etc/restichub-agent-server-scripts` |
| Windows | `%ProgramData%\ResticHub\Agent-server-scripts` |

The agent trusts the file only when root or Administrators own it and no other account can write it or the directory above it. If another account can write the file, the mode is off. The host list then shows **Disabled, the setting file is not trusted**. A job, the control plane and a self-update cannot change the file.

The agent reports the mode and the interpreters that it found. The control plane keeps this report. It refuses a plan or a job that holds a script for a host without the mode. The agent refuses such a job too and answers with the reason `server_scripts_disabled`.

## Write a script

1. Open **Profiles**. Select **Add profile**, or select **Edit** for a profile.
2. Go to the section **Scripts**.
3. In the list **Before the backup** or **After the backup**, select **Blank script** or **From a script template**.
4. Enter a name, select the interpreter and write the script.
5. Select **Save**.

Only an Admin can change the scripts of a profile. An Operator sees the scripts and can change the other settings of the profile.

The section has two lists. The scripts of the first list run before the backup. The scripts of the second list run after it. The order of a list is the order of the run. Use the arrows to move a script.

| Phase | When the script runs |
|---|---|
| Before | Before the backup. |
| After | After the backup, also when the backup failed. |
| On success | After the backup, when no step failed. |
| On error | After the backup, when a step failed. |
| Finally | At the end of the job, also after a cancel. |

### Interpreters

| Interpreter | System | Command line of the agent |
|---|---|---|
| Bash | Linux, macOS | `bash --noprofile --norc <file>` |
| sh | Linux, macOS | `sh <file>` |
| PowerShell | Windows | `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File <file>` |
| cmd | Windows | `cmd.exe /D /E:ON /V:OFF /C <file>` |

The flags are fixed. A Group Policy can set the PowerShell execution policy for the machine. That policy overrides `-ExecutionPolicy Bypass`. PowerShell then refuses to load the script, exits with a non-zero code, and the step fails with the error in its log. Ask the domain administrator to allow the policy `RemoteSigned` or `Bypass` for the agent host. PowerShell does not stop at the first error by itself. Start a PowerShell script with `$ErrorActionPreference = 'Stop'`. Start a Bash script with `set -euo pipefail`.

### Failure policy and time limit

The policy decides what a failed script does:

- **Stop the backup.** A failed script of the phase Before stops the backup. The scripts of the phases On error and Finally still run. The job fails.
- **Go on and report a warning.** The job goes on. The step shows as failed, and the job ends with a warning.

The time limit is in seconds. An empty value means one hour. The longest limit is 24 hours. When the limit ends, the agent stops the script and all its child processes.

### Parameters and secrets

A script reads its parameters as environment variables. A parameter is plain text or a secret.

For a secret, select a stored secret of the purpose `hook_secret`. Select **New secret** to create one. The control plane reads the value only when it sends the job, and writes the audit record `secret_delivered`. The agent masks the value in the log of the script. The value never goes into an argument or a file.

## Templates

A script template is a ready script with declared parameters. A profile gets a copy of the template. A later change of the template does not change the profile.

Open **Settings**, then **Script templates**, to see the templates. The built-in templates are read-only. Select **Make a copy** to change one. The control plane ships these built-in templates:

| Template | System |
|---|---|
| PostgreSQL dump with pg_dump | Linux, Windows |
| MySQL dump with mysqldump | Linux |
| MSSQL backup with sqlcmd | Windows |
| Windows VSS note | Windows |
| Stop a service, start a service | Linux, Windows |
| Free disk space check | Linux, Windows |
| Run a command | Linux, Windows |

A job template is a set of scripts in a fixed order. In the profile form, select **Use a job template** to add its scripts. Open **Settings**, then **Job templates**, to manage them. A secret stays empty in a job template. Select the secret in the profile.

## What the agent does

For each script, the agent does these steps:

1. It checks the trust mode and the script again.
2. It writes the script to a new file in a protected directory of its cache. Only the agent account can use the directory.
3. It starts the interpreter with the fixed flags. The environment is the environment of the agent, without the variables that start with `RESTIC`, plus the parameters of the script. The search path (`PATH`) holds only directories that root or Administrators own and that no other account can write, with every directory above them. The agent drops each other directory and names it once for each job in the log of the first script. On Windows, `TEMP` and `TMP` point to the protected directory. The working directory is the protected directory.
4. It reads the output. It masks secret values and keeps at most 256 KiB for each script.
5. It deletes the file and the directory.

On a timeout or a cancel, the agent asks the script to stop. It kills the whole process tree after a short grace period. A second cancel kills the tree at once.

The scripts of the phases After, On success, On error and Finally have a time limit past the end of the job. They run for the time left plus at most 10 minutes, or for their own limit if that is shorter.

A process that leaves the process group, for example with `setsid` or `nohup`, is outside the tree that the agent kills. On Linux, the service setting `KillMode=control-group` of the agent kills such a process when the service stops. Do not start a long-lived process from a script.

If the agent restarts while a job with a script runs, the agent does not run the job again. It answers with the final result `interrupted`, because a script may not run twice. A job that already finished returns its stored result, also after you turned the mode off.

On Linux, the service of the agent runs with `ProtectSystem=strict`. A script can write only to the cache directory, the configuration directory and the restore roots. To allow another path, add a drop-in file:

```ini
# systemctl edit restichub-agent
[Service]
ReadWritePaths=/var/backups
```

## Job detail

The job detail lists every script and hook in order before the job runs. Each step shows its status, start time, duration and exit code. Select a step to read its own log. The backup step has its own log too. The complete log of the job stays below the steps.

## Audit

| Action | When |
|---|---|
| `profile_create`, `profile_update` | A profile changes. The record holds the name, the phase and the hash of each script, and the names of the parameters. |
| `script_template_create`, `script_template_update`, `script_template_delete` | A script template changes. |
| `job_template_create`, `job_template_update`, `job_template_delete` | A job template changes. |
| `script_run` | The control plane sends a job with scripts. The record holds the name, the phase and the hash of each script. |
| `secret_delivered` | The control plane sends a secret to an agent. |

No record holds the text of a script or the value of a parameter.

## Roles

| Action | Admin | Operator | Viewer |
|---|---|---|---|
| Read scripts and templates | yes | yes | yes |
| Change the scripts of a profile | yes | no | no |
| Change a template | yes | no | no |
| Run a plan | yes | yes | no |
