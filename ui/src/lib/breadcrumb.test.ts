import { afterEach, describe, expect, it } from 'vitest';
import { buildBreadcrumb } from './breadcrumb';
import { forgetListQueries, rememberListQuery } from './stores/page-meta.svelte';

const hosts = [{ label: 'Hosts', href: '/hosts' }];

afterEach(() => forgetListQueries());

describe('buildBreadcrumb', () => {
	it('shows the section as the current level of a list page', () => {
		expect(buildBreadcrumb(hosts, null)).toEqual([{ label: 'Hosts' }]);
		expect(buildBreadcrumb(hosts, { crumbs: [] })).toEqual([{ label: 'Hosts' }]);
	});

	it('links the section and adds the levels of the page, the last one current', () => {
		expect(
			buildBreadcrumb(hosts, {
				crumbs: [{ label: 'db-01', href: '/hosts/a1' }, { label: 'Certificate' }]
			})
		).toEqual([
			{ label: 'Hosts', href: '/hosts' },
			{ label: 'db-01', href: '/hosts/a1' },
			{ label: 'Certificate' }
		]);
	});

	it('keeps the list state in the link of the section', () => {
		rememberListQuery('/hosts', '?state=offline&q=web');
		expect(buildBreadcrumb(hosts, { crumbs: [{ label: 'db-01' }] })[0]).toEqual({
			label: 'Hosts',
			href: '/hosts?state=offline&q=web'
		});
		expect(
			buildBreadcrumb(hosts, { crumbs: [{ label: 'db-01' }], listQuery: 'state=online' })[0].href
		).toBe('/hosts?state=online');
	});

	it('skips a level whose label is not known yet', () => {
		expect(buildBreadcrumb(hosts, { crumbs: [{ label: '' }] })).toEqual([{ label: 'Hosts' }]);
	});

	it('keeps both levels of a settings section', () => {
		const settings = [
			{ label: 'Settings', href: '/settings' },
			{ label: 'Templates', href: '/settings/scripts/templates' }
		];
		expect(buildBreadcrumb(settings, { crumbs: [{ label: 'Stop service' }] })).toEqual([
			{ label: 'Settings', href: '/settings' },
			{ label: 'Templates', href: '/settings/scripts/templates' },
			{ label: 'Stop service' }
		]);
		expect(buildBreadcrumb(settings, null)).toEqual([
			{ label: 'Settings', href: '/settings' },
			{ label: 'Templates' }
		]);
	});
});
