/*
 * The local filter of the snapshot list on the Restore screen (FR-RS-01). The screen loads the whole
 * list of a repository once and narrows it here. The result must agree with the filters that the
 * control plane and restic apply to the same list (AC-RS-01).
 */
import type { SnapshotItem } from '#lib/api/index.ts';
import { t } from '#lib/i18n/index.ts';
import { profileOfTag } from '#lib/restore.ts';
import type { RestoreFilters } from '#lib/restore.ts';
import { narrowsPeriod, periodWindow, splitList } from './filters';

export {
	CUSTOM_PERIOD,
	PERIOD_PRESETS,
	facetCounts,
	joinList,
	narrowsPeriod,
	periodOf,
	periodWindow,
	splitList
} from './filters';

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

/** The name of a tag on screen: an automatic profile tag shows the name of its profile. */
export function tagLabel(tag: string, profiles: ReadonlyMap<string, string>): string {
	const profile = profileOfTag(tag);
	if (!profile) return tag;
	return t('restore.profileTag', { name: profiles.get(profile) ?? profile.slice(0, 8) });
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
