import { describe, expect, it } from 'vitest';
import {
	emptyFilters,
	facetCounts,
	hasActiveFilters,
	inPeriod,
	matchesSearch,
	readQuery,
	splitList,
	toggleValue,
	writeQuery
} from './filters';

describe('filters', () => {
	it('reads the keys from a query string and writes them over the other parameters', () => {
		const query = new URLSearchParams('q=web&state=online&repository=r1');
		const state = readQuery(query, ['q', 'state', 'os'] as const);
		expect(state).toEqual({ q: 'web', state: 'online', os: '' });
		const written = writeQuery({ ...state, state: '', os: 'linux' }, query);
		expect(written.toString()).toBe('q=web&repository=r1&os=linux');
	});

	it('counts a custom period only when it has a date', () => {
		expect(hasActiveFilters({ q: '', period: '' })).toBe(false);
		expect(hasActiveFilters({ period: 'custom', from: '', to: '' })).toBe(false);
		expect(hasActiveFilters({ period: 'custom', from: '2026-01-01', to: '' })).toBe(true);
		expect(hasActiveFilters({ q: ' web ' })).toBe(true);
		expect(hasActiveFilters(emptyFilters(['q', 'state']))).toBe(false);
	});

	it('keeps a time inside the period and drops a missing time from a bounded period', () => {
		const now = Date.parse('2026-03-10T12:00:00Z');
		expect(inPeriod('2026-03-10T00:00:00Z', { period: '24h' }, now)).toBe(true);
		expect(inPeriod('2026-03-01T00:00:00Z', { period: '7d' }, now)).toBe(false);
		expect(inPeriod(null, { period: '7d' }, now)).toBe(false);
		expect(inPeriod(null, { period: '' }, now)).toBe(true);
	});

	it('splits, toggles and counts the values of a facet', () => {
		expect(splitList(' a, ,b ')).toEqual(['a', 'b']);
		expect(toggleValue(['a', 'b'], 'a')).toEqual(['b']);
		expect(toggleValue(['a'], 'b')).toEqual(['a', 'b']);
		expect(
			facetCounts([{ os: 'linux' }, { os: 'linux' }, { os: 'windows' }], (row) => [row.os])
		).toEqual([
			{ value: 'linux', count: 2 },
			{ value: 'windows', count: 1 }
		]);
	});

	it('matches the search text in any field, ignoring case', () => {
		expect(matchesSearch('WEB', ['db-01', 'web-01'])).toBe(true);
		expect(matchesSearch('mail', ['db-01', null])).toBe(false);
		expect(matchesSearch('  ', ['db-01'])).toBe(true);
	});
});
