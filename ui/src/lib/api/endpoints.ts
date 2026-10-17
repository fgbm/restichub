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
		revoke: (id: string) => `/agents/${encodeURIComponent(id)}/revoke`,
		reenroll: (id: string) => `/agents/${encodeURIComponent(id)}/reenroll`,
		replace: (id: string) => `/agents/${encodeURIComponent(id)}/replace`
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
		usage: (id: string) => `/repositories/${encodeURIComponent(id)}/usage`,
		init: (id: string) => `/repositories/${encodeURIComponent(id)}/init`,
		testAccess: (id: string) => `/repositories/${encodeURIComponent(id)}/test-access`,
		maintenance: (id: string) => `/repositories/${encodeURIComponent(id)}/maintenance`,
		runMaintenance: (id: string) => `/repositories/${encodeURIComponent(id)}/maintenance/run`,
		unlockAll: (id: string) => `/repositories/${encodeURIComponent(id)}/unlock-all`,
		snapshots: (id: string) => `/repositories/${encodeURIComponent(id)}/snapshots`,
		ls: (id: string) => `/repositories/${encodeURIComponent(id)}/ls`,
		restore: (id: string) => `/repositories/${encodeURIComponent(id)}/restore`
	},
	jobs: {
		list: '/jobs',
		byId: (id: string) => `/jobs/${encodeURIComponent(id)}`,
		cancel: (id: string) => `/jobs/${encodeURIComponent(id)}/cancel`,
		logs: (id: string) => `/jobs/${encodeURIComponent(id)}/logs`
	},
	profiles: {
		list: '/profiles',
		defaults: '/profiles/defaults',
		create: '/profiles',
		byId: (id: string) => `/profiles/${encodeURIComponent(id)}`,
		usage: (id: string) => `/profiles/${encodeURIComponent(id)}/usage`,
		preview: (id: string) => `/profiles/${encodeURIComponent(id)}/preview`
	},
	plans: {
		list: '/plans',
		create: '/plans',
		preview: '/plans/preview',
		byId: (id: string) => `/plans/${encodeURIComponent(id)}`,
		run: (id: string) => `/plans/${encodeURIComponent(id)}/run`,
		nextRuns: (id: string, count: number) =>
			`/plans/${encodeURIComponent(id)}/next-runs?count=${count}`
	},
	secrets: {
		list: '/secrets',
		create: '/secrets',
		byId: (id: string) => `/secrets/${encodeURIComponent(id)}`,
		value: (id: string) => `/secrets/${encodeURIComponent(id)}/value`
	},
	scriptTemplates: {
		list: '/script-templates',
		create: '/script-templates',
		byId: (id: string) => `/script-templates/${encodeURIComponent(id)}`
	},
	jobTemplates: {
		list: '/job-templates',
		create: '/job-templates',
		byId: (id: string) => `/job-templates/${encodeURIComponent(id)}`
	},
	dashboard: '/dashboard',
	events: '/events',
	alerts: {
		list: '/alerts',
		rules: '/alert-rules'
	},
	channels: {
		list: '/notification-channels',
		create: '/notification-channels',
		byId: (id: string) => `/notification-channels/${encodeURIComponent(id)}`,
		test: (id: string) => `/notification-channels/${encodeURIComponent(id)}/test`
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
		userPassword: (id: string) => `/platform/users/${encodeURIComponent(id)}/password`,
		userMembership: (id: string, tenantId: string) =>
			`/platform/users/${encodeURIComponent(id)}/memberships/${encodeURIComponent(tenantId)}`,
		audit: '/platform/audit'
	}
} as const;

export type Endpoints = typeof endpoints;
