/*
 * Every API path of the UI in one module.
 *
 * The control plane API is not final (issues #18, #19, #20). When the paths
 * change, edit this file only.
 */
export const API_PREFIX = '/api/v1';

export const endpoints = {
	auth: {
		signIn: '/auth/sign-in',
		signOut: '/auth/sign-out',
		me: '/auth/me'
	},
	setup: {
		status: '/setup/status',
		create: '/setup'
	},
	users: {
		list: '/users',
		create: '/users',
		byId: (id: string) => `/users/${encodeURIComponent(id)}`
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
		suspendTenant: (id: string) => `/platform/tenants/${encodeURIComponent(id)}/suspend`,
		resumeTenant: (id: string) => `/platform/tenants/${encodeURIComponent(id)}/resume`,
		deleteTenant: (id: string) => `/platform/tenants/${encodeURIComponent(id)}`,
		audit: '/platform/audit'
	}
} as const;

export type Endpoints = typeof endpoints;
