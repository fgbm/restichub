/*
 * Navigation sections and items. An item appears only for the roles that may
 * use it.
 */
import type { Role } from '#lib/api/index.ts';
import type { IconName } from '#lib/components/ui/index.ts';
import type { MessageKey } from '#lib/i18n/index.ts';

const ALL_ROLES: Role[] = ['platform_admin', 'admin', 'operator', 'viewer'];
const ADMIN_ROLES: Role[] = ['platform_admin', 'admin'];

export type SettingsHref = '/(app)/settings/access/users' | '/(app)/settings/access/api-tokens';

export type NavHref =
	| '/(app)'
	| '/(app)/hosts'
	| '/(app)/repositories'
	| '/(app)/profiles'
	| '/(app)/plans'
	| '/(app)/jobs'
	| '/(app)/audit'
	| '/(app)/settings'
	| SettingsHref
	| '/(app)/platform/tenants'
	| '/(app)/platform/users'
	| '/(app)/platform/audit';

export interface NavItem {
	href: NavHref;
	labelKey: MessageKey;
	icon: IconName;
	roles: Role[];
}

export interface NavSection {
	labelKey: MessageKey;
	items: NavItem[];
}

export type NavMode = 'organization' | 'platform';

export interface SettingsItem {
	href: SettingsHref;
	labelKey: MessageKey;
	icon: IconName;
	roles: Role[];
}

export interface SettingsGroup {
	labelKey: MessageKey;
	items: SettingsItem[];
}

// The order follows the full tab set of the design; a tab appears only when its screen exists.
export const navigation: NavSection = {
	labelKey: 'nav.section.administration',
	items: [
		{ href: '/(app)', labelKey: 'nav.overview', icon: 'overview', roles: ALL_ROLES },
		{
			href: '/(app)/hosts',
			labelKey: 'nav.hosts',
			icon: 'host',
			roles: ['admin', 'operator', 'viewer']
		},
		{
			href: '/(app)/repositories',
			labelKey: 'nav.repositories',
			icon: 'repository',
			roles: ['admin', 'operator', 'viewer']
		},
		{
			href: '/(app)/profiles',
			labelKey: 'nav.profiles',
			icon: 'checklist',
			roles: ['admin', 'operator', 'viewer']
		},
		{
			href: '/(app)/plans',
			labelKey: 'nav.plans',
			icon: 'calendar',
			roles: ['admin', 'operator', 'viewer']
		},
		{
			href: '/(app)/jobs',
			labelKey: 'nav.jobs',
			icon: 'job',
			roles: ['admin', 'operator', 'viewer']
		},
		{ href: '/(app)/audit', labelKey: 'nav.audit', icon: 'audit', roles: ALL_ROLES },
		{ href: '/(app)/settings', labelKey: 'nav.settings', icon: 'settings', roles: ADMIN_ROLES }
	]
};

// The platform screens have their own tab set; the switcher entry opens them (ADR-0029).
export const platformNavigation: NavSection = {
	labelKey: 'nav.platformMode',
	items: [
		{
			href: '/(app)/platform/tenants',
			labelKey: 'nav.platform.organizations',
			icon: 'box',
			roles: ['platform_admin']
		},
		{
			href: '/(app)/platform/users',
			labelKey: 'nav.platform.users',
			icon: 'users',
			roles: ['platform_admin']
		},
		{
			href: '/(app)/platform/audit',
			labelKey: 'nav.platform.audit',
			icon: 'audit',
			roles: ['platform_admin']
		}
	]
};

export const PLATFORM_HOME: NavHref = '/(app)/platform/tenants';

export const settingsNavigation: SettingsGroup[] = [
	{
		labelKey: 'settings.group.access',
		items: [
			{
				href: '/(app)/settings/access/users',
				labelKey: 'nav.users',
				icon: 'users',
				roles: ADMIN_ROLES
			},
			{
				href: '/(app)/settings/access/api-tokens',
				labelKey: 'nav.apiTokens',
				icon: 'key',
				roles: ADMIN_ROLES
			}
		]
	}
];

/**
 * The primary tabs for the current actor.
 *
 * The platform tabs need the Platform Admin right, not a membership (ADR-0029). The organization
 * tabs follow the role of the current membership.
 */
export function tabsForMode(mode: NavMode, role: Role | null, platformAdmin: boolean): NavItem[] {
	if (mode === 'platform') return platformAdmin ? platformNavigation.items : [];
	return role ? navigation.items.filter((item) => item.roles.includes(role)) : [];
}

// The address bar holds the resolved path, so the route group `(app)` is absent. The base path
// is empty (ADR-0012).
function isPlatformPath(pathname: string): boolean {
	return platformNavigation.items.some((item) => {
		const path = item.href.replace('/(app)', '');
		return pathname === path || pathname.startsWith(`${path}/`);
	});
}

/**
 * The route a Platform Admin without a current organization must open in place of the address
 * bar. An organization route needs an organization to show, and the right without a membership
 * has none, so the route opens the platform mode (ADR-0029). Returns null for every other actor
 * and for a platform route: an account without the right keeps its empty state.
 */
export function platformRedirectFor(
	me: { tenant?: unknown; platform_admin: boolean } | null,
	singleTenant: boolean,
	pathname: string
): NavHref | null {
	if (!me?.platform_admin || me.tenant || singleTenant || isPlatformPath(pathname)) return null;
	return PLATFORM_HOME;
}

export function settingsGroupsForRole(role: Role | null): SettingsGroup[] {
	return settingsNavigation
		.map((group) => ({
			...group,
			items: group.items.filter((item) => (role ? item.roles.includes(role) : false))
		}))
		.filter((group) => group.items.length > 0);
}
