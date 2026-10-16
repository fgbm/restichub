/*
 * The filter state of a list: a flat map of query parameters. A multi-value filter joins its values
 * with a comma, so the state round-trips through the address and a parent breadcrumb level can
 * restore the list exactly.
 */

export type FilterState = Record<string, string>;

const HOUR = 3_600_000;
const DAY = 24 * HOUR;

/** The presets of the period filter; `custom` takes the dates of `from` and `to`. */
export const PERIOD_PRESETS = ['24h', '7d', '30d', '90d'] as const;
export const CUSTOM_PERIOD = 'custom';

export const PERIOD_KEYS = ['period', 'from', 'to'] as const;

export interface PeriodFilter {
	period: string;
	from: string;
	to: string;
}

const PRESET_SPAN: Record<(typeof PERIOD_PRESETS)[number], number> = {
	'24h': DAY,
	'7d': 7 * DAY,
	'30d': 30 * DAY,
	'90d': 90 * DAY
};

/** The values of a multi-value filter. A value holds no comma, so a comma separates them. */
export function splitList(value: string | undefined): string[] {
	return (value ?? '')
		.split(',')
		.map((item) => item.trim())
		.filter((item) => item !== '');
}

export function joinList(values: readonly string[]): string {
	return values.join(',');
}

export function toggleValue(list: readonly string[], value: string): string[] {
	return list.includes(value) ? list.filter((item) => item !== value) : [...list, value];
}

/** The period of the filters. A bare date range from an older address counts as a custom one. */
export function periodOf(filters: Partial<PeriodFilter>): string {
	if (filters.period) return filters.period;
	return filters.from || filters.to ? CUSTOM_PERIOD : '';
}

/** The time window in milliseconds. A day of a custom range covers the whole local day. */
export function periodWindow(
	filters: Partial<PeriodFilter>,
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

/** True when the period limits the list. A custom range without a date does not. */
export function narrowsPeriod(filters: Partial<PeriodFilter>): boolean {
	const period = periodOf(filters);
	return period !== '' && (period !== CUSTOM_PERIOD || Boolean(filters.from || filters.to));
}

/** True when the time is inside the period. A missing time falls out of a bounded period. */
export function inPeriod(
	time: string | null | undefined,
	filters: Partial<PeriodFilter>,
	now: number
): boolean {
	const { start, end } = periodWindow(filters, now);
	if (!Number.isFinite(start) && !Number.isFinite(end)) return true;
	const value = time ? Date.parse(time) : Number.NaN;
	return value >= start && value <= end;
}

/** True when any filter narrows the list. The period keys count only when they bound the time. */
export function hasActiveFilters(state: FilterState): boolean {
	return (
		Object.entries(state).some(([key, value]) => {
			if ((PERIOD_KEYS as readonly string[]).includes(key)) return false;
			return value.trim() !== '';
		}) || narrowsPeriod(state)
	);
}

/** The state with every key empty. */
export function emptyFilters<K extends string>(keys: readonly K[]): Record<K, string> {
	return Object.fromEntries(keys.map((key) => [key, ''])) as Record<K, string>;
}

/** The filter values of the keys from a query string. A missing key is empty. */
export function readQuery<K extends string>(
	query: { get(name: string): string | null },
	keys: readonly K[]
): Record<K, string> {
	return Object.fromEntries(keys.map((key) => [key, query.get(key) ?? ''])) as Record<K, string>;
}

/**
 * The query string with the filter values written over `base`. An empty value removes its key, so
 * the parameters of the page that are not filters stay as they are.
 */
export function writeQuery(state: FilterState, base: { toString(): string } = ''): URLSearchParams {
	const query = new URLSearchParams(base.toString());
	for (const [key, value] of Object.entries(state)) {
		if (value) query.set(key, value);
		else query.delete(key);
	}
	return query;
}

/** The distinct values with the number of items that carry them, the most frequent first. */
export function facetCounts<T>(
	items: readonly T[],
	pick: (item: T) => readonly string[]
): { value: string; count: number }[] {
	const counts = new Map<string, number>();
	for (const item of items) {
		for (const value of new Set(pick(item))) counts.set(value, (counts.get(value) ?? 0) + 1);
	}
	return [...counts]
		.map(([value, count]) => ({ value, count }))
		.sort((a, b) => b.count - a.count || a.value.localeCompare(b.value));
}

/** True when one of the texts holds the search text, ignoring case. An empty search matches. */
export function matchesSearch(
	search: string,
	texts: readonly (string | null | undefined)[]
): boolean {
	const needle = search.trim().toLowerCase();
	if (needle === '') return true;
	return texts.some((text) => (text ?? '').toLowerCase().includes(needle));
}
