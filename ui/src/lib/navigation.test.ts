import { describe, expect, it } from 'vitest';
import { navSectionsForRole, settingsGroupsForRole } from './navigation';

function hrefs(role: Parameters<typeof navSectionsForRole>[0], platformAdmin: boolean): string[] {
	return navSectionsForRole(role, platformAdmin).flatMap((section) =>
		section.items.map((item) => item.href)
	);
}

describe('navigation', () => {
	it('shows only the tabs whose screens exist, in the order of the design', () => {
		expect(hrefs('admin', false)).toEqual([
			'/(app)',
			'/(app)/hosts',
			'/(app)/audit',
			'/(app)/settings'
		]);
	});

	it('hides Settings from an Operator and a Viewer', () => {
		expect(hrefs('operator', false)).toEqual(['/(app)', '/(app)/hosts', '/(app)/audit']);
		expect(hrefs('viewer', false)).toEqual(['/(app)', '/(app)/hosts', '/(app)/audit']);
	});

	it('adds the platform section for the Platform Admin right in any organization', () => {
		const sections = navSectionsForRole('viewer', true);
		const platform = sections.find((section) => section.platform);
		expect(platform?.items.map((item) => item.href)).toEqual([
			'/(app)/platform/tenants',
			'/(app)/platform/users',
			'/(app)/platform/audit'
		]);
	});

	it('shows the platform section without an organization', () => {
		expect(hrefs(null, true)).toEqual([
			'/(app)/platform/tenants',
			'/(app)/platform/users',
			'/(app)/platform/audit'
		]);
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
