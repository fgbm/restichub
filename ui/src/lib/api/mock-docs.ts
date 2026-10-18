/*
 * The demo installation of the documentation screenshots. The spec `e2e/docs-screens.spec.ts`
 * sets the storage key and a fixed clock, so each frame shows the same data on every run.
 *
 * Only `example.com` names and RFC 5737 addresses appear here: the frames are public.
 */
import type {
	Agent,
	Alert,
	AuditRecord,
	Job,
	Plan,
	Profile,
	Repository,
	Tenant,
	TenantAccess
} from './types';

export const DOCS_KEY = 'restichub.mock.docs';

export function docsMode(): boolean {
	return typeof localStorage !== 'undefined' && localStorage.getItem(DOCS_KEY) === 'docs';
}

/** The frame of the first administrator needs an installation before its setup. */
export function docsSetupMode(): boolean {
	return typeof localStorage !== 'undefined' && localStorage.getItem(DOCS_KEY) === 'setup';
}

const MINUTE = 60_000;
const DAY = 24 * 60;

function iso(minutesAgo: number): string {
	return new Date(Date.now() - minutesAgo * MINUTE).toISOString();
}

export function docsMemberships(): TenantAccess[] {
	return [{ id: 't_01', name: 'example', role: 'admin' }];
}

export function docsTenants(): Tenant[] {
	return [
		{
			id: 't_01',
			name: 'example',
			status: 'active',
			created_at: iso(DAY * 60),
			user_count: 3,
			agent_count: 3,
			admin_state: 'present'
		}
	];
}

function agent(id: string, hostname: string, os: string, seenMinutes: number): Agent {
	const linux = os === 'linux';
	return {
		id,
		hostname,
		os,
		arch: 'amd64',
		agent_version: '0.1.0',
		restic_version: '0.19.1',
		labels: [],
		status: 'active',
		restore_roots: linux ? ['/var/lib/restichub-restore'] : ['C:\\ProgramData\\ResticHub-Restore'],
		online: true,
		last_seen_at: iso(seenMinutes),
		first_connected_at: iso(DAY * 21 + Number(id) * 37),
		possible_duplicate_of: null,
		created_at: iso(DAY * 21 + Number(id) * 41),
		certificate_expires_at: iso(-(DAY * 9 + Number(id) * 113)),
		last_certificate_rotation_at: iso(DAY * 21 - Number(id) * 7),
		superseded_certificate_used_at: null,
		certificate_serial: '3a' + id.padStart(14, '0'),
		protocol_version: '1.1',
		control_plane_version: '0.1.0',
		version_status: 'current',
		revoked_at: null,
		server_scripts: hostname === 'db-01' ? 'enabled' : 'disabled',
		script_interpreters: hostname === 'db-01' ? ['bash', 'sh'] : []
	};
}

export function docsAgents(): Agent[] {
	return [
		agent('1', 'fs-01', 'windows', 0.4),
		agent('2', 'db-01', 'linux', 0.3),
		agent('3', 'web-01', 'linux', 0.6)
	];
}

export function docsRepositories(): Repository[] {
	const repository = (
		id: string,
		name: string,
		backend: Repository['backend'],
		snapshots: number,
		checkedMinutesAgo: number
	): Repository => ({
		id,
		name,
		backend,
		credentials: {
			password: true,
			s3_access_key: backend.kind === 's3',
			s3_secret_key: backend.kind === 's3',
			rest_password: false
		},
		initialized: true,
		maintenance_agent_id: '2',
		created_at: iso(DAY * 20),
		updated_at: iso(DAY * 4 + 73),
		created_by: 'u_01',
		lock_state: 'free',
		last_check: { finished_at: iso(checkedMinutesAgo), result: 'passed' },
		snapshot_count: snapshots,
		shares_location_with: []
	});
	return [
		repository(
			'r_1',
			's3-main',
			{ kind: 's3', endpoint: 's3.example.com', bucket: 'backups', prefix: 'example' },
			214,
			DAY + 312
		),
		repository('r_2', 'local-nas', { kind: 'local', path: '/srv/restic/nas' }, 58, DAY * 3 + 95)
	];
}

const OPTIONS = {
	use_fs_snapshot: false,
	one_file_system: true,
	exclude_caches: true,
	limit_upload_kib: 0,
	limit_download_kib: 0,
	stall_timeout_minutes: 60,
	max_duration_minutes: 2880
};

const RETENTION = {
	keep_last: 0,
	keep_hourly: 0,
	keep_daily: 7,
	keep_weekly: 4,
	keep_monthly: 12,
	keep_yearly: 0,
	keep_within: '',
	keep_tags: [],
	group_by: [],
	filter: { hosts: [], tags: [], paths: [] }
};

export function docsProfiles(): Profile[] {
	const profile = (
		id: string,
		name: string,
		paths: Profile['paths'],
		excludes: string[],
		tags: string[],
		options = OPTIONS
	): Profile => ({
		id,
		name,
		paths,
		excludes,
		exclude_files: [],
		tags,
		options,
		scripts: [],
		parameters: [],
		retention: RETENTION,
		check: { read_data_subset: '5%' },
		created_at: iso(DAY * 19),
		updated_at: iso(DAY * 2 + 214),
		created_by: 'u_01'
	});
	return [
		profile(
			'p_1',
			'file-shares',
			{ all: [], windows: ['C:\\Data', 'D:\\Shares'], linux: [], macos: [] },
			['*.tmp', '~$*'],
			['files'],
			{ ...OPTIONS, use_fs_snapshot: true, one_file_system: false }
		),
		profile(
			'p_2',
			'postgres-dumps',
			{ all: ['/var/backups/postgres'], windows: [], linux: ['/etc/postgresql'], macos: [] },
			[],
			['database']
		),
		profile(
			'p_3',
			'web-content',
			{ all: ['/srv/www'], windows: [], linux: ['/etc/nginx'], macos: [] },
			['cache/', '*.log'],
			['web']
		)
	];
}

export function docsPlans(nextFireTime: (cron: string, timeZone: string) => string): Plan[] {
	const plan = (
		id: string,
		name: string,
		profile: [string, string],
		repository: string,
		host: [string, string],
		cron: string,
		lastSuccessMinutesAgo: number
	): Plan => ({
		id,
		name,
		profile_id: profile[0],
		profile_name: profile[1],
		repository_id: repository,
		parameter_values: [],
		agent_id: host[0],
		agent_hostname: host[1],
		cron,
		time_zone: 'Europe/Amsterdam',
		missed_run_policy: 'latest_only',
		enabled: true,
		rpo_seconds: null,
		effective_rpo_seconds: 26 * 3600,
		next_run_at: nextFireTime(cron, 'Europe/Amsterdam'),
		last_fire_time: iso(lastSuccessMinutesAgo + 17),
		last_success_at: iso(lastSuccessMinutesAgo),
		rpo_state: 'ok',
		created_at: iso(DAY * 18),
		updated_at: iso(DAY * 2 + 31)
	});
	return [
		plan(
			'pl_1',
			'file-shares-nightly',
			['p_1', 'file-shares'],
			'r_1',
			['1', 'fs-01'],
			'0 1 * * *',
			431
		),
		plan(
			'pl_2',
			'postgres-hourly',
			['p_2', 'postgres-dumps'],
			'r_1',
			['2', 'db-01'],
			'15 * * * *',
			23
		),
		plan(
			'pl_3',
			'web-content-daily',
			['p_3', 'web-content'],
			'r_2',
			['3', 'web-01'],
			'30 3 * * *',
			318
		)
	];
}

const FAILED_JOB = '0199e120-74e9-7a3d-b16c-e8f2d4a5c906';

type DocsJob = { job: Job; readyAt: number; repositoryId: string; initializes: boolean };

export function docsJobs(): DocsJob[] {
	const plans: Record<string, [string, string, string, string, string]> = {
		pl_1: ['file-shares-nightly', '1', 'fs-01', 'r_1', 's3-main'],
		pl_2: ['postgres-hourly', '2', 'db-01', 'r_1', 's3-main'],
		pl_3: ['web-content-daily', '3', 'web-01', 'r_2', 'local-nas']
	};
	const job = (
		id: string,
		plan: string,
		status: Job['status'],
		resultCode: Job['result_code'],
		message: string | null,
		startedMinutesAgo: number | null,
		durationMinutes: number | null
	): DocsJob => {
		const [planName, agentId, hostname, repositoryId, repositoryName] = plans[plan];
		const created = (startedMinutesAgo ?? 1.3) + 0.4;
		const finished =
			startedMinutesAgo === null || durationMinutes === null
				? null
				: startedMinutesAgo - durationMinutes;
		return {
			job: {
				id,
				kind: 'backup',
				repository_id: repositoryId,
				repository_name: repositoryName,
				agent_id: agentId,
				agent_hostname: hostname,
				plan_id: plan,
				plan_name: planName,
				status,
				result_code: resultCode,
				result_message: message,
				created_at: iso(created),
				updated_at: iso(finished ?? startedMinutesAgo ?? created),
				finished_at: finished === null ? null : iso(finished),
				started_at: startedMinutesAgo === null ? null : iso(startedMinutesAgo),
				expires_at: null,
				run_at: iso(created),
				attempt: 0
			},
			// A demo job never changes on its own.
			readyAt: Number.MAX_SAFE_INTEGER,
			repositoryId,
			initializes: false
		};
	};
	return [
		job('0199e1c4-9a37-7d10-b3c2-6f1e2a9d4b01', 'pl_2', 'running', null, null, 2.6, null),
		job('0199e18e-31f2-7b44-8e19-2c7d5a3f9b02', 'pl_2', 'succeeded', 'succeeded', null, 62.4, 1.9),
		job('0199e157-d0a8-7c6b-a4f3-91be6c2d7e03', 'pl_2', 'succeeded', 'succeeded', null, 122.1, 2.3),
		job(
			'0199e0a9-6e14-7f28-9b57-d3a1c8e4f604',
			'pl_3',
			'succeeded',
			'succeeded_with_warnings',
			null,
			331.7,
			6.4
		),
		job('0199e035-2b7c-7e91-85d6-4a0f9c3b1e05', 'pl_1', 'succeeded', 'succeeded', null, 469.5, 3.2),
		job(FAILED_JOB, 'pl_2', 'failed', 'failed', 'step_failed', 182.8, 0.7),
		job('0199e0e8-c35b-7d82-9f4a-1b6e7c2d8a07', 'pl_2', 'succeeded', 'succeeded', null, 242.3, 2.1),
		job(
			'0199dad1-58f6-7b19-a2e7-6c4d3f9b0e08',
			'pl_1',
			'succeeded',
			'succeeded',
			null,
			DAY + 471.2,
			3.7
		),
		job(
			'0199db5e-e9a3-7c57-8d1b-f2a6e4c7b309',
			'pl_3',
			'succeeded',
			'succeeded',
			null,
			DAY + 329.9,
			5.8
		)
	];
}

export function docsAlerts(): Alert[] {
	return [
		{
			id: 'al_d2',
			kind: 'job_failed',
			status: 'open',
			agent_id: '2',
			agent_hostname: 'db-01',
			plan_id: 'pl_2',
			plan_name: 'postgres-hourly',
			repository_id: 'r_1',
			repository_name: 's3-main',
			job_id: FAILED_JOB,
			params: { job_kind: 'backup', error_code: 'failed' },
			error_text: 'step_failed',
			url: `https://restichub.example.com/jobs/${FAILED_JOB}`,
			opened_at: iso(182.1),
			resolved_at: null
		},
		{
			id: 'al_d1',
			kind: 'agent_offline',
			status: 'resolved',
			agent_id: '3',
			agent_hostname: 'web-01',
			plan_id: null,
			plan_name: null,
			repository_id: null,
			repository_name: null,
			job_id: null,
			params: { elapsed_seconds: 47 * 60, threshold_minutes: 30 },
			error_text: null,
			url: null,
			opened_at: iso(DAY * 2 + 412),
			resolved_at: iso(DAY * 2 + 395)
		}
	];
}

export function docsAudit(): AuditRecord[] {
	const record = (
		id: string,
		minutesAgo: number,
		action: string,
		object: [string, string, string],
		diff: AuditRecord['diff'] = null
	): AuditRecord => ({
		id,
		tenant_id: 't_01',
		occurred_at: iso(minutesAgo),
		actor_type: 'user',
		actor_id: 'u_01',
		source_ip: '192.0.2.24',
		action,
		object_type: object[0],
		object_id: object[1],
		object_name: object[2],
		actor_name: 'Anna Petrova',
		diff
	});
	const run = (id: string, minutesAgo: number, plan: string): AuditRecord => ({
		...record(id, minutesAgo, 'job_run', ['job', id, plan]),
		actor_type: 'system',
		actor_id: null,
		source_ip: null,
		actor_name: null
	});
	return [
		run('a_d10', 2.9, 'postgres-hourly'),
		run('a_d09', 62.7, 'postgres-hourly'),
		record('a_d08', 97.4, 'config_update', ['plan', 'pl_1', 'file-shares-nightly'], {
			cron: { old: '0 2 * * *', new: '0 1 * * *' }
		}),
		run('a_d07', 122.4, 'postgres-hourly'),
		run('a_d06', 183.1, 'postgres-hourly'),
		run('a_d05', 332.0, 'web-content-daily'),
		run('a_d04', 469.8, 'file-shares-nightly'),
		record('a_d03', DAY + 214.6, 'agent_enroll', ['agent', '3', 'web-01']),
		record('a_d02', DAY * 2 + 31.2, 'repository_create', ['repository', 'r_2', 'local-nas']),
		record('a_d01', DAY * 2 + 58.9, 'user_create', ['user', 'u_02', 'Boris Ivanov'], {
			old: null,
			new: { username: 'operator', display_name: 'Boris Ivanov', role: 'operator' }
		})
	];
}
