import { describe, expect, it } from 'vitest';
import { landingHref, settingsGroupsForRole, tabsForMode, type NavMode } from './navigation';

function hrefs(
	mode: NavMode,
	role: Parameters<typeof tabsForMode>[1],
	platformAdmin: boolean
): string[] {
	return tabsForMode(mode, role, platformAdmin).map((item) => item.href);
}

describe('navigation', () => {
	it('shows only the tabs whose screens exist, in the order of the design', () => {
		expect(hrefs('organization', 'admin', false)).toEqual([
			'/(app)',
			'/(app)/hosts',
			'/(app)/audit',
			'/(app)/settings'
		]);
	});

	it('hides Settings from an Operator and a Viewer', () => {
		expect(hrefs('organization', 'operator', false)).toEqual([
			'/(app)',
			'/(app)/hosts',
			'/(app)/audit'
		]);
		expect(hrefs('organization', 'viewer', false)).toEqual([
			'/(app)',
			'/(app)/hosts',
			'/(app)/audit'
		]);
	});

	it('keeps the platform tabs out of the organization tab set', () => {
		expect(hrefs('organization', 'admin', true)).toEqual([
			'/(app)',
			'/(app)/hosts',
			'/(app)/audit',
			'/(app)/settings'
		]);
	});

	it('gives the platform mode exactly Organizations, Users and Audit', () => {
		const tabs = tabsForMode('platform', 'viewer', true);
		expect(tabs.map((item) => item.href)).toEqual([
			'/(app)/platform/tenants',
			'/(app)/platform/users',
			'/(app)/platform/audit'
		]);
		expect(tabs.map((item) => item.labelKey)).toEqual([
			'nav.platform.organizations',
			'nav.platform.users',
			'nav.platform.audit'
		]);
		expect(tabs.map((item) => item.icon)).toEqual(['box', 'users', 'audit']);
	});

	it('gives the platform mode without an organization', () => {
		expect(hrefs('platform', null, true)).toHaveLength(3);
	});

	it('gives no platform tabs without the Platform Admin right', () => {
		expect(hrefs('platform', 'admin', false)).toEqual([]);
	});

	it('lands a Platform Admin without an organization in the platform mode', () => {
		expect(landingHref({ tenant: null, platform_admin: true }, false)).toBe(
			'/(app)/platform/tenants'
		);
		expect(landingHref({ tenant: { id: 't', name: 'acme' }, platform_admin: true }, false)).toBe(
			'/(app)'
		);
		expect(landingHref({ tenant: null, platform_admin: false }, false)).toBe('/(app)');
		expect(landingHref({ tenant: null, platform_admin: true }, true)).toBe('/(app)');
	});

	it('puts users and API tokens under Settings, Access for an Admin', () => {
		const groups = settingsGroupsForRole('admin');
		expect(groups.map((group) => group.labelKey)).toEqual(['settings.group.access']);
		expect(groups[0].items.map((item) => item.href)).toEqual([
			'/(app)/settings/access/users',
			'/(app)/settings/access/api-tokens'
		]);
		expect(settingsGroupsForRole('operator')).toEqual([]);
	});
});
