/*
 * Navigation sections and items. An item appears only for the roles that may
 * use it.
 */
import type { Role } from '#lib/api/index.ts';
import type { MessageKey } from '#lib/i18n/index.ts';

const ALL_ROLES: Role[] = ['platform_admin', 'admin', 'operator', 'viewer'];

/** The route ids that the navigation uses. */
export type NavHref =
	| '/(app)'
	| '/(app)/users'
	| '/(app)/api-tokens'
	| '/(app)/audit'
	| '/(app)/platform/tenants'
	| '/(app)/platform/audit';

export interface NavItem {
	href: NavHref;
	labelKey: MessageKey;
	roles: Role[];
}

export interface NavSection {
	labelKey: MessageKey;
	items: NavItem[];
}

export const navigation: NavSection[] = [
	{
		labelKey: 'nav.section.administration',
		items: [
			{ href: '/(app)', labelKey: 'nav.overview', roles: ALL_ROLES },
			{ href: '/(app)/users', labelKey: 'nav.users', roles: ['platform_admin', 'admin'] },
			{
				href: '/(app)/api-tokens',
				labelKey: 'nav.apiTokens',
				roles: ['platform_admin', 'admin']
			},
			{ href: '/(app)/audit', labelKey: 'nav.audit', roles: ALL_ROLES }
		]
	},
	{
		labelKey: 'nav.section.platform',
		items: [
			{
				href: '/(app)/platform/tenants',
				labelKey: 'nav.tenants',
				roles: ['platform_admin']
			},
			{
				href: '/(app)/platform/audit',
				labelKey: 'nav.platformAudit',
				roles: ['platform_admin']
			}
		]
	}
];

/** The sections and items that the role may use. */
export function navSectionsForRole(role: Role | null): NavSection[] {
	if (!role) return [];
	return navigation
		.map((section) => ({
			...section,
			items: section.items.filter((item) => item.roles.includes(role))
		}))
		.filter((section) => section.items.length > 0);
}
