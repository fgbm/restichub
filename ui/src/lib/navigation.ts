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

/**
 * The first screen after the sign-in. A Platform Admin without a current organization has no
 * organization screen to open, so it starts in the platform screens. Single mode has none.
 */
export function landingHref(
	me: { tenant?: unknown; platform_admin: boolean },
	singleTenant: boolean
): NavHref {
	return me.platform_admin && !me.tenant && !singleTenant ? PLATFORM_HOME : '/(app)';
}

export function settingsGroupsForRole(role: Role | null): SettingsGroup[] {
	return settingsNavigation
		.map((group) => ({
			...group,
			items: group.items.filter((item) => (role ? item.roles.includes(role) : false))
		}))
		.filter((group) => group.items.length > 0);
}
