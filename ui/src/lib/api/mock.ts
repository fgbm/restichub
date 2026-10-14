/*
 * In-memory mock of the API service. Development and tests use it only.
 *
 * The module holds the same functions as the real service, so a screen does not
 * know which one it uses. The tenant functions are the planned API of issue #20.
 */
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
	JobKind,
	JobStatus,
	Me,
	MembershipInput,
	NextRunsResponse,
	Plan,
	PlanResponse,
	PlatformUser,
	PreviewResponse,
	Profile,
	ProfileResponse,
	Repository,
	ResetPasswordRequest,
	SetRoleRequest,
	SetupRequest,
	SetupStatus,
	SignInRequest,
	Tenant,
	TenantAccess,
	UpdateAccountRequest,
	UpdatePlanRequest,
	UpdateProfileRequest,
	UpdateRepositoryRequest,
	UpdateUserRequest,
	User
} from './types';

const REDACTED = '<redacted>';

function iso(minutesAgo: number): string {
	return new Date(Date.now() - minutesAgo * 60_000).toISOString();
}

/// The next fire time of a mock plan: one hour from now, in UTC (FR-SC-01).
function nextFireTime(): string {
	return new Date(Date.now() + 60 * 60_000).toISOString();
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
			next_run_at: nextFireTime(),
			last_fire_time: iso(60 * 24),
			created_at: iso(60 * 24 * 9),
			updated_at: iso(60 * 24)
		}
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
	return status === 'succeeded' || status === 'failed' || status === 'expired';
}

export class MockApiService {
	private users: User[] = makeUsers();
	private tenants: Tenant[] = makeTenants();
	private platformUsers: PlatformUser[] = makePlatformUsers();
	private tokens: ApiToken[] = makeTokens();
	private agents: Agent[] = makeAgents();
	private enrollmentTokens: EnrollmentToken[] = [];
	private repositories: Repository[] = [];
	private jobs: MockJob[] = [];
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

	async getJob(id: string): Promise<Job> {
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
		return { ...entry.job };
	}

	private startJob(id: string, kind: JobKind, request?: JobActionRequest): Job {
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
			expires_at: null
		};
		this.jobs = [
			...this.jobs,
			{ job, readyAt: Date.now() + 700, repositoryId: id, initializes: true }
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
		return { profile, warnings: warningsFor(profile) };
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
		return { profile: updated, warnings: warningsFor(updated) };
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
			next_run_at: request.enabled === false ? null : nextFireTime(),
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
			next_run_at: enabled ? nextFireTime() : null,
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
		// The mock has no cron parser, so it stands in one fire time a day from now.
		const runs: string[] = [];
		let at = Date.now() + 60 * 60_000;
		for (let index = 0; index < count; index += 1) {
			runs.push(new Date(at).toISOString());
			at += 24 * 60 * 60_000;
		}
		return { plan_id: id, time_zone: plan.time_zone, runs };
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
