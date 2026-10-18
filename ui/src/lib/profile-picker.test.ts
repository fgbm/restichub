import { describe, expect, it } from 'vitest';
import {
	profileDescription,
	profileOptions,
	profilePathText,
	profileRetention,
	withProfiles
} from './profile-picker';

const retention = {
	keep_last: 0,
	keep_hourly: 0,
	keep_daily: 7,
	keep_weekly: 2,
	keep_monthly: 0,
	keep_yearly: 0,
	keep_within: '',
	keep_tags: [],
	group_by: [],
	filter: { hosts: [], tags: [], paths: [] }
};

const profile = (overrides: Record<string, unknown> = {}) => ({
	id: 'p1',
	name: 'pg-dump',
	paths: { all: ['/var/backups/pg'], windows: [], linux: [], macos: [] },
	retention,
	...overrides
});

describe('the profile picker options', () => {
	it('gives each profile its path and its retention', () => {
		expect(profileOptions([profile()])).toEqual([
			{
				value: 'p1',
				label: 'pg-dump',
				description: ['/var/backups/pg', '7 d · 2 wk'],
				search: ['/var/backups/pg']
			}
		]);
	});

	it('counts the other paths of the profile and finds each of them', () => {
		const paths = { all: ['/srv'], windows: [], linux: ['/etc'], macos: [] };
		const option = profileOptions([profile({ paths })])[0];
		expect(profilePathText(profile({ paths }))).toBe('/srv +1');
		expect(option.description?.[0]).toBe('/srv +1');
		expect(option.search).toEqual(['/srv', '/etc']);
	});

	it('leaves a part that the profile does not hold out of the line', () => {
		expect(profileDescription(profile({ retention: null, paths: { all: [] } }))).toEqual([]);
		expect(profileDescription(profile({ retention: null }))).toEqual(['/var/backups/pg']);
		expect(profileDescription(profile({ paths: { all: [] } }))).toEqual(['7 d · 2 wk']);
	});

	it('shows the retention without a rule as an empty text', () => {
		const noRule = { ...retention, keep_daily: 0, keep_weekly: 0 };
		expect(profileRetention(profile({ retention: noRule }))).toBe('');
		expect(profileRetention(profile({ retention: null }))).toBe('');
		expect(profileRetention(profile({ retention: { ...retention, keep_within: '30d' } }))).toBe(
			'7 d · 2 wk · within 30d'
		);
	});

	it('names the profile in the facet and gives it the same secondary line', () => {
		expect(
			withProfiles(
				[
					{ value: 'p1', count: 2 },
					{ value: 'p9', count: 1 }
				],
				[profile()]
			)
		).toEqual([
			{ value: 'p1', count: 2, label: 'pg-dump', description: ['/var/backups/pg', '7 d · 2 wk'] },
			{ value: 'p9', count: 1 }
		]);
	});
});
