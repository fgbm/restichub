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
	/** True for the platform section, which single mode hides (ADR-0024). */
	platform: boolean;
}

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
export const navigation: NavSection[] = [
	{
		labelKey: 'nav.section.administration',
		platform: false,
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
	},
	{
		labelKey: 'nav.section.platform',
		platform: true,
		items: [
			{
				href: '/(app)/platform/tenants',
				labelKey: 'nav.tenants',
				icon: 'box',
				roles: ['platform_admin']
			},
			{
				href: '/(app)/platform/users',
				labelKey: 'nav.platformUsers',
				icon: 'users',
				roles: ['platform_admin']
			},
			{
				href: '/(app)/platform/audit',
				labelKey: 'nav.platformAudit',
				icon: 'audit',
				roles: ['platform_admin']
			}
		]
	}
];

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
 * The sections for the current actor.
 *
 * The platform section appears with the Platform Admin right, independently of the current
 * organization (ADR-0029). The administration items follow the role of the current membership.
 */
export function navSectionsForRole(role: Role | null, platformAdmin: boolean): NavSection[] {
	return navigation
		.map((section) => ({
			...section,
			items: section.items.filter((item) =>
				section.platform ? platformAdmin : role ? item.roles.includes(role) : false
			)
		}))
		.filter((section) => section.items.length > 0);
}

export function settingsGroupsForRole(role: Role | null): SettingsGroup[] {
	return settingsNavigation
		.map((group) => ({
			...group,
			items: group.items.filter((item) => (role ? item.roles.includes(role) : false))
		}))
		.filter((group) => group.items.length > 0);
}
