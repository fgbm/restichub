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
	CreateTenantRequest,
	CreateUserRequest,
	EnrollmentToken,
	EnrollmentTokenCreated,
	Instance,
	Me,
	MembershipInput,
	PlatformUser,
	ResetPasswordRequest,
	SetRoleRequest,
	SetupRequest,
	SetupStatus,
	SignInRequest,
	Tenant,
	TenantAccess,
	UpdateAccountRequest,
	UpdateUserRequest,
	User
} from './types';

const REDACTED = '<redacted>';

function iso(minutesAgo: number): string {
	return new Date(Date.now() - minutesAgo * 60_000).toISOString();
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

export class MockApiService {
	private users: User[] = makeUsers();
	private tenants: Tenant[] = makeTenants();
	private platformUsers: PlatformUser[] = makePlatformUsers();
	private tokens: ApiToken[] = makeTokens();
	private agents: Agent[] = [];
	private enrollmentTokens: EnrollmentToken[] = [];
	private audit: AuditRecord[] = makeAudit();
	private memberships: TenantAccess[] = makeMemberships();
	private currentTenant = 't_01';
	// The mock stands for a ready instance, so the setup screen stays reachable at its own route.
	// A fresh page starts without a session, the way the real installation does.
	private setupDone = true;
	private signedIn = false;

	private me(): Me {
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
		return this.me();
	}

	async signOut(): Promise<void> {
		this.signedIn = false;
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
		const ttl = request.ttl_seconds ?? 900;
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
