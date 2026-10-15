import { describe, expect, it } from 'vitest';
import {
	platformRedirectFor,
	settingsGroupsForRole,
	tabsForMode,
	type NavMode
} from './navigation';

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
			'/(app)/repositories',
			'/(app)/profiles',
			'/(app)/plans',
			'/(app)/jobs',
			'/(app)/alerts',
			'/(app)/audit',
			'/(app)/settings'
		]);
	});

	it('hides Settings from an Operator and a Viewer', () => {
		expect(hrefs('organization', 'operator', false)).toEqual([
			'/(app)',
			'/(app)/hosts',
			'/(app)/repositories',
			'/(app)/profiles',
			'/(app)/plans',
			'/(app)/jobs',
			'/(app)/alerts',
			'/(app)/audit'
		]);
		expect(hrefs('organization', 'viewer', false)).toEqual([
			'/(app)',
			'/(app)/hosts',
			'/(app)/repositories',
			'/(app)/profiles',
			'/(app)/plans',
			'/(app)/jobs',
			'/(app)/alerts',
			'/(app)/audit'
		]);
	});

	it('keeps the platform tabs out of the organization tab set', () => {
		expect(hrefs('organization', 'admin', true)).toEqual([
			'/(app)',
			'/(app)/hosts',
			'/(app)/repositories',
			'/(app)/profiles',
			'/(app)/plans',
			'/(app)/jobs',
			'/(app)/alerts',
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

	it('sends a Platform Admin without an organization from an organization route to the platform mode', () => {
		const me = { tenant: null, platform_admin: true };
		for (const pathname of [
			'/',
			'/hosts',
			'/audit',
			'/settings/access/users',
			'/settings/access/api-tokens',
			'/users',
			'/api-tokens',
			'/a-bookmark'
		]) {
			expect(platformRedirectFor(me, false, pathname)).toBe('/(app)/platform/tenants');
		}
	});

	it('keeps a platform route for a Platform Admin without an organization', () => {
		const me = { tenant: null, platform_admin: true };
		expect(platformRedirectFor(me, false, '/platform/tenants')).toBeNull();
		expect(platformRedirectFor(me, false, '/platform/users')).toBeNull();
		expect(platformRedirectFor(me, false, '/platform/audit')).toBeNull();
	});

	it('keeps an organization route for the other actors', () => {
		const member = { tenant: { id: 't', name: 'acme' }, platform_admin: true };
		expect(platformRedirectFor(member, false, '/hosts')).toBeNull();
		expect(
			platformRedirectFor({ tenant: null, platform_admin: false }, false, '/hosts')
		).toBeNull();
		expect(platformRedirectFor({ tenant: null, platform_admin: true }, true, '/hosts')).toBeNull();
		expect(platformRedirectFor(null, false, '/hosts')).toBeNull();
	});

	it('puts users and API tokens under Settings, Access for an Admin', () => {
		const groups = settingsGroupsForRole('admin');
		expect(groups.map((group) => group.labelKey)).toEqual([
			'settings.group.access',
			'settings.group.notifications'
		]);
		expect(groups[0].items.map((item) => item.href)).toEqual([
			'/(app)/settings/access/users',
			'/(app)/settings/access/api-tokens'
		]);
		expect(groups[1].items.map((item) => item.href)).toEqual([
			'/(app)/settings/notifications/channels',
			'/(app)/settings/notifications/rules'
		]);
		expect(settingsGroupsForRole('operator')).toEqual([]);
	});
});
