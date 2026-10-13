/*
 * Navigation sections and items. An item appears only for the roles that may
 * use it.
 */
import type { Role } from '#lib/api/index.ts';
import type { MessageKey } from '#lib/i18n/index.ts';

const ALL_ROLES: Role[] = ['platform_admin', 'admin', 'operator', 'viewer'];

export type NavHref =
	| '/(app)'
	| '/(app)/users'
	| '/(app)/api-tokens'
	| '/(app)/audit'
	| '/(app)/platform/tenants'
	| '/(app)/platform/users'
	| '/(app)/platform/audit';

export interface NavItem {
	href: NavHref;
	labelKey: MessageKey;
	roles: Role[];
}

export interface NavSection {
	labelKey: MessageKey;
	items: NavItem[];
	/** True for the platform section, which single mode hides (ADR-0024). */
	platform: boolean;
}

export const navigation: NavSection[] = [
	{
		labelKey: 'nav.section.administration',
		platform: false,
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
		platform: true,
		items: [
			{
				href: '/(app)/platform/tenants',
				labelKey: 'nav.tenants',
				roles: ['platform_admin']
			},
			{
				href: '/(app)/platform/users',
				labelKey: 'nav.platformUsers',
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
