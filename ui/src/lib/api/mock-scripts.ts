/*
 * Script templates, job templates and secrets of the mock (FR-HK-12, ADR-0037).
 *
 * The built-in templates agree in name and in shape with the ones the control plane ships. The mock
 * keeps a smaller set of them, with shorter scripts.
 */
import type {
	CreateJobTemplateRequest,
	CreateScriptTemplateRequest,
	CreateSecretRequest,
	JobTemplate,
	ScriptDef,
	ScriptTemplate,
	Secret,
	UpdateJobTemplateRequest,
	UpdateScriptTemplateRequest
} from './types';

function problem(code: string, field: string, reason: string): Error {
	return Object.assign(new Error(`${field}: ${reason}`), {
		problem: {
			type: 'about:blank',
			title: 'Bad Request',
			status: 400,
			code,
			request_id: '',
			field,
			reason
		}
	});
}

const PG_BASH = `set -euo pipefail
mkdir -p "$DUMP_DIR"
target="$DUMP_DIR/$PGDATABASE.dump"
pg_dump --format=custom --file="$target" "$PGDATABASE"
echo "dump written: $target"
`;

const PG_POWERSHELL = `$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $env:DUMP_DIR | Out-Null
$target = Join-Path $env:DUMP_DIR "$($env:PGDATABASE).dump"
& pg_dump.exe --format=custom --file=$target $env:PGDATABASE
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Output "dump written: $target"
`;

const STOP_BASH = `set -euo pipefail
systemctl stop "$SERVICE_NAME"
echo "stopped: $SERVICE_NAME"
`;

const START_BASH = `set -euo pipefail
systemctl start "$SERVICE_NAME"
echo "started: $SERVICE_NAME"
`;

const DISK_BASH = `set -euo pipefail
free_kb=$(df --output=avail -k "$CHECK_PATH" | tail -n 1 | tr -d ' ')
need_kb=$((MIN_FREE_GB * 1024 * 1024))
echo "free: $((free_kb / 1024 / 1024)) GB, needed: $MIN_FREE_GB GB"
if [ "$free_kb" -lt "$need_kb" ]; then
  echo "not enough free space on $CHECK_PATH" >&2
  exit 1
fi
`;

const MSSQL = `$ErrorActionPreference = 'Stop'
$file = Join-Path $env:BACKUP_DIR "$($env:MSSQL_DATABASE).bak"
$query = "BACKUP DATABASE [$($env:MSSQL_DATABASE)] TO DISK = N'$file' WITH INIT, COMPRESSION, CHECKSUM"
& sqlcmd.exe -b -S $env:SQLCMDSERVER -Q $query
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
`;

function builtin(
	slug: string,
	name: string,
	description: string,
	interpreter: ScriptTemplate['interpreter'],
	phase: ScriptTemplate['phase'],
	body: string,
	parameters: ScriptTemplate['parameters'],
	failurePolicy: ScriptTemplate['failure_policy'] = 'abort_backup'
): ScriptTemplate {
	return {
		id: `builtin:${slug}`,
		builtin: true,
		name,
		description,
		interpreter,
		phase,
		body,
		timeout_seconds: 3600,
		failure_policy: failurePolicy,
		parameters
	};
}

export const BUILTIN_SCRIPT_TEMPLATES: ScriptTemplate[] = [
	builtin(
		'postgres-dump-bash',
		'PostgreSQL dump with pg_dump (Linux)',
		'Writes a custom-format dump of one database before the backup. Needs pg_dump on the host.',
		'bash',
		'before',
		PG_BASH,
		[
			{ name: 'PGHOST', label: 'Database host', type: 'string', default: 'localhost' },
			{ name: 'PGPORT', label: 'Port', type: 'integer', default: '5432' },
			{ name: 'PGUSER', label: 'User', type: 'string', default: 'postgres' },
			{ name: 'PGPASSWORD', label: 'Password', type: 'secret_ref' },
			{ name: 'PGDATABASE', label: 'Database', type: 'string' },
			{
				name: 'DUMP_DIR',
				label: 'Dump directory',
				type: 'string',
				default: '/var/backups/postgres'
			}
		]
	),
	builtin(
		'postgres-dump-powershell',
		'PostgreSQL dump with pg_dump (Windows)',
		'Writes a custom-format dump of one database before the backup. Needs pg_dump.exe in the PATH of the agent.',
		'powershell',
		'before',
		PG_POWERSHELL,
		[
			{ name: 'PGHOST', label: 'Database host', type: 'string', default: 'localhost' },
			{ name: 'PGUSER', label: 'User', type: 'string', default: 'postgres' },
			{ name: 'PGPASSWORD', label: 'Password', type: 'secret_ref' },
			{ name: 'PGDATABASE', label: 'Database', type: 'string' },
			{
				name: 'DUMP_DIR',
				label: 'Dump directory',
				type: 'string',
				default: 'C:\\Backups\\postgres'
			}
		]
	),
	builtin(
		'mssql-backup-powershell',
		'MSSQL backup with sqlcmd (Windows)',
		'Runs BACKUP DATABASE to a .bak file before the file backup. Needs sqlcmd.exe in the PATH of the agent.',
		'powershell',
		'before',
		MSSQL,
		[
			{ name: 'SQLCMDSERVER', label: 'Server', type: 'string', default: 'localhost' },
			{ name: 'SQLCMDUSER', label: 'User', type: 'string', default: 'sa' },
			{ name: 'SQLCMDPASSWORD', label: 'Password', type: 'secret_ref' },
			{ name: 'MSSQL_DATABASE', label: 'Database', type: 'string' },
			{
				name: 'BACKUP_DIR',
				label: 'Backup directory',
				type: 'string',
				default: 'C:\\Backups\\mssql'
			}
		]
	),
	builtin(
		'service-stop-bash',
		'Stop a service (Linux)',
		'Stops a systemd service before the backup. Pair it with the start template in the finally phase.',
		'bash',
		'before',
		STOP_BASH,
		[{ name: 'SERVICE_NAME', label: 'Service', type: 'string' }]
	),
	builtin(
		'service-start-bash',
		'Start a service (Linux)',
		'Starts a systemd service. Use it in the finally phase, so the service starts again when the backup fails.',
		'bash',
		'finally',
		START_BASH,
		[{ name: 'SERVICE_NAME', label: 'Service', type: 'string' }],
		'continue'
	),
	builtin(
		'disk-space-check-bash',
		'Free disk space check (Linux)',
		'Fails when the path has less free space than the limit.',
		'bash',
		'before',
		DISK_BASH,
		[
			{ name: 'CHECK_PATH', label: 'Path', type: 'string', default: '/' },
			{ name: 'MIN_FREE_GB', label: 'Minimum free space in GB', type: 'integer', default: '10' }
		]
	)
];

function scriptOf(slug: string, overrides: Record<string, string> = {}): ScriptDef | undefined {
	const template = BUILTIN_SCRIPT_TEMPLATES.find((item) => item.id === `builtin:${slug}`);
	if (!template) return undefined;
	return {
		name: template.name,
		interpreter: template.interpreter,
		phase: template.phase,
		body: template.body,
		timeout_seconds: template.timeout_seconds,
		failure_policy: template.failure_policy,
		env: template.parameters.map((parameter) => ({
			name: parameter.name,
			kind: parameter.type === 'secret_ref' ? 'secret_ref' : 'string',
			value:
				parameter.type === 'secret_ref'
					? ''
					: (overrides[parameter.name] ?? parameter.default ?? '')
		})),
		template_id: template.id
	};
}

function builtinJob(
	slug: string,
	name: string,
	description: string,
	scripts: [string, Record<string, string>?][]
): JobTemplate {
	return {
		id: `builtin:${slug}`,
		builtin: true,
		name,
		description,
		scripts: scripts
			.map(([template, overrides]) => scriptOf(template, overrides))
			.filter((script): script is ScriptDef => script !== undefined)
	};
}

export const BUILTIN_JOB_TEMPLATES: JobTemplate[] = [
	builtinJob(
		'postgres-linux',
		'PostgreSQL server (Linux)',
		'Checks the free space, then dumps one database before the backup.',
		[['disk-space-check-bash', { CHECK_PATH: '/var/backups' }], ['postgres-dump-bash']]
	),
	builtinJob(
		'stop-start-service-linux',
		'Application with a service (Linux)',
		'Stops a service before the backup and starts it again in the finally phase.',
		[['service-stop-bash'], ['service-start-bash']]
	)
];

const SAMPLE_KEY = 'restichub.mock.profiles';

function now(): string {
	return new Date().toISOString();
}

/** The stored templates and secrets of the mock. */
export class MockScripts {
	private templates: ScriptTemplate[] = [];
	private jobTemplates: JobTemplate[] = [];
	private secrets: Secret[] = [];
	private counter = 0;

	constructor() {
		const sample =
			typeof localStorage !== 'undefined' && localStorage.getItem(SAMPLE_KEY) === 'sample';
		if (sample) {
			this.secrets = [
				{
					id: 's_db',
					name: 'db.password',
					purpose: 'hook_secret',
					created_at: now(),
					updated_at: now(),
					created_by: 'u_01',
					used_by: [{ kind: 'profile', id: 'p_3', name: 'db-nightly' }]
				},
				{
					id: 's_repo',
					name: 'primary-s3.password',
					purpose: 'repository_password',
					created_at: now(),
					updated_at: now(),
					created_by: 'u_01',
					used_by: [{ kind: 'repository', id: 'r_1', name: 'primary-s3' }]
				},
				{
					id: 's_spare',
					name: 'spare.token',
					purpose: 'hook_secret',
					created_at: now(),
					updated_at: now(),
					created_by: 'u_01',
					used_by: []
				}
			];
			this.templates = [
				{
					id: 'st_1',
					builtin: false,
					name: 'Remove old dumps',
					description: 'Deletes dump files older than a number of days.',
					interpreter: 'bash',
					phase: 'finally',
					body: 'set -euo pipefail\nfind "$DUMP_DIR" -name "*.dump" -mtime +"$KEEP_DAYS" -print -delete\n',
					timeout_seconds: 300,
					failure_policy: 'continue',
					parameters: [
						{ name: 'DUMP_DIR', label: 'Dump directory', type: 'string', default: '/var/backups' },
						{ name: 'KEEP_DAYS', label: 'Days to keep', type: 'integer', default: '7' }
					],
					created_at: now(),
					updated_at: now()
				}
			];
		}
	}

	listSecrets(): Secret[] {
		return [...this.secrets];
	}

	createSecret(request: CreateSecretRequest): Secret {
		if (this.secrets.some((item) => item.name === request.name)) {
			throw problem('conflict', 'name', 'conflict');
		}
		this.counter += 1;
		const secret: Secret = {
			id: `s_${this.counter}`,
			name: request.name,
			purpose: request.purpose,
			created_at: now(),
			updated_at: now(),
			created_by: 'u_01',
			used_by: []
		};
		this.secrets = [...this.secrets, secret];
		return secret;
	}

	replaceSecretValue(id: string): void {
		const secret = this.secrets.find((item) => item.id === id);
		if (!secret) throw problem('not_found', 'id', 'unknown');
		this.secrets = this.secrets.map((item) =>
			item.id === id ? { ...item, updated_at: now() } : item
		);
	}

	deleteSecret(id: string): void {
		const secret = this.secrets.find((item) => item.id === id);
		if (!secret) throw problem('not_found', 'id', 'unknown');
		if (secret.used_by.length > 0) {
			throw Object.assign(new Error('conflict'), {
				problem: {
					type: 'about:blank',
					title: 'Conflict',
					status: 409,
					code: 'conflict',
					request_id: ''
				}
			});
		}
		this.secrets = this.secrets.filter((item) => item.id !== id);
	}

	listScriptTemplates(): ScriptTemplate[] {
		return [...BUILTIN_SCRIPT_TEMPLATES, ...this.templates];
	}

	createScriptTemplate(request: CreateScriptTemplateRequest): ScriptTemplate {
		if (!request.name.trim()) throw problem('invalid_argument', 'name', 'empty');
		if (!request.body.trim()) throw problem('invalid_argument', 'body', 'empty');
		if (this.templates.some((item) => item.name === request.name)) {
			throw problem('conflict', 'name', 'conflict');
		}
		this.counter += 1;
		const template: ScriptTemplate = {
			id: `st_${this.counter + 100}`,
			builtin: false,
			name: request.name.trim(),
			description: request.description ?? '',
			interpreter: request.interpreter,
			phase: request.phase,
			body: request.body,
			timeout_seconds: request.timeout_seconds ?? 0,
			failure_policy: request.failure_policy,
			parameters: request.parameters ?? [],
			created_at: now(),
			updated_at: now()
		};
		this.templates = [...this.templates, template];
		return template;
	}

	updateScriptTemplate(id: string, request: UpdateScriptTemplateRequest): ScriptTemplate {
		if (id.startsWith('builtin:')) throw problem('invalid_argument', 'id', 'reserved');
		const current = this.templates.find((item) => item.id === id);
		if (!current) throw new Error('not found');
		const updated: ScriptTemplate = {
			...current,
			name: request.name ?? current.name,
			description: request.description ?? current.description,
			interpreter: request.interpreter ?? current.interpreter,
			phase: request.phase ?? current.phase,
			body: request.body ?? current.body,
			timeout_seconds: request.timeout_seconds ?? current.timeout_seconds,
			failure_policy: request.failure_policy ?? current.failure_policy,
			parameters: request.parameters ?? current.parameters,
			updated_at: now()
		};
		this.templates = this.templates.map((item) => (item.id === id ? updated : item));
		return updated;
	}

	deleteScriptTemplate(id: string): void {
		if (id.startsWith('builtin:')) throw problem('invalid_argument', 'id', 'reserved');
		this.templates = this.templates.filter((item) => item.id !== id);
	}

	listJobTemplates(): JobTemplate[] {
		return [...BUILTIN_JOB_TEMPLATES, ...this.jobTemplates];
	}

	createJobTemplate(request: CreateJobTemplateRequest): JobTemplate {
		if (!request.name.trim()) throw problem('invalid_argument', 'name', 'empty');
		if (this.jobTemplates.some((item) => item.name === request.name)) {
			throw problem('conflict', 'name', 'conflict');
		}
		this.counter += 1;
		const template: JobTemplate = {
			id: `jt_${this.counter + 100}`,
			builtin: false,
			name: request.name.trim(),
			description: request.description ?? '',
			scripts: request.scripts ?? [],
			created_at: now(),
			updated_at: now()
		};
		this.jobTemplates = [...this.jobTemplates, template];
		return template;
	}

	updateJobTemplate(id: string, request: UpdateJobTemplateRequest): JobTemplate {
		if (id.startsWith('builtin:')) throw problem('invalid_argument', 'id', 'reserved');
		const current = this.jobTemplates.find((item) => item.id === id);
		if (!current) throw new Error('not found');
		const updated: JobTemplate = {
			...current,
			name: request.name ?? current.name,
			description: request.description ?? current.description,
			scripts: request.scripts ?? current.scripts,
			updated_at: now()
		};
		this.jobTemplates = this.jobTemplates.map((item) => (item.id === id ? updated : item));
		return updated;
	}

	deleteJobTemplate(id: string): void {
		if (id.startsWith('builtin:')) throw problem('invalid_argument', 'id', 'reserved');
		this.jobTemplates = this.jobTemplates.filter((item) => item.id !== id);
	}
}
