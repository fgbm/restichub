/*
 * In-memory mock of the API service. Development and tests use it only.
 *
 * The module holds the same functions as the real service, so a screen does not
 * know which one it uses. The tenant functions are the planned API of issue #20.
 */
import { filterSnapshots, inRange, listDirectory, makeSnapshots } from './mock-restore';
import type {
	Agent,
	ApiToken,
	ApiTokenCreated,
	AuditPage,
	AuditQuery,
	AuditRecord,
	ChangePasswordRequest,
	CreateApiTokenRequest,
	CreateEnrollmentTokenRequest,
	CreatePlatformUserRequest,
	CreatePlanRequest,
	CreateProfileRequest,
	CreateRepositoryRequest,
	CreateTenantRequest,
	CreateUserRequest,
	EnrollmentToken,
	EnrollmentTokenCreated,
	Instance,
	Job,
	JobActionRequest,
	JobDetail,
	JobListResponse,
	JobLogPage,
	JobLogQuery,
	JobQuery,
	JobKind,
	JobStatus,
	LsQueryRequest,
	LsView,
	Maintenance,
	MaintenanceTask,
	Me,
	MembershipInput,
	NextRunsResponse,
	Plan,
	PlanResponse,
	PlatformUser,
	PreviewResponse,
	PreviewScheduleRequest,
	Profile,
	ProfileResponse,
	Repository,
	ResetPasswordRequest,
	RestoreRequest,
	SchedulePreviewResponse,
	SetRoleRequest,
	SetupRequest,
	SetupStatus,
	SignInRequest,
	SnapshotsParams,
	SnapshotsQuery,
	SnapshotsView,
	Tenant,
	TenantAccess,
	UpdateAccountRequest,
	UpdatePlanRequest,
	UpdateMaintenanceRequest,
	UpdateProfileRequest,
	UpdateRepositoryRequest,
	UpdateUserRequest,
	User
} from './types';

import { nextRunsFromCron } from '#lib/schedule.ts';
import { detailOf, logPageOf } from './mock-jobs';

const REDACTED = '<redacted>';

function iso(minutesAgo: number): string {
	return new Date(Date.now() - minutesAgo * 60_000).toISOString();
}

/// The next fire time of a plan, from its cron and time zone (FR-SC-01).
function nextFireTime(cron: string, timeZone: string): string {
	return nextRunsFromCron(cron, timeZone, 1)[0] ?? new Date(Date.now() + 60 * 60_000).toISOString();
}

/// The paths of a profile for one operating system: `all` plus the paths of that system.
function pathsForOs(profile: Profile, os: string): string[] {
	const paths = profile.paths ?? { all: [], windows: [], linux: [], macos: [] };
	const extra =
		os === 'windows'
			? (paths.windows ?? [])
			: os === 'linux'
				? (paths.linux ?? [])
				: (paths.macos ?? []);
	return [...(paths.all ?? []), ...extra];
}

/// The save-time warnings of the mock (FR-PF-05). The VSS flag is a warning for a non-Windows
/// target, not a refusal.
function warningsFor(profile: Profile): { field: string; reason: string; os?: string }[] {
	const warnings: { field: string; reason: string; os?: string }[] = [];
	if (profile.options?.use_fs_snapshot) {
		warnings.push({ field: 'options.use_fs_snapshot', reason: 'unsupported', os: 'linux' });
		warnings.push({ field: 'options.use_fs_snapshot', reason: 'unsupported', os: 'macos' });
	}
	return warnings;
}

/// A problem error that the UI reads as an API problem, the way the HTTP client does.
function problemError(code: string, field: string, reason: string): Error {
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

// The mock starts without an agent. A review capture sets this storage key to see a filled list.
const SAMPLE_AGENTS_KEY = 'restichub.mock.agents';

// The mock starts without a profile. A review capture sets this storage key to see a filled list.
const SAMPLE_PROFILES_KEY = 'restichub.mock.profiles';

// The mock starts without a repository. A review capture or an e2e test sets this storage key to
// see initialized repositories on the Restore screen.
const SAMPLE_REPOSITORIES_KEY = 'restichub.mock.repositories';

// The mock starts without a plan. A review capture or an e2e test sets this storage key to see a
// filled list.
const SAMPLE_PLANS_KEY = 'restichub.mock.plans';
// A review capture or an e2e test sets this storage key to drive the mock as a Platform Admin
// without a membership. The sign-in then survives a full page load, so a direct visit to an
// address keeps the session, the way a session cookie does.
const SESSION_KEY = 'restichub.mock.session';
const PLATFORM_ADMIN_NO_ORG = 'platform-admin-no-org';
const SESSION_SIGNED_IN_KEY = 'restichub.mock.session.signed-in';

function platformAdminNoOrg(): boolean {
	return (
		typeof localStorage !== 'undefined' &&
		localStorage.getItem(SESSION_KEY) === PLATFORM_ADMIN_NO_ORG
	);
}

function makeAgents(): Agent[] {
	if (typeof localStorage === 'undefined' || localStorage.getItem(SAMPLE_AGENTS_KEY) !== 'sample') {
		return [];
	}
	const agent = (
		id: string,
		hostname: string,
		os: string,
		arch: string,
		version: string,
		seen: number,
		status: Agent['status'] = 'active'
	): Agent => ({
		id,
		hostname,
		os,
		arch,
		agent_version: version,
		restic_version: '0.19.0',
		labels: [],
		status,
		last_seen_at: iso(seen),
		created_at: iso(60 * 24 * 30),
		certificate_expires_at: iso(-60 * 24 * 60),
		certificate_serial: '0f' + id.padStart(14, '0'),
		revoked_at: status === 'revoked' ? iso(60) : null
	});
	return [
		agent('1', 'win-fs01', 'windows', 'amd64', '0.4.2', 14),
		agent('2', 'srv-db-01', 'linux', 'amd64', '0.4.2', 26),
		agent('3', 'win-dc01', 'windows', 'amd64', '0.4.1', 60),
		agent('4', 'nas-backup', 'linux', 'arm64', '0.4.2', 120),
		agent('5', 'mac-build-01', 'darwin', 'arm64', '0.4.0', 60 * 48),
		agent('6', 'srv-app-02', 'linux', 'amd64', '0.3.9', 180, 'revoked')
	];
}

function makeRepositories(): Repository[] {
	if (
		typeof localStorage === 'undefined' ||
		localStorage.getItem(SAMPLE_REPOSITORIES_KEY) !== 'sample'
	) {
		return [];
	}
	const repository = (
		id: string,
		name: string,
		backend: Repository['backend'],
		agent: string | null
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
		maintenance_agent_id: agent,
		created_at: iso(60 * 24 * 40),
		updated_at: iso(60 * 24 * 2),
		created_by: 'u_01'
	});
	return [
		repository(
			'r_1',
			'offsite-s3',
			{ kind: 's3', endpoint: 's3.eu.example.net', bucket: 'restichub-offsite', prefix: 'nas' },
			'4'
		),
		repository('r_2', 'nas-local', { kind: 'local', path: '/srv/restic/nas' }, '4')
	];
}

function makeProfiles(): Profile[] {
	if (
		typeof localStorage === 'undefined' ||
		localStorage.getItem(SAMPLE_PROFILES_KEY) !== 'sample'
	) {
		return [];
	}
	const retention = {
		keep_last: 0,
		keep_hourly: 0,
		keep_daily: 7,
		keep_weekly: 0,
		keep_monthly: 0,
		keep_yearly: 2,
		keep_within: '',
		keep_tags: [],
		group_by: [],
		filter: { hosts: ['web-*'], tags: [], paths: [] }
	};
	return [
		{
			id: 'p_1',
			name: 'web-daily',
			repository_id: 'r_1',
			paths: { all: ['/srv'], windows: [], linux: ['/etc'], macos: [] },
			excludes: ['*.tmp', 'node_modules'],
			exclude_files: [],
			tags: ['daily', 'web'],
			options: {
				use_fs_snapshot: false,
				one_file_system: true,
				exclude_caches: true,
				limit_upload_kib: 0,
				limit_download_kib: 0
			},
			hooks: [],
			retention,
			check: { read_data_subset: '10%' },
			created_at: iso(60 * 24 * 10),
			updated_at: iso(60),
			created_by: 'u_01'
		},
		{
			id: 'p_2',
			name: 'win-files',
			repository_id: 'r_1',
			paths: { all: [], windows: ['C:\\Data'], linux: [], macos: [] },
			excludes: [],
			exclude_files: [],
			tags: ['windows'],
			options: {
				use_fs_snapshot: true,
				one_file_system: false,
				exclude_caches: false,
				limit_upload_kib: 2048,
				limit_download_kib: 0
			},
			hooks: [],
			retention: null,
			check: null,
			created_at: iso(60 * 24 * 5),
			updated_at: iso(60 * 24),
			created_by: 'u_01'
		}
	];
}

/// One sample plan that binds the first profile to the first agent (FR-SC-01). A review capture or
/// an e2e test sets the storage key to see it.
function makePlans(): Plan[] {
	if (typeof localStorage === 'undefined' || localStorage.getItem(SAMPLE_PLANS_KEY) !== 'sample') {
		return [];
	}
	return [
		{
			id: 'pl_1',
			name: 'nightly-web',
			profile_id: 'p_1',
			profile_name: 'web-daily',
			repository_id: 'r_1',
			agent_id: '1',
			agent_hostname: 'win-fs01',
			cron: '0 2 * * *',
			time_zone: 'Europe/Berlin',
			missed_run_policy: 'latest_only',
			enabled: true,
			next_run_at: nextFireTime('0 2 * * *', 'Europe/Berlin'),
			last_fire_time: iso(60 * 24),
			created_at: iso(60 * 24 * 9),
			updated_at: iso(60 * 24)
		}
	];
}

/// Sample jobs of the sample plan, one for each lifecycle state that the Jobs screen shows
/// (FR-SC-09). The sample plan storage key shows them too.
function makeJobs(): MockJob[] {
	if (typeof localStorage === 'undefined' || localStorage.getItem(SAMPLE_PLANS_KEY) !== 'sample') {
		return [];
	}
	const job = (
		id: string,
		status: JobStatus,
		resultCode: Job['result_code'],
		message: string | null,
		createdMinutesAgo: number,
		startedMinutesAgo: number | null,
		finishedMinutesAgo: number | null
	): MockJob => ({
		job: {
			id,
			kind: 'backup',
			repository_id: 'r_1',
			repository_name: 'primary-s3',
			agent_id: '1',
			agent_hostname: 'win-fs01',
			plan_id: 'pl_1',
			plan_name: 'nightly-web',
			status,
			result_code: resultCode,
			result_message: message,
			created_at: iso(createdMinutesAgo),
			updated_at: iso(finishedMinutesAgo ?? startedMinutesAgo ?? createdMinutesAgo),
			finished_at: finishedMinutesAgo === null ? null : iso(finishedMinutesAgo),
			started_at: startedMinutesAgo === null ? null : iso(startedMinutesAgo),
			expires_at: null,
			run_at: iso(createdMinutesAgo),
			attempt: 0
		},
		// A sample job never changes on its own.
		readyAt: Number.MAX_SAFE_INTEGER,
		repositoryId: 'r_1',
		initializes: false
	});
	return [
		job('j_s1', 'running', null, null, 12, 11, null),
		job('j_s2', 'queued', null, null, 3, null, null),
		job('j_s3', 'succeeded', 'succeeded', null, 60 * 24 + 40, 60 * 24 + 38, 60 * 24 + 20),
		job('j_s6', 'succeeded', 'succeeded_with_warnings', null, 60 * 30, 60 * 30 - 2, 60 * 30 - 20),
		job('j_s4', 'failed', 'config_error', 'compile_failed', 60 * 48 + 5, null, 60 * 48 + 5),
		job(
			'j_s5',
			'dead_letter',
			'failed',
			'lock_retries_exhausted',
			60 * 72 + 30,
			60 * 72,
			60 * 72 - 20
		)
	];
}

function makeUsers(): User[] {
	return [
		{
			id: 'u_01',
			username: 'admin',
			display_name: 'Anna Petrova',
			role: 'admin',
			disabled: false,
			shared: false
		},
		{
			id: 'u_02',
			username: 'operator',
			display_name: 'Boris Ivanov',
			role: 'operator',
			disabled: false,
			shared: false
		},
		{
			id: 'u_03',
			username: 'viewer',
			display_name: 'Vera Sokolova',
			role: 'viewer',
			disabled: true,
			shared: true
		}
	];
}

/** The memberships of the caller (ADR-0025). */
function makeMemberships(): TenantAccess[] {
	return [
		{ id: 't_01', name: 'acme', role: 'admin' },
		{ id: 't_02', name: 'globex', role: 'operator' },
		{ id: 't_03', name: 'initech', role: 'viewer' },
		{ id: 't_04', name: 'northwind', role: 'admin' },
		{ id: 't_05', name: 'umbrella', role: 'viewer' },
		{ id: 't_06', name: 'brightline', role: 'operator' },
		{ id: 't_07', name: 'redwood', role: 'admin' },
		{ id: 't_08', name: 'summit', role: 'viewer' }
	];
}

function makeTenants(): Tenant[] {
	return [
		{
			id: 't_01',
			name: 'acme',
			status: 'active',
			created_at: iso(60 * 24 * 90),
			user_count: 4,
			admin_state: 'present'
		},
		{
			id: 't_02',
			name: 'globex',
			status: 'active',
			created_at: iso(60 * 24 * 60),
			user_count: 2,
			admin_state: 'present'
		},
		{
			id: 't_03',
			name: 'initech',
			status: 'suspended',
			created_at: iso(60 * 24 * 30),
			user_count: 0,
			admin_state: 'none'
		}
	];
}

/** The installation accounts with their memberships (ADR-0028). */
function makePlatformUsers(): PlatformUser[] {
	return [
		{
			id: 'pu_01',
			username: 'root',
			display_name: 'Platform Admin',
			email: null,
			email_verified: false,
			disabled: false,
			platform_admin: true,
			memberships: [{ tenant_id: 't_01', tenant_name: 'acme', role: 'admin', disabled: false }]
		},
		{
			id: 'pu_02',
			username: 'anna',
			display_name: 'Anna Petrova',
			email: 'anna@example.com',
			email_verified: false,
			disabled: false,
			platform_admin: false,
			memberships: [
				{ tenant_id: 't_01', tenant_name: 'acme', role: 'admin', disabled: false },
				{ tenant_id: 't_02', tenant_name: 'globex', role: 'viewer', disabled: false }
			]
		},
		{
			id: 'pu_03',
			username: 'boris',
			display_name: 'Boris Ivanov',
			email: null,
			email_verified: false,
			disabled: true,
			platform_admin: false,
			memberships: []
		}
	];
}

function makeTokens(): ApiToken[] {
	return [
		{
			id: 'k_01',
			name: 'ansible',
			role: 'admin',
			display_prefix: 'ab12cd34',
			created_at: iso(60 * 24 * 15),
			expires_at: null,
			last_used_at: iso(120),
			revoked_at: null
		},
		{
			id: 'k_02',
			name: 'monitoring',
			role: 'viewer',
			display_prefix: 'ef56gh78',
			created_at: iso(60 * 24 * 7),
			expires_at: iso(-60 * 24 * 90),
			last_used_at: null,
			revoked_at: null
		}
	];
}

function makeAudit(): AuditRecord[] {
	return [
		{
			id: 'a_01',
			tenant_id: 't_01',
			occurred_at: iso(3),
			actor_type: 'user',
			actor_id: 'u_01',
			source_ip: '10.0.0.5',
			action: 'user_update',
			object_type: 'user',
			object_id: 'u_03',
			diff: { disabled: { old: false, new: true } }
		},
		{
			id: 'a_02',
			tenant_id: 't_01',
			occurred_at: iso(40),
			actor_type: 'user',
			actor_id: 'u_01',
			source_ip: '10.0.0.5',
			action: 'api_token_create',
			object_type: 'api_token',
			object_id: 'k_02',
			diff: {
				old: null,
				new: { name: 'monitoring', role: 'viewer', display_prefix: 'ef56gh78' }
			}
		},
		{
			id: 'a_03',
			tenant_id: 't_01',
			occurred_at: iso(60 * 5),
			actor_type: 'api_token',
			actor_id: 'k_01',
			source_ip: '10.0.0.9',
			action: 'tenant_suspend',
			object_type: 'tenant',
			object_id: 't_03',
			diff: { status: { old: 'active', new: 'suspended' } }
		},
		{
			id: 'a_04',
			tenant_id: 't_01',
			occurred_at: iso(60 * 24),
			actor_type: 'user',
			actor_id: 'u_01',
			source_ip: '10.0.0.5',
			action: 'user_create',
			object_type: 'user',
			object_id: 'u_02',
			diff: {
				old: null,
				new: { username: 'operator', display_name: 'Boris Ivanov', role: 'operator' }
			}
		},
		{
			id: 'a_05',
			tenant_id: 't_01',
			occurred_at: iso(60 * 24 * 2),
			actor_type: 'system',
			actor_id: null,
			source_ip: '127.0.0.1',
			action: 'sign_in',
			object_type: 'user',
			object_id: 'u_01',
			diff: null
		},
		{
			id: 'a_06',
			tenant_id: 't_01',
			occurred_at: iso(60 * 24 * 3),
			actor_type: 'user',
			actor_id: 'u_01',
			source_ip: '10.0.0.5',
			action: 'user_update',
			object_type: 'user',
			object_id: 'u_01',
			diff: { password: { old: REDACTED, new: REDACTED } }
		}
	];
}

/** A mock job with the time when it finishes, so the polling dialog shows progress then a result. */
interface MockJob {
	job: Job;
	readyAt: number;
	repositoryId: string;
	initializes: boolean;
}

function isTerminal(status: JobStatus): boolean {
	return (
		status === 'succeeded' ||
		status === 'failed' ||
		status === 'expired' ||
		status === 'dead_letter'
	);
}

export class MockApiService {
	private users: User[] = makeUsers();
	private tenants: Tenant[] = makeTenants();
	private platformUsers: PlatformUser[] = makePlatformUsers();
	private tokens: ApiToken[] = makeTokens();
	private agents: Agent[] = makeAgents();
	private enrollmentTokens: EnrollmentToken[] = [];
	private repositories: Repository[] = makeRepositories();
	private jobs: MockJob[] = makeJobs();
	private maintenance = new Map<
		string,
		{
			time_zone: string;
			retention: { cron: string; enabled: boolean };
			check: { cron: string; enabled: boolean };
		}
	>();
	private profiles: Profile[] = makeProfiles();
	private plans: Plan[] = makePlans();
	private audit: AuditRecord[] = makeAudit();
	private memberships: TenantAccess[] = makeMemberships();
	private currentTenant = 't_01';
	// The mock stands for a ready instance, so the setup screen stays reachable at its own route.
	// A fresh page starts without a session, the way the real installation does. With the session
	// key set the sign-in state survives a page load, so a direct visit keeps the session.
	private setupDone = true;
	private signedIn =
		platformAdminNoOrg() &&
		typeof sessionStorage !== 'undefined' &&
		sessionStorage.getItem(SESSION_SIGNED_IN_KEY) === '1';

	private me(): Me {
		if (platformAdminNoOrg()) {
			// A Platform Admin without a membership: no current organization (ADR-0029).
			return { user: this.users[0], tenant: null, role: this.users[0].role, platform_admin: true };
		}
		const tenant = this.memberships.find((item) => item.id === this.currentTenant);
		return {
			user: this.users[0],
			tenant: tenant
				? { id: tenant.id, name: tenant.name }
				: { id: this.tenants[0].id, name: this.tenants[0].name },
			role: tenant?.role ?? this.users[0].role,
			// The mock stands for a Platform Admin, so the platform screens appear (ADR-0029).
			platform_admin: true
		};
	}

	async listMemberships(): Promise<TenantAccess[]> {
		if (platformAdminNoOrg()) return [];
		return [...this.memberships];
	}

	async switchTenant(tenantId: string): Promise<Me> {
		const tenant = this.memberships.find((item) => item.id === tenantId);
		if (!tenant) throw new Error('not found');
		this.currentTenant = tenantId;
		return this.me();
	}

	async signIn(_request: SignInRequest): Promise<Me> {
		void _request;
		this.signedIn = true;
		if (platformAdminNoOrg() && typeof sessionStorage !== 'undefined') {
			sessionStorage.setItem(SESSION_SIGNED_IN_KEY, '1');
		}
		return this.me();
	}

	async signOut(): Promise<void> {
		this.signedIn = false;
		if (typeof sessionStorage !== 'undefined') sessionStorage.removeItem(SESSION_SIGNED_IN_KEY);
		return undefined;
	}

	async currentUser(): Promise<Me> {
		// An anonymous caller gets a 401 from the control plane, and the session store
		// reads that as no session.
		if (!this.signedIn) throw new Error('no session');
		return this.me();
	}

	async changePassword(_request: ChangePasswordRequest): Promise<void> {
		void _request;
		return undefined;
	}

	async setupStatus(): Promise<SetupStatus> {
		return { setup_required: !this.setupDone, tenancy_mode: 'multi' };
	}

	async instance(): Promise<Instance> {
		return { tenancy_mode: 'multi', setup_required: !this.setupDone };
	}

	async setup(_request: SetupRequest): Promise<Me> {
		void _request;
		this.setupDone = true;
		return this.me();
	}

	async listUsers(): Promise<User[]> {
		return [...this.users];
	}

	async createUser(request: CreateUserRequest): Promise<User> {
		const user: User = {
			id: `u_${this.users.length + 1}`,
			username: request.username,
			display_name: request.display_name,
			role: request.role as User['role'],
			disabled: false,
			shared: false
		};
		this.users = [...this.users, user];
		return user;
	}

	async updateUser(id: string, patch: UpdateUserRequest): Promise<User> {
		const index = this.users.findIndex((user) => user.id === id);
		if (index === -1) throw new Error('not found');
		const current = this.users[index];
		const updated: User = {
			...current,
			display_name: patch.display_name ?? current.display_name,
			disabled: patch.disabled ?? current.disabled,
			role: (patch.role as User['role'] | undefined) ?? current.role
		};
		this.users = this.users.map((user) => (user.id === id ? updated : user));
		return updated;
	}

	async deleteUser(id: string): Promise<void> {
		this.users = this.users.map((user) => (user.id === id ? { ...user, disabled: true } : user));
	}

	async resetUserPassword(_id: string, _request: ResetPasswordRequest): Promise<void> {
		void _id;
		void _request;
		return undefined;
	}

	async listTokens(): Promise<ApiToken[]> {
		return [...this.tokens];
	}

	async createToken(request: CreateApiTokenRequest): Promise<ApiTokenCreated> {
		const created: ApiTokenCreated = {
			id: `k_${this.tokens.length + 1}`,
			name: request.name,
			role: request.role as ApiTokenCreated['role'],
			display_prefix: 'ef56gh78',
			expires_at: request.expires_at ?? null,
			token: 'rh_EXAMPLEONLYVALUE0000000000000000000000000000'
		};
		const token: ApiToken = {
			id: created.id,
			name: created.name,
			role: created.role,
			display_prefix: created.display_prefix,
			created_at: new Date().toISOString(),
			expires_at: created.expires_at ?? null,
			last_used_at: null,
			revoked_at: null
		};
		this.tokens = [...this.tokens, token];
		return created;
	}

	async revokeToken(id: string): Promise<void> {
		this.tokens = this.tokens.map((token) =>
			token.id === id ? { ...token, revoked_at: new Date().toISOString() } : token
		);
	}

	async listAgents(): Promise<Agent[]> {
		return [...this.agents];
	}

	async renameAgent(id: string, hostname: string): Promise<Agent> {
		const agent = this.agents.find((item) => item.id === id);
		if (!agent) throw new Error('not found');
		agent.hostname = hostname;
		return agent;
	}

	async revokeAgent(id: string): Promise<void> {
		this.agents = this.agents.map((agent) =>
			agent.id === id
				? { ...agent, status: 'revoked', revoked_at: new Date().toISOString() }
				: agent
		);
	}

	async listEnrollmentTokens(): Promise<EnrollmentToken[]> {
		return [...this.enrollmentTokens];
	}

	async createEnrollmentToken(
		request: CreateEnrollmentTokenRequest
	): Promise<EnrollmentTokenCreated> {
		const now = new Date();
		// ADR-0032: the default time to live is one hour.
		const ttl = request.ttl_seconds ?? 3600;
		const value = 're_EXAMPLEONLYVALUE00000000000000000000000000';
		const prefix = value.slice(3, 11);
		const created: EnrollmentTokenCreated = {
			id: `e_${this.enrollmentTokens.length + 1}`,
			token: value,
			display_prefix: prefix,
			labels: request.labels ?? [],
			expires_at: new Date(now.getTime() + ttl * 1000).toISOString(),
			created_at: now.toISOString()
		};
		const token: EnrollmentToken = {
			id: created.id,
			display_prefix: prefix,
			labels: created.labels,
			expires_at: created.expires_at,
			used_at: null,
			revoked_at: null,
			revoked_by_user_id: null,
			created_at: created.created_at,
			created_by: this.users[0].id
		};
		this.enrollmentTokens = [...this.enrollmentTokens, token];
		return created;
	}

	async revokeEnrollmentToken(id: string): Promise<void> {
		const token = this.enrollmentTokens.find((item) => item.id === id);
		if (!token) throw new Error('not found');
		if (token.used_at || token.revoked_at) throw new Error('conflict');
		token.revoked_at = new Date().toISOString();
		token.revoked_by_user_id = this.users[0].id;
	}

	async listRepositories(): Promise<Repository[]> {
		return [...this.repositories];
	}

	async createRepository(request: CreateRepositoryRequest): Promise<Repository> {
		const now = new Date().toISOString();
		const repository: Repository = {
			id: `r_${this.repositories.length + 1}`,
			name: request.name,
			backend: request.backend,
			credentials: {
				password: true,
				s3_access_key: request.backend.kind === 's3',
				s3_secret_key: request.backend.kind === 's3',
				rest_password: request.backend.kind === 'rest' && !!request.rest_password
			},
			initialized: false,
			maintenance_agent_id: request.maintenance_agent_id ?? null,
			created_at: now,
			updated_at: now,
			created_by: this.users[0].id
		};
		this.repositories = [...this.repositories, repository];
		return repository;
	}

	async updateRepository(id: string, request: UpdateRepositoryRequest): Promise<Repository> {
		const current = this.repositories.find((item) => item.id === id);
		if (!current) throw new Error('not found');
		const kind = request.backend?.kind ?? current.backend.kind;
		const updated: Repository = {
			...current,
			name: request.name ?? current.name,
			backend: request.backend ?? current.backend,
			credentials: {
				password: true,
				s3_access_key: kind === 's3' ? true : current.credentials.s3_access_key,
				s3_secret_key: kind === 's3' ? true : current.credentials.s3_secret_key,
				rest_password: !!request.rest_password || current.credentials.rest_password
			},
			maintenance_agent_id:
				request.maintenance_agent_id === undefined
					? current.maintenance_agent_id
					: request.maintenance_agent_id || null,
			updated_at: new Date().toISOString()
		};
		this.repositories = this.repositories.map((item) => (item.id === id ? updated : item));
		return updated;
	}

	async deleteRepository(id: string): Promise<void> {
		this.repositories = this.repositories.filter((item) => item.id !== id);
	}

	async initRepository(id: string, request?: JobActionRequest): Promise<Job> {
		return this.startJob(id, 'init', request);
	}

	async testAccessRepository(id: string, request?: JobActionRequest): Promise<Job> {
		return this.startJob(id, 'test_access', request);
	}

	async getMaintenance(id: string): Promise<Maintenance> {
		const repository = this.repositories.find((item) => item.id === id);
		if (!repository) throw new Error('not found');
		return this.maintenanceView(repository);
	}

	async updateMaintenance(id: string, request: UpdateMaintenanceRequest): Promise<Maintenance> {
		const repository = this.repositories.find((item) => item.id === id);
		if (!repository) throw new Error('not found');
		for (const [field, cron] of [
			['retention.cron', request.retention.cron],
			['check.cron', request.check.cron]
		]) {
			if (cron.trim().split(/\s+/).length !== 5) {
				throw problemError('invalid_argument', field, 'invalid_format');
			}
		}
		this.maintenance.set(id, {
			time_zone: request.time_zone,
			retention: { ...request.retention },
			check: { ...request.check }
		});
		return this.maintenanceView(repository);
	}

	async runMaintenance(id: string, task: MaintenanceTask): Promise<Job> {
		const repository = this.repositories.find((item) => item.id === id);
		if (!repository) throw new Error('not found');
		if (!repository.maintenance_agent_id) {
			throw problemError('invalid_argument', 'maintenance_agent_id', 'empty');
		}
		if (!repository.initialized) throw problemError('conflict', 'id', 'conflict');
		const now = new Date().toISOString();
		const agentId = repository.maintenance_agent_id;
		const job: Job = {
			id: `j_${this.jobs.length + 1}`,
			kind: task === 'retention' ? 'forget' : 'check',
			repository_id: id,
			agent_id: agentId,
			status: 'queued',
			result_code: null,
			result_message: null,
			created_at: now,
			updated_at: now,
			finished_at: null,
			expires_at: null,
			plan_id: null,
			plan_name: null,
			agent_hostname: this.agents.find((item) => item.id === agentId)?.hostname ?? null,
			repository_name: repository.name,
			started_at: null,
			run_at: now,
			attempt: 0
		};
		this.jobs = [
			...this.jobs,
			{ job, readyAt: Date.now() + 700, repositoryId: id, initializes: false }
		];
		return { ...job };
	}

	async unlockAllRepository(id: string, confirm: string): Promise<Job> {
		const repository = this.repositories.find((item) => item.id === id);
		if (!repository) throw new Error('not found');
		if (confirm !== repository.name) throw problemError('invalid_argument', 'confirm', 'mismatch');
		if (!repository.maintenance_agent_id) {
			throw problemError('invalid_argument', 'maintenance_agent_id', 'empty');
		}
		const now = new Date().toISOString();
		const agentId = repository.maintenance_agent_id;
		const job: Job = {
			id: `j_${this.jobs.length + 1}`,
			kind: 'unlock_all',
			repository_id: id,
			agent_id: agentId,
			status: 'queued',
			result_code: null,
			result_message: null,
			created_at: now,
			updated_at: now,
			finished_at: null,
			expires_at: null,
			plan_id: null,
			plan_name: null,
			agent_hostname: this.agents.find((item) => item.id === agentId)?.hostname ?? null,
			repository_name: repository.name,
			started_at: null,
			run_at: now,
			attempt: 0
		};
		this.jobs = [
			...this.jobs,
			{ job, readyAt: Date.now() + 700, repositoryId: id, initializes: false }
		];
		return { ...job };
	}

	private maintenanceView(repository: Repository): Maintenance {
		const stored = this.maintenance.get(repository.id) ?? {
			time_zone: 'UTC',
			retention: { cron: '0 3 * * 0', enabled: true },
			check: { cron: '0 4 1 * *', enabled: true }
		};
		const schedule = (input: { cron: string; enabled: boolean }) => ({
			cron: input.cron,
			time_zone: stored.time_zone,
			enabled: input.enabled,
			next_run_at: input.enabled
				? (nextRunsFromCron(input.cron, stored.time_zone, 1)[0] ?? null)
				: null,
			last_fire_time: null
		});
		const latest = (kind: JobKind): Job | null => {
			const jobs = this.jobs.filter(
				(entry) => entry.repositoryId === repository.id && entry.job.kind === kind
			);
			const last = jobs[jobs.length - 1];
			return last ? { ...last.job } : null;
		};
		return {
			repository_id: repository.id,
			maintenance_agent_id: repository.maintenance_agent_id ?? null,
			retention: schedule(stored.retention),
			check: schedule(stored.check),
			last_retention: latest('forget'),
			last_check: latest('check')
		};
	}

	// A snapshot list or a tree page needs a moment, so the screen shows the state `pending` first.
	private browse = new Map<
		string,
		{ readyAt: number; repositoryId: string; query?: SnapshotsQuery; page?: LsQueryRequest }
	>();
	private latestSnapshots = new Map<string, { job: string; fetchedAt: number }>();
	private storedPages = new Set<string>();

	private pageKey(repositoryId: string, page: LsQueryRequest): string {
		return [
			repositoryId,
			page.snapshot_id,
			page.path ?? '',
			page.offset ?? 0,
			page.limit ?? 0
		].join('|');
	}

	private browseRepository(id: string, agentId?: string | null): Repository {
		const repository = this.repositories.find((item) => item.id === id);
		if (!repository) throw new Error('not found');
		if (!repository.initialized) throw problemError('conflict', 'id', 'conflict');
		const agent = agentId || repository.maintenance_agent_id || this.agents[0]?.id;
		if (!agent) throw problemError('agent_offline', 'agent_id', 'unknown');
		return repository;
	}

	private snapshotsView(
		jobId: string,
		params: SnapshotsParams | undefined,
		now: number
	): SnapshotsView {
		const entry = this.browse.get(jobId);
		if (!entry?.query) throw new Error('not found');
		const query = entry.query;
		const base = {
			job_id: jobId,
			agent_id: this.repositories.find((item) => item.id === entry.repositoryId)
				?.maintenance_agent_id,
			hosts: query.hosts ?? [],
			tags: query.tags ?? [],
			paths: query.paths ?? [],
			truncated: false,
			error_code: null
		};
		if (now < entry.readyAt) {
			return { ...base, state: 'pending', fetched_at: null, age_seconds: null, snapshots: [] };
		}
		const found = inRange(filterSnapshots(makeSnapshots(), query), params?.from, params?.to);
		return {
			...base,
			state: 'done',
			fetched_at: new Date(entry.readyAt).toISOString(),
			age_seconds: Math.round((now - entry.readyAt) / 1000),
			snapshots: found
		};
	}

	async startSnapshots(id: string, request: SnapshotsQuery): Promise<SnapshotsView> {
		this.browseRepository(id, request.agent_id);
		const job = `b_${this.browse.size + 1}`;
		const readyAt = Date.now() + 250;
		this.browse.set(job, { readyAt, repositoryId: id, query: request });
		this.latestSnapshots.set(id, { job, fetchedAt: readyAt });
		return this.snapshotsView(job, undefined, Date.now());
	}

	async getSnapshots(id: string, params?: SnapshotsParams): Promise<SnapshotsView> {
		if (!this.repositories.some((item) => item.id === id)) throw new Error('not found');
		const job = params?.job ?? this.latestSnapshots.get(id)?.job;
		if (!job) {
			return {
				state: 'empty',
				job_id: null,
				agent_id: null,
				fetched_at: null,
				age_seconds: null,
				truncated: false,
				hosts: [],
				tags: [],
				paths: [],
				error_code: null,
				snapshots: []
			};
		}
		return this.snapshotsView(job, params, Date.now());
	}

	private lsView(jobId: string, now: number, cached = false): LsView {
		const entry = this.browse.get(jobId);
		if (!entry?.page) throw new Error('not found');
		const page = entry.page;
		const base = {
			job_id: jobId,
			snapshot_id: page.snapshot_id,
			path: page.path ?? '',
			offset: page.offset ?? 0,
			limit: page.limit ?? 1000,
			cached,
			error_code: null
		};
		if (now < entry.readyAt) {
			return {
				...base,
				state: 'pending',
				fetched_at: null,
				last: false,
				next_offset: null,
				nodes: []
			};
		}
		const found = listDirectory(base.path, base.offset, base.limit);
		return { ...base, state: 'done', fetched_at: new Date(entry.readyAt).toISOString(), ...found };
	}

	async startLs(id: string, request: LsQueryRequest): Promise<LsView> {
		this.browseRepository(id, request.agent_id);
		const key = this.pageKey(id, request);
		const job = `b_${this.browse.size + 1}`;
		const stored = this.storedPages.has(key);
		this.browse.set(job, {
			readyAt: Date.now() + (stored ? 0 : 150),
			repositoryId: id,
			page: request
		});
		this.storedPages.add(key);
		return this.lsView(job, Date.now(), stored);
	}

	async getLs(id: string, job: string): Promise<LsView> {
		const entry = this.browse.get(job);
		if (!entry || entry.repositoryId !== id) throw new Error('not found');
		return this.lsView(job, Date.now());
	}

	async restoreSnapshot(id: string, request: RestoreRequest): Promise<Job> {
		this.browseRepository(id, request.agent_id);
		if (request.includes?.length && request.excludes?.length) {
			throw problemError('invalid_argument', 'excludes', 'invalid_body');
		}
		return this.startJob(id, 'restore', { agent_id: request.agent_id }, false);
	}

	async getJob(id: string): Promise<JobDetail> {
		const entry = this.jobs.find((item) => item.job.id === id);
		if (!entry) throw new Error('not found');
		// The mock reaches a terminal state after a short delay, so the dialog shows progress then a
		// result, the way the real job does.
		if (Date.now() >= entry.readyAt && !isTerminal(entry.job.status)) {
			const now = new Date().toISOString();
			entry.job.status = 'succeeded';
			entry.job.result_code = 'succeeded';
			entry.job.finished_at = now;
			entry.job.updated_at = now;
			if (entry.initializes) {
				this.repositories = this.repositories.map((repository) =>
					repository.id === entry.repositoryId ? { ...repository, initialized: true } : repository
				);
			}
		}
		return detailOf(entry.job, Date.now());
	}

	async getJobLogs(id: string, query?: JobLogQuery): Promise<JobLogPage> {
		const entry = this.jobs.find((item) => item.job.id === id);
		if (!entry) throw new Error('not found');
		return logPageOf(entry.job, query, Date.now());
	}

	private startJob(id: string, kind: JobKind, request?: JobActionRequest, initializes = true): Job {
		const repository = this.repositories.find((item) => item.id === id);
		if (!repository) throw new Error('not found');
		const agentId =
			request?.agent_id || repository.maintenance_agent_id || this.agents[0]?.id || '';
		if (!agentId) throw new Error('agent_offline');
		const now = new Date().toISOString();
		const job: Job = {
			id: `j_${this.jobs.length + 1}`,
			kind,
			repository_id: id,
			agent_id: agentId,
			status: 'sent',
			result_code: null,
			result_message: null,
			created_at: now,
			updated_at: now,
			finished_at: null,
			expires_at: null,
			plan_id: null,
			plan_name: null,
			agent_hostname: this.agents.find((item) => item.id === agentId)?.hostname ?? null,
			repository_name: repository.name,
			started_at: null,
			run_at: now,
			attempt: 0
		};
		this.jobs = [...this.jobs, { job, readyAt: Date.now() + 700, repositoryId: id, initializes }];
		return { ...job };
	}

	async listJobs(query?: JobQuery): Promise<JobListResponse> {
		const rows = this.jobs
			.map((entry) => entry.job)
			.filter(
				(job) =>
					(!query?.status || job.status === query.status) &&
					(!query?.plan_id || job.plan_id === query.plan_id) &&
					(!query?.repository_id || job.repository_id === query.repository_id) &&
					(!query?.agent_id || job.agent_id === query.agent_id)
			)
			.sort((a, b) => b.created_at.localeCompare(a.created_at));
		const limit = query?.limit ?? 50;
		const start = query?.cursor ? Number(query.cursor) : 0;
		const page = rows.slice(start, start + limit);
		return {
			items: page.map((job) => ({ ...job })),
			next_cursor: start + limit < rows.length ? String(start + limit) : null
		};
	}

	async cancelJob(id: string): Promise<Job> {
		const entry = this.jobs.find((item) => item.job.id === id);
		if (!entry) throw new Error('not found');
		if (isTerminal(entry.job.status)) throw problemError('conflict', 'id', 'conflict');
		const now = new Date().toISOString();
		if (entry.job.status === 'queued') {
			entry.job.status = 'failed';
			entry.job.result_code = 'cancelled';
			entry.job.finished_at = now;
		}
		entry.job.updated_at = now;
		return { ...entry.job };
	}

	async runPlan(id: string): Promise<Job> {
		const plan = this.plans.find((item) => item.id === id);
		if (!plan) throw new Error('not found');
		const now = new Date().toISOString();
		const job: Job = {
			id: `j_${this.jobs.length + 1}`,
			kind: 'backup',
			repository_id: plan.repository_id,
			repository_name: null,
			agent_id: plan.agent_id,
			agent_hostname: plan.agent_hostname,
			plan_id: plan.id,
			plan_name: plan.name,
			status: 'queued',
			result_code: null,
			result_message: null,
			created_at: now,
			updated_at: now,
			finished_at: null,
			started_at: null,
			expires_at: null,
			run_at: now,
			attempt: 0
		};
		this.jobs = [
			...this.jobs,
			{
				job,
				readyAt: Number.MAX_SAFE_INTEGER,
				repositoryId: plan.repository_id,
				initializes: false
			}
		];
		return { ...job };
	}

	async listProfiles(): Promise<Profile[]> {
		return [...this.profiles];
	}

	async createProfile(request: CreateProfileRequest): Promise<ProfileResponse> {
		const now = new Date().toISOString();
		const profile: Profile = {
			id: `p_${this.profiles.length + 1}`,
			name: request.name,
			repository_id: request.repository_id,
			paths: request.paths ?? { all: [], windows: [], linux: [], macos: [] },
			excludes: request.excludes ?? [],
			exclude_files: request.exclude_files ?? [],
			tags: request.tags ?? [],
			options: request.options ?? {
				use_fs_snapshot: false,
				one_file_system: false,
				exclude_caches: false,
				limit_upload_kib: 0,
				limit_download_kib: 0
			},
			hooks: request.hooks ?? [],
			retention: request.retention ?? null,
			check: request.check ?? null,
			created_at: now,
			updated_at: now,
			created_by: this.users[0].id
		};
		this.profiles = [...this.profiles, profile];
		return { profile, warnings: warningsFor(profile), plan_warnings: [] };
	}

	async updateProfile(id: string, request: UpdateProfileRequest): Promise<ProfileResponse> {
		const current = this.profiles.find((item) => item.id === id);
		if (!current) throw new Error('not found');
		const updated: Profile = {
			...current,
			name: request.name ?? current.name,
			repository_id: request.repository_id,
			paths: request.paths ?? { all: [], windows: [], linux: [], macos: [] },
			excludes: request.excludes ?? [],
			exclude_files: request.exclude_files ?? [],
			tags: request.tags ?? [],
			options: request.options ?? current.options,
			hooks: request.hooks ?? [],
			retention: request.retention ?? null,
			check: request.check ?? null,
			updated_at: new Date().toISOString()
		};
		this.profiles = this.profiles.map((item) => (item.id === id ? updated : item));
		return { profile: updated, warnings: warningsFor(updated), plan_warnings: [] };
	}

	async deleteProfile(id: string): Promise<void> {
		this.profiles = this.profiles.filter((item) => item.id !== id);
	}

	async listPlans(): Promise<Plan[]> {
		return [...this.plans];
	}

	async createPlan(request: CreatePlanRequest): Promise<PlanResponse> {
		const profile = this.profiles.find((item) => item.id === request.profile_id);
		const agent = this.agents.find((item) => item.id === request.agent_id);
		if (!profile) throw problemError('invalid_argument', 'profile_id', 'unknown');
		if (!agent || agent.status !== 'active') {
			throw problemError('invalid_argument', 'agent_id', 'unknown');
		}
		const now = new Date().toISOString();
		const plan: Plan = {
			id: `pl_${this.plans.length + 1}`,
			name: request.name,
			profile_id: request.profile_id,
			profile_name: profile.name,
			repository_id: profile.repository_id,
			agent_id: request.agent_id,
			agent_hostname: agent.hostname,
			cron: request.cron,
			time_zone: request.time_zone,
			missed_run_policy: request.missed_run_policy,
			enabled: request.enabled ?? true,
			next_run_at: request.enabled === false ? null : nextFireTime(request.cron, request.time_zone),
			last_fire_time: null,
			created_at: now,
			updated_at: now
		};
		this.plans = [...this.plans, plan];
		return { plan };
	}

	async updatePlan(id: string, request: UpdatePlanRequest): Promise<PlanResponse> {
		const current = this.plans.find((item) => item.id === id);
		if (!current) throw new Error('not found');
		const profile = this.profiles.find((item) => item.id === request.profile_id);
		const agent = this.agents.find((item) => item.id === request.agent_id);
		if (!profile) throw problemError('invalid_argument', 'profile_id', 'unknown');
		if (!agent || agent.status !== 'active') {
			throw problemError('invalid_argument', 'agent_id', 'unknown');
		}
		const enabled = request.enabled ?? true;
		const updated: Plan = {
			...current,
			name: request.name ?? current.name,
			profile_id: request.profile_id,
			profile_name: profile.name,
			repository_id: profile.repository_id,
			agent_id: request.agent_id,
			agent_hostname: agent.hostname,
			cron: request.cron,
			time_zone: request.time_zone,
			missed_run_policy: request.missed_run_policy,
			enabled,
			next_run_at: enabled ? nextFireTime(request.cron, request.time_zone) : null,
			updated_at: new Date().toISOString()
		};
		this.plans = this.plans.map((item) => (item.id === id ? updated : item));
		return { plan: updated };
	}

	async deletePlan(id: string): Promise<void> {
		this.plans = this.plans.filter((item) => item.id !== id);
	}

	async nextRuns(id: string, count: number): Promise<NextRunsResponse> {
		const plan = this.plans.find((item) => item.id === id);
		if (!plan) throw new Error('not found');
		return {
			plan_id: id,
			time_zone: plan.time_zone,
			runs: nextRunsFromCron(plan.cron, plan.time_zone, count)
		};
	}

	async previewSchedule(request: PreviewScheduleRequest): Promise<SchedulePreviewResponse> {
		const count = request.count ?? 5;
		return {
			time_zone: request.time_zone,
			runs: nextRunsFromCron(request.cron, request.time_zone, count)
		};
	}

	async previewProfile(id: string, agentId: string): Promise<PreviewResponse> {
		const profile = this.profiles.find((item) => item.id === id);
		if (!profile) throw new Error('not found');
		const agent = this.agents.find((item) => item.id === agentId);
		if (!agent) throw new Error('not found');
		const paths = pathsForOs(profile, agent.os);
		if (paths.length === 0) {
			// The compiler refuses a target without a path for its operating system.
			throw problemError('invalid_argument', 'paths', 'empty');
		}
		return {
			profile_id: id,
			repository_id: profile.repository_id,
			steps: [
				{
					kind: 'backup',
					maintenance: false,
					command: `restic backup --one-file-system --tag=daily -- ${paths.join(' ')}`,
					environment: [
						{ name: 'RESTIC_REPOSITORY', value: 's3:https://s3.example.net/backups-prod' },
						{ name: 'RESTIC_PASSWORD', value: '***' },
						{ name: 'AWS_ACCESS_KEY_ID', value: '***' },
						{ name: 'AWS_SECRET_ACCESS_KEY', value: '***' }
					]
				},
				{
					kind: 'forget',
					maintenance: true,
					command: 'restic forget --keep-daily=7 --retry-lock',
					environment: [
						{ name: 'RESTIC_REPOSITORY', value: 's3:https://s3.example.net/backups-prod' },
						{ name: 'RESTIC_PASSWORD', value: '***' }
					]
				}
			]
		};
	}

	async listTenants(): Promise<Tenant[]> {
		return [...this.tenants];
	}
	async createTenant(request: CreateTenantRequest): Promise<Tenant> {
		const tenant: Tenant = {
			id: `t_${this.tenants.length + 1}`,
			name: request.name,
			status: 'active',
			created_at: new Date().toISOString(),
			user_count: 0,
			admin_state: 'none'
		};
		this.tenants = [...this.tenants, tenant];
		return tenant;
	}

	async listPlatformUsers(): Promise<PlatformUser[]> {
		return [...this.platformUsers];
	}

	async createPlatformUser(request: CreatePlatformUserRequest): Promise<PlatformUser> {
		if (
			this.platformUsers.some(
				(user) => user.username.toLowerCase() === request.username.toLowerCase()
			)
		) {
			throw new Error('username_taken');
		}
		if (
			request.email &&
			this.platformUsers.some((user) => user.email?.toLowerCase() === request.email?.toLowerCase())
		) {
			throw new Error('email_taken');
		}
		const memberships = (request.memberships ?? []).map((membership) => ({
			tenant_id: membership.tenant_id,
			tenant_name: this.tenantName(membership.tenant_id),
			role: membership.role,
			disabled: false
		}));
		const user: PlatformUser = {
			id: `pu_${this.platformUsers.length + 1}`,
			username: request.username,
			display_name: request.display_name,
			email: request.email?.toLowerCase() ?? null,
			email_verified: false,
			disabled: false,
			platform_admin: request.platform_admin ?? false,
			memberships
		};
		this.platformUsers = [...this.platformUsers, user];
		return user;
	}

	async updatePlatformUser(id: string, patch: UpdateAccountRequest): Promise<void> {
		this.platformUsers = this.platformUsers.map((user) =>
			user.id === id
				? {
						...user,
						disabled: patch.disabled ?? user.disabled,
						email: patch.email ? patch.email.toLowerCase() : user.email,
						platform_admin: patch.platform_admin ?? user.platform_admin
					}
				: user
		);
	}

	async addPlatformMembership(id: string, request: MembershipInput): Promise<void> {
		this.platformUsers = this.platformUsers.map((user) => {
			if (user.id !== id || user.memberships.some((item) => item.tenant_id === request.tenant_id)) {
				return user;
			}
			return {
				...user,
				memberships: [
					...user.memberships,
					{
						tenant_id: request.tenant_id,
						tenant_name: this.tenantName(request.tenant_id),
						role: request.role,
						disabled: false
					}
				]
			};
		});
	}

	async setPlatformMembershipRole(
		id: string,
		tenantId: string,
		request: SetRoleRequest
	): Promise<void> {
		this.platformUsers = this.platformUsers.map((user) =>
			user.id === id
				? {
						...user,
						memberships: user.memberships.map((item) =>
							item.tenant_id === tenantId ? { ...item, role: request.role } : item
						)
					}
				: user
		);
	}

	async removePlatformMembership(id: string, tenantId: string): Promise<void> {
		this.platformUsers = this.platformUsers.map((user) =>
			user.id === id
				? { ...user, memberships: user.memberships.filter((item) => item.tenant_id !== tenantId) }
				: user
		);
	}

	private tenantName(id: string): string {
		return this.tenants.find((tenant) => tenant.id === id)?.name ?? id;
	}

	async suspendTenant(id: string): Promise<void> {
		this.setTenantStatus(id, 'suspended');
	}

	async resumeTenant(id: string): Promise<void> {
		this.setTenantStatus(id, 'active');
	}

	async deleteTenant(id: string): Promise<void> {
		// The control plane deletes a tenant with a status change, not with a row removal.
		this.setTenantStatus(id, 'deleted');
	}

	private setTenantStatus(id: string, status: Tenant['status']): Tenant {
		const tenant = this.tenants.find((item) => item.id === id);
		if (!tenant) throw new Error('not found');
		tenant.status = status;
		return tenant;
	}

	async listAudit(query: AuditQuery): Promise<AuditPage> {
		const limit = query.limit ?? 20;
		let items = [...this.audit];
		if (query.action) items = items.filter((item) => item.action.includes(query.action as string));
		if (query.actor_type) items = items.filter((item) => item.actor_type === query.actor_type);
		if (query.actor_id)
			items = items.filter((item) => item.actor_id?.includes(query.actor_id as string));
		if (query.object_type) items = items.filter((item) => item.object_type === query.object_type);
		if (query.object_id)
			items = items.filter((item) => item.object_id?.includes(query.object_id as string));
		if (query.from) items = items.filter((item) => item.occurred_at >= (query.from as string));
		if (query.to) items = items.filter((item) => item.occurred_at <= (query.to as string));
		items.sort((a, b) => (a.occurred_at < b.occurred_at ? 1 : -1));

		const start = query.cursor ? Number(query.cursor) : 0;
		const page = items.slice(start, start + limit);
		const nextIndex = start + limit;
		return {
			items: page,
			next_cursor: nextIndex < items.length ? String(nextIndex) : null
		};
	}

	async listPlatformAudit(query: AuditQuery): Promise<AuditPage> {
		return this.listAudit(query);
	}
}

export const mockApi = new MockApiService();
