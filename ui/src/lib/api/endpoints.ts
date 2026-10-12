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
		password: '/auth/password'
	},
	setup: {
		status: '/setup/status',
		create: '/setup'
	},
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
		audit: '/platform/audit'
	}
} as const;

export type Endpoints = typeof endpoints;
