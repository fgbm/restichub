/*
 * In-memory mock of the API service. Development and tests use it only.
 *
 * The module holds the same functions as the real service, so a screen does not
 * know which one it uses. The tenant functions are the planned API of issue #20.
 */
import type {
	ApiToken,
	ApiTokenCreated,
	AuditPage,
	AuditQuery,
	AuditRecord,
	ChangePasswordRequest,
	CreateApiTokenRequest,
	CreateTenantRequest,
	CreateUserRequest,
	Me,
	ResetPasswordRequest,
	SetupRequest,
	SetupStatus,
	SignInRequest,
	Tenant,
	UpdateUserRequest,
	User
} from './types';

const REDACTED = '<redacted>';

function iso(minutesAgo: number): string {
	return new Date(Date.now() - minutesAgo * 60_000).toISOString();
}

function makeUsers(): User[] {
	return [
		{ id: 'u_01', username: 'admin', display_name: 'Anna Petrova', role: 'admin', disabled: false },
		{
			id: 'u_02',
			username: 'operator',
			display_name: 'Boris Ivanov',
			role: 'operator',
			disabled: false
		},
		{
			id: 'u_03',
			username: 'viewer',
			display_name: 'Vera Sokolova',
			role: 'viewer',
			disabled: true
		}
	];
}

function makeTenants(): Tenant[] {
	return [
		{ id: 't_01', name: 'acme', status: 'active', created_at: iso(60 * 24 * 90) },
		{ id: 't_02', name: 'globex', status: 'active', created_at: iso(60 * 24 * 60) },
		{ id: 't_03', name: 'initech', status: 'suspended', created_at: iso(60 * 24 * 30) }
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
	private tokens: ApiToken[] = makeTokens();
	private audit: AuditRecord[] = makeAudit();
	private setupDone = false;

	private me(): Me {
		return {
			user: this.users[0],
			tenant: { id: this.tenants[0].id, name: this.tenants[0].name },
			role: this.users[0].role
		};
	}

	async signIn(_request: SignInRequest): Promise<Me> {
		void _request;
		return this.me();
	}

	async signOut(): Promise<void> {
		return undefined;
	}

	async currentUser(): Promise<Me> {
		return this.me();
	}

	async changePassword(_request: ChangePasswordRequest): Promise<void> {
		void _request;
		return undefined;
	}

	async setupStatus(): Promise<SetupStatus> {
		return { setup_required: !this.setupDone };
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
			disabled: false
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

	async listTenants(): Promise<Tenant[]> {
		return [...this.tenants];
	}

	async createTenant(request: CreateTenantRequest): Promise<Tenant> {
		const tenant: Tenant = {
			id: `t_${this.tenants.length + 1}`,
			name: request.name,
			status: 'active',
			created_at: new Date().toISOString()
		};
		this.tenants = [...this.tenants, tenant];
		return tenant;
	}

	async suspendTenant(id: string): Promise<Tenant> {
		return this.setTenantStatus(id, 'suspended');
	}

	async resumeTenant(id: string): Promise<Tenant> {
		return this.setTenantStatus(id, 'active');
	}

	async deleteTenant(id: string): Promise<void> {
		this.tenants = this.tenants.filter((tenant) => tenant.id !== id);
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
