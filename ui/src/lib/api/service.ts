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
	Instance,
	Me,
	ResetPasswordRequest,
	SetupRequest,
	SetupStatus,
	SignInRequest,
	Tenant,
	TenantListResponse,
	UpdateUserRequest,
	User,
	UserListResponse
} from './types';

export interface ApiService {
	signIn(request: SignInRequest): Promise<Me>;
	signOut(): Promise<void>;
	currentUser(): Promise<Me>;
	changePassword(request: ChangePasswordRequest): Promise<void>;

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

	listAudit(query: AuditQuery): Promise<AuditPage>;
	listPlatformAudit(query: AuditQuery): Promise<AuditPage>;
}

const httpService: ApiService = {
	signIn: (request) => api.post<Me>(endpoints.auth.signIn, request),
	signOut: () => api.post<void>(endpoints.auth.signOut),
	currentUser: () => api.get<Me>(endpoints.auth.me),
	changePassword: (request) => api.post<void>(endpoints.auth.password, request),

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
