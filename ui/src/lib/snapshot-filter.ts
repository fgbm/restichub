/*
 * The local filter of the snapshot list on the Restore screen (FR-RS-01). The screen loads the whole
 * list of a repository once and narrows it here. The result must agree with the filters that the
 * control plane and restic apply to the same list (AC-RS-01).
 */
import type { SnapshotItem } from '#lib/api/index.ts';
import { t } from '#lib/i18n/index.ts';
import { profileOfTag } from '#lib/restore.ts';
import type { RestoreFilters } from '#lib/restore.ts';

const HOUR = 3_600_000;
const DAY = 24 * HOUR;

/** The presets of the period filter; `custom` takes the dates of `from` and `to`. */
export const PERIOD_PRESETS = ['24h', '7d', '30d', '90d'] as const;
export const CUSTOM_PERIOD = 'custom';

const PRESET_SPAN: Record<(typeof PERIOD_PRESETS)[number], number> = {
	'24h': DAY,
	'7d': 7 * DAY,
	'30d': 30 * DAY,
	'90d': 90 * DAY
};

/** The values of a multi-value filter. A host or a tag holds no comma, so a comma separates them. */
export function splitList(value: string): string[] {
	return value
		.split(',')
		.map((item) => item.trim())
		.filter((item) => item !== '');
}

export function joinList(values: readonly string[]): string {
	return values.join(',');
}

/** The period of the filters. A bare date range from an older address counts as a custom one. */
export function periodOf(filters: Pick<RestoreFilters, 'period' | 'from' | 'to'>): string {
	if (filters.period) return filters.period;
	return filters.from || filters.to ? CUSTOM_PERIOD : '';
}

/** The time window in milliseconds. A day of a custom range covers the whole local day. */
export function periodWindow(
	filters: Pick<RestoreFilters, 'period' | 'from' | 'to'>,
	now: number
): { start: number; end: number } {
	const period = periodOf(filters);
	const span = PRESET_SPAN[period as keyof typeof PRESET_SPAN];
	if (span !== undefined) return { start: now - span, end: Number.POSITIVE_INFINITY };
	if (period !== CUSTOM_PERIOD) {
		return { start: Number.NEGATIVE_INFINITY, end: Number.POSITIVE_INFINITY };
	}
	return {
		start: filters.from ? new Date(`${filters.from}T00:00:00`).getTime() : Number.NEGATIVE_INFINITY,
		end: filters.to ? new Date(`${filters.to}T23:59:59.999`).getTime() : Number.POSITIVE_INFINITY
	};
}

/**
 * The snapshots that match the filters. A host or a tag matches when it is one of the chosen values
 * (restic treats repeated `--host` and `--tag` flags the same way). The facets combine with AND. A
 * snapshot without a time falls out of a bounded period, as in the control plane. The path text
 * matches when any backup path holds it, ignoring case.
 */
export function applyFilters(
	snapshots: readonly SnapshotItem[],
	filters: RestoreFilters,
	now: number
): SnapshotItem[] {
	const hosts = splitList(filters.host);
	const tags = splitList(filters.tag);
	const text = filters.path.trim().toLowerCase();
	const { start, end } = periodWindow(filters, now);
	const bounded = Number.isFinite(start) || Number.isFinite(end);
	return snapshots.filter((item) => {
		if (hosts.length > 0 && !hosts.includes(item.hostname)) return false;
		if (tags.length > 0 && !tags.some((tag) => item.tags.includes(tag))) return false;
		if (text !== '' && !item.paths.some((path) => path.toLowerCase().includes(text))) return false;
		if (!bounded) return true;
		const time = item.time ? Date.parse(item.time) : Number.NaN;
		return time >= start && time <= end;
	});
}

/** The distinct values with the number of snapshots that carry them, the most frequent first. */
export function facetCounts(
	snapshots: readonly SnapshotItem[],
	pick: (item: SnapshotItem) => readonly string[]
): { value: string; count: number }[] {
	const counts = new Map<string, number>();
	for (const item of snapshots) {
		for (const value of new Set(pick(item))) counts.set(value, (counts.get(value) ?? 0) + 1);
	}
	return [...counts]
		.map(([value, count]) => ({ value, count }))
		.sort((a, b) => b.count - a.count || a.value.localeCompare(b.value));
}

/** The name of a tag on screen: an automatic profile tag shows the name of its profile. */
export function tagLabel(tag: string, profiles: ReadonlyMap<string, string>): string {
	const profile = profileOfTag(tag);
	if (!profile) return tag;
	return t('restore.profileTag', { name: profiles.get(profile) ?? profile.slice(0, 8) });
}

/** True when the period limits the list. A custom range without a date does not. */
export function narrowsPeriod(filters: Pick<RestoreFilters, 'period' | 'from' | 'to'>): boolean {
	const period = periodOf(filters);
	return period !== '' && (period !== CUSTOM_PERIOD || Boolean(filters.from || filters.to));
}

/** True when any filter narrows the list. */
export function hasFilters(filters: RestoreFilters): boolean {
	return (
		splitList(filters.host).length > 0 ||
		splitList(filters.tag).length > 0 ||
		filters.path.trim() !== '' ||
		narrowsPeriod(filters)
	);
}
