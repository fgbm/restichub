/*
 * The service layer. One place connects the screens to the API.
 *
 * Set `VITE_USE_MOCK=true` to use the in-memory mock (development and tests).
 * The mock is loaded with a dynamic import, so a production build holds no mock
 * code: Vite replaces the flag with `false` and removes the import.
 */
import { api } from './client';
import { endpoints } from './endpoints';
import type {
	ApiToken,
	ApiTokenCreated,
	ApiTokenListResponse,
	AuditPage,
	AuditQuery,
	ChangePasswordRequest,
	CreateApiTokenRequest,
	CreateTenantRequest,
	CreateUserRequest,
	CreatePlatformUserRequest,
	Instance,
	Me,
	MembershipInput,
	PlatformUser,
	PlatformUserListResponse,
	ResetPasswordRequest,
	SetRoleRequest,
	SetupRequest,
	SetupStatus,
	SignInRequest,
	Tenant,
	TenantAccess,
	TenantAccessListResponse,
	TenantListResponse,
	UpdateAccountRequest,
	UpdateUserRequest,
	User,
	UserListResponse
} from './types';

export interface ApiService {
	signIn(request: SignInRequest): Promise<Me>;
	signOut(): Promise<void>;
	currentUser(): Promise<Me>;
	changePassword(request: ChangePasswordRequest): Promise<void>;

	/** The memberships of the caller (ADR-0025). */
	listMemberships(): Promise<TenantAccess[]>;
	/** Moves the session to another tenant and returns the new session. */
	switchTenant(tenantId: string): Promise<Me>;

	setupStatus(): Promise<SetupStatus>;
	setup(request: SetupRequest): Promise<Me>;

	instance(): Promise<Instance>;

	listUsers(): Promise<User[]>;
	createUser(request: CreateUserRequest): Promise<User>;
	updateUser(id: string, patch: UpdateUserRequest): Promise<User>;
	deleteUser(id: string): Promise<void>;
	resetUserPassword(id: string, request: ResetPasswordRequest): Promise<void>;

	listTokens(): Promise<ApiToken[]>;
	createToken(request: CreateApiTokenRequest): Promise<ApiTokenCreated>;
	revokeToken(id: string): Promise<void>;

	listTenants(): Promise<Tenant[]>;
	createTenant(request: CreateTenantRequest): Promise<Tenant>;
	suspendTenant(id: string): Promise<void>;
	resumeTenant(id: string): Promise<void>;
	deleteTenant(id: string): Promise<void>;

	/** Every account of the installation with its memberships (ADR-0028). */
	listPlatformUsers(): Promise<PlatformUser[]>;
	createPlatformUser(request: CreatePlatformUserRequest): Promise<PlatformUser>;
	updatePlatformUser(id: string, patch: UpdateAccountRequest): Promise<void>;
	addPlatformMembership(id: string, request: MembershipInput): Promise<void>;
	setPlatformMembershipRole(id: string, tenantId: string, request: SetRoleRequest): Promise<void>;
	removePlatformMembership(id: string, tenantId: string): Promise<void>;

	listAudit(query: AuditQuery): Promise<AuditPage>;
	listPlatformAudit(query: AuditQuery): Promise<AuditPage>;
}

const httpService: ApiService = {
	signIn: (request) => api.post<Me>(endpoints.auth.signIn, request),
	signOut: () => api.post<void>(endpoints.auth.signOut),
	currentUser: () => api.get<Me>(endpoints.auth.me),
	changePassword: (request) => api.post<void>(endpoints.auth.password, request),

	listMemberships: () =>
		api.get<TenantAccessListResponse>(endpoints.auth.tenants).then((response) => response.items),
	switchTenant: (tenantId) => api.post<Me>(endpoints.auth.switchTenant, { tenant_id: tenantId }),

	setupStatus: () => api.get<SetupStatus>(endpoints.setup.status),
	setup: (request) => api.post<Me>(endpoints.setup.create, request),

	instance: () => api.get<Instance>(endpoints.instance),

	listUsers: () =>
		api.get<UserListResponse>(endpoints.users.list).then((response) => response.items),
	createUser: (request) => api.post<User>(endpoints.users.create, request),
	updateUser: (id, patch) => api.patch<User>(endpoints.users.byId(id), patch),
	deleteUser: (id) => api.delete<void>(endpoints.users.byId(id)),
	resetUserPassword: (id, request) => api.post<void>(endpoints.users.password(id), request),

	listTokens: () =>
		api.get<ApiTokenListResponse>(endpoints.apiTokens.list).then((response) => response.items),
	createToken: (request) => api.post<ApiTokenCreated>(endpoints.apiTokens.create, request),
	revokeToken: (id) => api.delete<void>(endpoints.apiTokens.byId(id)),

	listTenants: () =>
		api.get<TenantListResponse>(endpoints.platform.tenants).then((response) => response.items),
	createTenant: (request) => api.post<Tenant>(endpoints.platform.createTenant, request),
	suspendTenant: (id) => api.post<void>(endpoints.platform.suspendTenant(id)),
	resumeTenant: (id) => api.post<void>(endpoints.platform.resumeTenant(id)),
	deleteTenant: (id) => api.delete<void>(endpoints.platform.deleteTenant(id)),

	listPlatformUsers: () =>
		api.get<PlatformUserListResponse>(endpoints.platform.users).then((response) => response.items),
	createPlatformUser: (request) => api.post<PlatformUser>(endpoints.platform.createUser, request),
	updatePlatformUser: (id, patch) => api.patch<void>(endpoints.platform.userById(id), patch),
	addPlatformMembership: (id, request) =>
		api.post<void>(endpoints.platform.userMemberships(id), request),
	setPlatformMembershipRole: (id, tenantId, request) =>
		api.patch<void>(endpoints.platform.userMembership(id, tenantId), request),
	removePlatformMembership: (id, tenantId) =>
		api.delete<void>(endpoints.platform.userMembership(id, tenantId)),

	listAudit: (query) =>
		api.get<AuditPage>(endpoints.audit.list, {
			query: query as Record<string, string | number | undefined>
		}),

	listPlatformAudit: (query) =>
		api.get<AuditPage>(endpoints.platform.audit, {
			query: query as Record<string, string | number | undefined>
		})
};

export const useMock = import.meta.env.VITE_USE_MOCK === 'true';

/**
 * The service of the application.
 *
 * The mock is loaded with a dynamic import and only when `VITE_USE_MOCK` is set.
 * A production build replaces the flag with `false` and removes the import, so
 * the mock code never reaches the bundle.
 */
export const service: ApiService = useMock
	? ((await import('./mock')).mockApi as unknown as ApiService)
	: httpService;
