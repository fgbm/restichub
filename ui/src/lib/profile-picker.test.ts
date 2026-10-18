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
	parameters: [],
	...overrides
});

function parameter(name: string) {
	return {
		name,
		kind: 'text' as const,
		required: false,
		default_value: null,
		description: null,
		hidden: false
	};
}

describe('the profile picker options', () => {
	it('gives each profile its path and its retention', () => {
		expect(profileOptions([profile()])).toEqual([
			{
				value: 'p1',
				label: 'pg-dump',
				description: ['/var/backups/pg', '7 d · 2 wk'],
				detail: ['/var/backups/pg'],
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

	it('fr_pf_09_shows_the_count_of_the_parameters_as_the_first_part', () => {
		const option = profileOptions([
			profile({ parameters: [parameter('pg_dir'), parameter('pg_password')] })
		])[0];
		expect(option.description).toEqual([
			{ tag: '2 parameters', icon: 'braces' },
			'/var/backups/pg',
			'7 d · 2 wk'
		]);
		expect(option.detail).toEqual([
			'/var/backups/pg',
			{ tag: '2 parameters', icon: 'braces' }
		]);
		// A profile without a parameter shows no chip: an empty field does not show.
		expect(profileOptions([profile()])[0].description).toEqual(['/var/backups/pg', '7 d · 2 wk']);
		expect(profileOptions([profile()])[0].detail).toEqual(['/var/backups/pg']);
	});

	it('fr_pf_10_finds_a_profile_by_the_name_of_a_parameter', () => {
		const option = profileOptions([profile({ parameters: [parameter('pg_host')] })])[0];
		expect(option.search).toEqual(['/var/backups/pg', 'pg_host']);
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
