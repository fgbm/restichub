/*
 * The filter state of a list page, kept in the query string. A breadcrumb level above the page links
 * back with the same query, so the list opens as the person left it (UI principles 1.2).
 */
import { goto } from '$app/navigation';
import { page } from '$app/state';
import { emptyFilters, readQuery, writeQuery } from '#lib/filters.ts';

export interface QueryFilters<K extends string> {
	readonly value: Record<K, string>;
	/** Applies at once. A change of a filter other than `page` returns the list to its first page. */
	update(patch: Partial<Record<K, string>>): void;
	/** Clears the filters and the page. The page size and the sort order stay: they are the view. */
	reset(): void;
}

const VIEW_KEYS: readonly string[] = ['size', 'sort', 'order'];

/** Call during the initialization of a page component. */
export function queryFilters<K extends string>(keys: readonly K[]): QueryFilters<K> {
	let state = $state(readQuery(page.url.searchParams, keys));

	$effect(() => {
		const fresh = readQuery(page.url.searchParams, keys);
		if (keys.some((key) => fresh[key] !== state[key])) state = fresh;
	});

	function sync(): void {
		const query = writeQuery(state, page.url.searchParams).toString();
		const address = query ? `${page.url.pathname}?${query}` : page.url.pathname;
		// The path comes from the current address, which is already resolved.
		// eslint-disable-next-line svelte/no-navigation-without-resolve
		void goto(address, { replace: true, reset: false });
	}

	return {
		get value() {
			return state;
		},
		update(patch) {
			const next = { ...state, ...patch };
			if ((keys as readonly string[]).includes('page') && !('page' in patch)) {
				(next as Record<string, string>).page = '';
			}
			state = next;
			sync();
		},
		reset() {
			const next = emptyFilters(keys);
			for (const key of keys) {
				if (VIEW_KEYS.includes(key)) next[key] = state[key];
			}
			state = next;
			sync();
		}
	};
}
