//! The built-in script templates and job templates (FR-HK-12).
//!
//! They ship with the control plane, are read-only and hold no secret. Each one declares the
//! parameters that the form shows; the value of a parameter reaches the script as an environment
//! variable, never inside the text of the script.

use super::templates::{
    JobTemplateView, ScriptTemplateParameter, ScriptTemplateView, TemplateParameterKind,
};
use super::{EnvKind, FailurePolicy, Interpreter, ScriptDef, ScriptEnvDef};
use crate::profiles::HookPhase;

struct Builtin {
    slug: &'static str,
    name: &'static str,
    description: &'static str,
    interpreter: Interpreter,
    phase: HookPhase,
    timeout_seconds: u32,
    failure_policy: FailurePolicy,
    parameters: &'static [(
        &'static str,
        &'static str,
        TemplateParameterKind,
        Option<&'static str>,
    )],
    body: &'static str,
}

const PG_DUMP_BASH: &str = r#"set -euo pipefail
mkdir -p "$DUMP_DIR"
target="$DUMP_DIR/$PGDATABASE.dump"
pg_dump --format=custom --file="$target" "$PGDATABASE"
echo "dump written: $target"
"#;

const PG_DUMP_POWERSHELL: &str = r#"$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $env:DUMP_DIR | Out-Null
$target = Join-Path $env:DUMP_DIR "$($env:PGDATABASE).dump"
& pg_dump.exe --format=custom --file=$target $env:PGDATABASE
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Output "dump written: $target"
"#;

const MYSQL_DUMP_BASH: &str = r#"set -euo pipefail
mkdir -p "$DUMP_DIR"
target="$DUMP_DIR/$MYSQL_DATABASE.sql"
mysqldump --host="$MYSQL_HOST" --user="$MYSQL_USER" --single-transaction --routines \
  --result-file="$target" "$MYSQL_DATABASE"
echo "dump written: $target"
"#;

const MSSQL_BACKUP_POWERSHELL: &str = r#"$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $env:BACKUP_DIR | Out-Null
$file = Join-Path $env:BACKUP_DIR "$($env:MSSQL_DATABASE).bak"
$query = "BACKUP DATABASE [$($env:MSSQL_DATABASE)] TO DISK = N'$file' WITH INIT, COMPRESSION, CHECKSUM"
& sqlcmd.exe -b -S $env:SQLCMDSERVER -Q $query
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Output "backup written: $file"
"#;

const VSS_NOTE_POWERSHELL: &str = r#"# The profile option "Use a file system snapshot" makes restic read the files through a VSS
# snapshot. This script only lists the writers, so the job log shows their state before the backup.
$ErrorActionPreference = 'Stop'
& vssadmin.exe list writers
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
"#;

const SYSTEMCTL_STOP_BASH: &str = r#"set -euo pipefail
systemctl stop "$SERVICE_NAME"
echo "stopped: $SERVICE_NAME"
"#;

const SYSTEMCTL_START_BASH: &str = r#"set -euo pipefail
systemctl start "$SERVICE_NAME"
echo "started: $SERVICE_NAME"
"#;

const SERVICE_STOP_POWERSHELL: &str = r#"$ErrorActionPreference = 'Stop'
Stop-Service -Name $env:SERVICE_NAME -Force
Write-Output "stopped: $($env:SERVICE_NAME)"
"#;

const SERVICE_START_POWERSHELL: &str = r#"$ErrorActionPreference = 'Stop'
Start-Service -Name $env:SERVICE_NAME
Write-Output "started: $($env:SERVICE_NAME)"
"#;

const DISK_CHECK_BASH: &str = r#"set -euo pipefail
free_kb=$(df --output=avail -k "$CHECK_PATH" | tail -n 1 | tr -d ' ')
need_kb=$((MIN_FREE_GB * 1024 * 1024))
echo "free: $((free_kb / 1024 / 1024)) GB, needed: $MIN_FREE_GB GB"
if [ "$free_kb" -lt "$need_kb" ]; then
  echo "not enough free space on $CHECK_PATH" >&2
  exit 1
fi
"#;

const DISK_CHECK_POWERSHELL: &str = r#"$ErrorActionPreference = 'Stop'
$drive = (Get-Item -Path $env:CHECK_PATH).PSDrive
$freeGb = [math]::Floor($drive.Free / 1GB)
Write-Output "free: $freeGb GB, needed: $($env:MIN_FREE_GB) GB"
if ($freeGb -lt [int]$env:MIN_FREE_GB) {
  Write-Error "not enough free space on $($env:CHECK_PATH)"
  exit 1
}
"#;

const RUN_COMMAND_BASH: &str = r#"set -euo pipefail
eval "$COMMAND"
"#;

const RUN_COMMAND_POWERSHELL: &str = r#"$ErrorActionPreference = 'Stop'
Invoke-Expression $env:COMMAND
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
"#;

use TemplateParameterKind::{Integer, SecretRef, String as Text};

const BUILTINS: &[Builtin] = &[
    Builtin {
        slug: "postgres-dump-bash",
        name: "PostgreSQL dump with pg_dump (Linux)",
        description: "Writes a custom-format dump of one database before the backup. Needs pg_dump on the host.",
        interpreter: Interpreter::Bash,
        phase: HookPhase::Before,
        timeout_seconds: 3600,
        failure_policy: FailurePolicy::AbortBackup,
        parameters: &[
            ("PGHOST", "Database host", Text, Some("localhost")),
            ("PGPORT", "Port", Integer, Some("5432")),
            ("PGUSER", "User", Text, Some("postgres")),
            ("PGPASSWORD", "Password", SecretRef, None),
            ("PGDATABASE", "Database", Text, None),
            (
                "DUMP_DIR",
                "Dump directory",
                Text,
                Some("/var/backups/postgres"),
            ),
        ],
        body: PG_DUMP_BASH,
    },
    Builtin {
        slug: "postgres-dump-powershell",
        name: "PostgreSQL dump with pg_dump (Windows)",
        description: "Writes a custom-format dump of one database before the backup. Needs pg_dump.exe in the PATH of the agent.",
        interpreter: Interpreter::Powershell,
        phase: HookPhase::Before,
        timeout_seconds: 3600,
        failure_policy: FailurePolicy::AbortBackup,
        parameters: &[
            ("PGHOST", "Database host", Text, Some("localhost")),
            ("PGPORT", "Port", Integer, Some("5432")),
            ("PGUSER", "User", Text, Some("postgres")),
            ("PGPASSWORD", "Password", SecretRef, None),
            ("PGDATABASE", "Database", Text, None),
            (
                "DUMP_DIR",
                "Dump directory",
                Text,
                Some(r"C:\Backups\postgres"),
            ),
        ],
        body: PG_DUMP_POWERSHELL,
    },
    Builtin {
        slug: "mysql-dump-bash",
        name: "MySQL dump with mysqldump (Linux)",
        description: "Writes a consistent dump of one InnoDB database before the backup. Needs mysqldump on the host.",
        interpreter: Interpreter::Bash,
        phase: HookPhase::Before,
        timeout_seconds: 3600,
        failure_policy: FailurePolicy::AbortBackup,
        parameters: &[
            ("MYSQL_HOST", "Database host", Text, Some("localhost")),
            ("MYSQL_USER", "User", Text, Some("root")),
            ("MYSQL_PWD", "Password", SecretRef, None),
            ("MYSQL_DATABASE", "Database", Text, None),
            (
                "DUMP_DIR",
                "Dump directory",
                Text,
                Some("/var/backups/mysql"),
            ),
        ],
        body: MYSQL_DUMP_BASH,
    },
    Builtin {
        slug: "mssql-backup-powershell",
        name: "MSSQL backup with sqlcmd (Windows)",
        description: "Runs BACKUP DATABASE to a .bak file before the file backup. Needs sqlcmd.exe in the PATH of the agent.",
        interpreter: Interpreter::Powershell,
        phase: HookPhase::Before,
        timeout_seconds: 7200,
        failure_policy: FailurePolicy::AbortBackup,
        parameters: &[
            ("SQLCMDSERVER", "Server", Text, Some("localhost")),
            ("SQLCMDUSER", "User", Text, Some("sa")),
            ("SQLCMDPASSWORD", "Password", SecretRef, None),
            ("MSSQL_DATABASE", "Database", Text, None),
            (
                "BACKUP_DIR",
                "Backup directory",
                Text,
                Some(r"C:\Backups\mssql"),
            ),
        ],
        body: MSSQL_BACKUP_POWERSHELL,
    },
    Builtin {
        slug: "windows-vss-note-powershell",
        name: "Windows VSS note (Windows)",
        description: "Lists the VSS writers before the backup. The snapshot itself comes from the profile option for a file system snapshot.",
        interpreter: Interpreter::Powershell,
        phase: HookPhase::Before,
        timeout_seconds: 300,
        failure_policy: FailurePolicy::Continue,
        parameters: &[],
        body: VSS_NOTE_POWERSHELL,
    },
    Builtin {
        slug: "service-stop-bash",
        name: "Stop a service (Linux)",
        description: "Stops a systemd service before the backup. Pair it with the start template in the finally phase.",
        interpreter: Interpreter::Bash,
        phase: HookPhase::Before,
        timeout_seconds: 300,
        failure_policy: FailurePolicy::AbortBackup,
        parameters: &[("SERVICE_NAME", "Service", Text, None)],
        body: SYSTEMCTL_STOP_BASH,
    },
    Builtin {
        slug: "service-start-bash",
        name: "Start a service (Linux)",
        description: "Starts a systemd service. Use it in the finally phase, so the service starts again when the backup fails.",
        interpreter: Interpreter::Bash,
        phase: HookPhase::Finally,
        timeout_seconds: 300,
        failure_policy: FailurePolicy::Continue,
        parameters: &[("SERVICE_NAME", "Service", Text, None)],
        body: SYSTEMCTL_START_BASH,
    },
    Builtin {
        slug: "service-stop-powershell",
        name: "Stop a service (Windows)",
        description: "Stops a Windows service before the backup. Pair it with the start template in the finally phase.",
        interpreter: Interpreter::Powershell,
        phase: HookPhase::Before,
        timeout_seconds: 300,
        failure_policy: FailurePolicy::AbortBackup,
        parameters: &[("SERVICE_NAME", "Service", Text, None)],
        body: SERVICE_STOP_POWERSHELL,
    },
    Builtin {
        slug: "service-start-powershell",
        name: "Start a service (Windows)",
        description: "Starts a Windows service. Use it in the finally phase, so the service starts again when the backup fails.",
        interpreter: Interpreter::Powershell,
        phase: HookPhase::Finally,
        timeout_seconds: 300,
        failure_policy: FailurePolicy::Continue,
        parameters: &[("SERVICE_NAME", "Service", Text, None)],
        body: SERVICE_START_POWERSHELL,
    },
    Builtin {
        slug: "disk-space-check-bash",
        name: "Free disk space check (Linux)",
        description: "Fails when the path has less free space than the limit.",
        interpreter: Interpreter::Bash,
        phase: HookPhase::Before,
        timeout_seconds: 60,
        failure_policy: FailurePolicy::AbortBackup,
        parameters: &[
            ("CHECK_PATH", "Path", Text, Some("/")),
            (
                "MIN_FREE_GB",
                "Minimum free space in GB",
                Integer,
                Some("10"),
            ),
        ],
        body: DISK_CHECK_BASH,
    },
    Builtin {
        slug: "disk-space-check-powershell",
        name: "Free disk space check (Windows)",
        description: "Fails when the drive of the path has less free space than the limit.",
        interpreter: Interpreter::Powershell,
        phase: HookPhase::Before,
        timeout_seconds: 60,
        failure_policy: FailurePolicy::AbortBackup,
        parameters: &[
            ("CHECK_PATH", "Path", Text, Some(r"C:\")),
            (
                "MIN_FREE_GB",
                "Minimum free space in GB",
                Integer,
                Some("10"),
            ),
        ],
        body: DISK_CHECK_POWERSHELL,
    },
    Builtin {
        slug: "run-command-bash",
        name: "Run a command (Linux)",
        description: "Runs one command line with bash. Use a dedicated template for anything that needs quoting.",
        interpreter: Interpreter::Bash,
        phase: HookPhase::Before,
        timeout_seconds: 600,
        failure_policy: FailurePolicy::AbortBackup,
        parameters: &[("COMMAND", "Command", Text, None)],
        body: RUN_COMMAND_BASH,
    },
    Builtin {
        slug: "run-command-powershell",
        name: "Run a command (Windows)",
        description: "Runs one command line with PowerShell.",
        interpreter: Interpreter::Powershell,
        phase: HookPhase::Before,
        timeout_seconds: 600,
        failure_policy: FailurePolicy::AbortBackup,
        parameters: &[("COMMAND", "Command", Text, None)],
        body: RUN_COMMAND_POWERSHELL,
    },
];

fn view(template: &Builtin) -> ScriptTemplateView {
    ScriptTemplateView {
        id: format!("builtin:{}", template.slug),
        builtin: true,
        name: template.name.to_owned(),
        description: template.description.to_owned(),
        interpreter: template.interpreter,
        phase: template.phase,
        body: template.body.to_owned(),
        timeout_seconds: template.timeout_seconds,
        failure_policy: template.failure_policy,
        parameters: template
            .parameters
            .iter()
            .map(|(name, label, kind, default)| ScriptTemplateParameter {
                name: (*name).to_owned(),
                label: (*label).to_owned(),
                kind: *kind,
                default_value: default.map(str::to_owned),
            })
            .collect(),
        created_at: None,
        updated_at: None,
    }
}

pub(super) fn script_templates() -> Vec<ScriptTemplateView> {
    BUILTINS.iter().map(view).collect()
}

/// Looks up by the name after `builtin:`.
pub(super) fn script_template(slug: &str) -> Option<ScriptTemplateView> {
    BUILTINS
        .iter()
        .find(|template| template.slug == slug)
        .map(view)
}

/// A script of a job template, made from a built-in script template. A secret variable stays
/// empty: the user picks the secret when the profile is made.
fn script_of(slug: &str, overrides: &[(&str, &str)]) -> Option<ScriptDef> {
    let template = script_template(slug)?;
    let env = template
        .parameters
        .iter()
        .map(|parameter| {
            let value = overrides
                .iter()
                .find(|(name, _)| *name == parameter.name)
                .map(|(_, value)| (*value).to_owned())
                .or_else(|| parameter.default_value.clone())
                .unwrap_or_default();
            ScriptEnvDef {
                name: parameter.name.clone(),
                kind: if parameter.kind == TemplateParameterKind::SecretRef {
                    EnvKind::SecretRef
                } else {
                    EnvKind::String
                },
                value: if parameter.kind == TemplateParameterKind::SecretRef {
                    String::new()
                } else {
                    value
                },
            }
        })
        .collect();
    Some(ScriptDef {
        name: template.name,
        interpreter: template.interpreter,
        phase: template.phase,
        body: template.body,
        timeout_seconds: template.timeout_seconds,
        failure_policy: template.failure_policy,
        env,
        template_id: Some(template.id),
    })
}

fn job(
    slug: &str,
    name: &str,
    description: &str,
    scripts: &[(&str, &[(&str, &str)])],
) -> JobTemplateView {
    JobTemplateView {
        id: format!("builtin:{slug}"),
        builtin: true,
        name: name.to_owned(),
        description: description.to_owned(),
        scripts: scripts
            .iter()
            .filter_map(|(template, overrides)| script_of(template, overrides))
            .collect(),
        created_at: None,
        updated_at: None,
    }
}

pub(super) fn job_templates() -> Vec<JobTemplateView> {
    vec![
        job(
            "postgres-linux",
            "PostgreSQL server (Linux)",
            "Checks the free space, dumps one database before the backup and removes nothing after it.",
            &[
                ("disk-space-check-bash", &[("CHECK_PATH", "/var/backups")]),
                ("postgres-dump-bash", &[]),
            ],
        ),
        job(
            "mssql-windows",
            "Microsoft SQL Server (Windows)",
            "Lists the VSS writers and backs up one database to a .bak file before the file backup.",
            &[
                ("windows-vss-note-powershell", &[]),
                ("mssql-backup-powershell", &[]),
            ],
        ),
        job(
            "stop-start-service-linux",
            "Application with a service (Linux)",
            "Stops a service before the backup and starts it again in the finally phase.",
            &[("service-stop-bash", &[]), ("service-start-bash", &[])],
        ),
    ]
}

/// Looks up by the name after `builtin:`.
pub(super) fn job_template(slug: &str) -> Option<JobTemplateView> {
    job_templates()
        .into_iter()
        .find(|template| template.id == format!("builtin:{slug}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use restichub_proto::validate::{MAX_SCRIPT_BODY_BYTES, is_script_env_name};

    #[test]
    fn fr_hk_12_every_builtin_template_obeys_the_rules_of_a_script() {
        let mut slugs = std::collections::BTreeSet::new();
        for template in script_templates() {
            assert!(slugs.insert(template.id.clone()), "{}", template.id);
            assert!(template.builtin);
            assert!(!template.body.trim().is_empty());
            assert!(template.body.len() <= MAX_SCRIPT_BODY_BYTES);
            let mut names = std::collections::BTreeSet::new();
            for parameter in &template.parameters {
                assert!(is_script_env_name(&parameter.name), "{}", parameter.name);
                assert!(names.insert(parameter.name.clone()));
                if parameter.kind == TemplateParameterKind::SecretRef {
                    assert!(parameter.default_value.is_none());
                }
            }
            // A template never holds a secret value: the body names the variable only.
            for parameter in template
                .parameters
                .iter()
                .filter(|parameter| parameter.kind == TemplateParameterKind::SecretRef)
            {
                assert!(!template.body.contains(&format!("={}", parameter.name)));
            }
        }
        assert!(script_templates().len() >= 10);
    }

    #[test]
    fn fr_hk_12_the_job_templates_build_from_the_script_templates() {
        for template in job_templates() {
            assert!(!template.scripts.is_empty(), "{}", template.id);
        }
        let postgres = job_template("postgres-linux").expect("a built-in job template");
        assert_eq!(postgres.scripts.len(), 2);
        assert!(
            postgres.scripts[1]
                .env
                .iter()
                .any(|variable| variable.name == "PGPASSWORD" && variable.value.is_empty())
        );
    }
}
