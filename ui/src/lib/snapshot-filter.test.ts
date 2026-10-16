import { describe, expect, it } from 'vitest';
import type { SnapshotItem } from '#lib/api/index.ts';
import { filterSnapshots, inRange, makeSnapshots } from '#lib/api/mock-restore.ts';
import { EMPTY_FILTERS } from './restore';
import { applyFilters, facetCounts, hasFilters, periodWindow, splitList } from './snapshot-filter';

const NOW = Date.parse('2026-10-16T12:00:00Z');
const HOUR = 3_600_000;

function item(
	id: string,
	hostname: string,
	tags: string[],
	paths: string[],
	hoursAgo: number | null
) {
	return {
		id: id.padEnd(64, '0'),
		short_id: id,
		time: hoursAgo === null ? null : new Date(NOW - hoursAgo * HOUR).toISOString(),
		hostname,
		username: 'root',
		tags,
		paths,
		program_version: 'restic 0.18.1',
		total_files: 1,
		total_bytes: 1
	} satisfies SnapshotItem;
}

const fixtures = [
	item('s1', 'MindCore', ['daily', 'documents'], ['C:\\Users\\Vch', 'D:\\Projects'], 2),
	item('s2', 'MindCore', ['daily'], ['C:\\Users\\Vch'], 30),
	item('s3', 'nas-backup', ['weekly', 'offsite'], ['/srv', '/etc'], 100),
	item('s4', 'nas-backup', ['daily'], ['/srv'], 200),
	item('s5', 'ws-anna', ['laptop'], ['/home/anna'], 800),
	item('s6', 'ws-anna', [], ['/home/anna'], 3000),
	item('s7', 'ws-anna', ['daily'], ['/home/anna'], null)
];

const ids = (list: readonly SnapshotItem[]) => list.map((entry) => entry.short_id);

// The reference is what the control plane and restic return for the same filters: restic repeats
// `--host` and `--tag` as alternatives, and the control plane keeps the snapshots inside the window.
function server(hosts: string[], tags: string[], from?: number, to?: number): SnapshotItem[] {
	return fixtures.filter((entry) => {
		if (hosts.length > 0 && !hosts.includes(entry.hostname)) return false;
		if (tags.length > 0 && !tags.some((tag) => entry.tags.includes(tag))) return false;
		if (from === undefined && to === undefined) return true;
		const time = entry.time ? Date.parse(entry.time) : Number.NaN;
		return time >= (from ?? Number.NEGATIVE_INFINITY) && time <= (to ?? Number.POSITIVE_INFINITY);
	});
}

describe('snapshot filter (FR-RS-01, AC-RS-01)', () => {
	it('ac_rs_01_a_single_host_or_tag_agrees_with_the_server_filter', () => {
		const snapshots = makeSnapshots();
		const hosts = [...new Set(snapshots.map((entry) => entry.hostname))];
		const tags = [...new Set(snapshots.flatMap((entry) => entry.tags))];
		for (const host of hosts) {
			expect(ids(applyFilters(snapshots, { ...EMPTY_FILTERS, host }, NOW))).toEqual(
				ids(filterSnapshots(snapshots, { hosts: [host] }))
			);
		}
		for (const tag of tags) {
			expect(ids(applyFilters(snapshots, { ...EMPTY_FILTERS, tag }, NOW))).toEqual(
				ids(filterSnapshots(snapshots, { tags: [tag] }))
			);
		}
	});

	it('ac_rs_01_several_hosts_and_tags_match_any_of_them_and_the_facets_all_match', () => {
		const filters = { ...EMPTY_FILTERS, host: 'MindCore,nas-backup', tag: 'weekly,documents' };
		expect(ids(applyFilters(fixtures, filters, NOW))).toEqual(['s1', 's3']);
		expect(ids(applyFilters(fixtures, filters, NOW))).toEqual(
			ids(server(['MindCore', 'nas-backup'], ['weekly', 'documents']))
		);
		expect(ids(applyFilters(fixtures, { ...EMPTY_FILTERS, host: 'ws-anna' }, NOW))).toEqual(
			ids(server(['ws-anna'], []))
		);
	});

	it('ac_rs_01_a_custom_range_agrees_with_the_date_window_of_the_control_plane', () => {
		const filters = { ...EMPTY_FILTERS, period: 'custom', from: '2026-10-10', to: '2026-10-15' };
		const from = new Date('2026-10-10T00:00:00').toISOString();
		const to = new Date('2026-10-15T23:59:59.999').toISOString();
		expect(ids(applyFilters(fixtures, filters, NOW))).toEqual(ids(inRange(fixtures, from, to)));
		const open = { ...EMPTY_FILTERS, from: '2026-10-15' };
		expect(ids(applyFilters(fixtures, open, NOW))).toEqual(
			ids(inRange(fixtures, new Date('2026-10-15T00:00:00').toISOString()))
		);
	});

	it('ac_rs_01_a_preset_period_counts_back_from_now_and_drops_a_snapshot_without_time', () => {
		const week = applyFilters(fixtures, { ...EMPTY_FILTERS, period: '7d' }, NOW);
		expect(ids(week)).toEqual(ids(server([], [], NOW - 7 * 24 * HOUR)));
		expect(ids(week)).toEqual(['s1', 's2', 's3']);
		const day = applyFilters(fixtures, { ...EMPTY_FILTERS, period: '24h' }, NOW);
		expect(ids(day)).toEqual(['s1']);
		// Without a period every snapshot stays, the one without a time included.
		expect(applyFilters(fixtures, EMPTY_FILTERS, NOW)).toHaveLength(fixtures.length);
	});

	it('fr_rs_01_the_path_text_matches_any_backup_path_ignoring_case', () => {
		const path = (text: string) =>
			ids(applyFilters(fixtures, { ...EMPTY_FILTERS, path: text }, NOW));
		expect(path('c:\\users')).toEqual(['s1', 's2']);
		expect(path('D:\\PROJECTS')).toEqual(['s1']);
		expect(path('/srv')).toEqual(['s3', 's4']);
		expect(path('  ')).toHaveLength(fixtures.length);
		expect(path('nothing')).toEqual([]);
	});

	it('fr_rs_01_counts_the_values_that_the_list_holds_the_most_frequent_first', () => {
		expect(facetCounts(fixtures, (entry) => [entry.hostname])).toEqual(
			[
				{ value: 'MindCore', count: 2 },
				{ value: 'nas-backup', count: 2 },
				{ value: 'ws-anna', count: 3 }
			].sort((a, b) => b.count - a.count || a.value.localeCompare(b.value))
		);
		expect(facetCounts(fixtures, (entry) => entry.tags)[0]).toEqual({ value: 'daily', count: 4 });
	});

	it('fr_rs_01_reads_the_lists_of_the_address_and_knows_when_a_filter_is_set', () => {
		expect(splitList(' a, b,,c ')).toEqual(['a', 'b', 'c']);
		expect(hasFilters(EMPTY_FILTERS)).toBe(false);
		expect(hasFilters({ ...EMPTY_FILTERS, from: '2026-10-01' })).toBe(true);
		expect(hasFilters({ ...EMPTY_FILTERS, path: ' x ' })).toBe(true);
		expect(periodWindow(EMPTY_FILTERS, NOW).start).toBe(Number.NEGATIVE_INFINITY);
	});
});
