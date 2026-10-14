/*
 * Every API path of the UI in one module.
 *
 * The paths follow the control plane API of issues #18, #19 and #20.
 */
export const API_PREFIX = '/api/v1';

export const endpoints = {
	auth: {
		signIn: '/auth/sign-in',
		signOut: '/auth/sign-out',
		me: '/auth/me',
		password: '/auth/password',
		tenants: '/auth/tenants',
		switchTenant: '/auth/switch'
	},
	setup: {
		status: '/setup/status',
		create: '/setup'
	},
	instance: '/instance',
	users: {
		list: '/users',
		create: '/users',
		byId: (id: string) => `/users/${encodeURIComponent(id)}`,
		password: (id: string) => `/users/${encodeURIComponent(id)}/password`
	},
	apiTokens: {
		list: '/api-tokens',
		create: '/api-tokens',
		byId: (id: string) => `/api-tokens/${encodeURIComponent(id)}`
	},
	agents: {
		list: '/agents',
		byId: (id: string) => `/agents/${encodeURIComponent(id)}`,
		revoke: (id: string) => `/agents/${encodeURIComponent(id)}/revoke`
	},
	enrollmentTokens: {
		list: '/enrollment-tokens',
		create: '/enrollment-tokens',
		byId: (id: string) => `/enrollment-tokens/${encodeURIComponent(id)}`
	},
	repositories: {
		list: '/repositories',
		create: '/repositories',
		byId: (id: string) => `/repositories/${encodeURIComponent(id)}`,
		init: (id: string) => `/repositories/${encodeURIComponent(id)}/init`,
		testAccess: (id: string) => `/repositories/${encodeURIComponent(id)}/test-access`
	},
	jobs: {
		byId: (id: string) => `/jobs/${encodeURIComponent(id)}`
	},
	profiles: {
		list: '/profiles',
		create: '/profiles',
		byId: (id: string) => `/profiles/${encodeURIComponent(id)}`,
		preview: (id: string) => `/profiles/${encodeURIComponent(id)}/preview`
	},
	plans: {
		list: '/plans',
		create: '/plans',
		preview: '/plans/preview',
		byId: (id: string) => `/plans/${encodeURIComponent(id)}`,
		nextRuns: (id: string, count: number) =>
			`/plans/${encodeURIComponent(id)}/next-runs?count=${count}`
	},
	audit: {
		list: '/audit'
	},
	platform: {
		tenants: '/platform/tenants',
		createTenant: '/platform/tenants',
		byId: (id: string) => `/platform/tenants/${encodeURIComponent(id)}`,
		suspendTenant: (id: string) => `/platform/tenants/${encodeURIComponent(id)}/suspend`,
		resumeTenant: (id: string) => `/platform/tenants/${encodeURIComponent(id)}/resume`,
		deleteTenant: (id: string) => `/platform/tenants/${encodeURIComponent(id)}`,
		users: '/platform/users',
		createUser: '/platform/users',
		userById: (id: string) => `/platform/users/${encodeURIComponent(id)}`,
		userMemberships: (id: string) => `/platform/users/${encodeURIComponent(id)}/memberships`,
		userMembership: (id: string, tenantId: string) =>
			`/platform/users/${encodeURIComponent(id)}/memberships/${encodeURIComponent(tenantId)}`,
		audit: '/platform/audit'
	}
} as const;

export type Endpoints = typeof endpoints;
