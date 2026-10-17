/*
 * The filter state of a list page, kept in the query string. A breadcrumb level above the page links
 * back with the same query, so the list opens as the person left it (UI principles 1.2).
 * A list that opens with no list parameter in the address gets the view the person kept for it:
 * the tab, the filters, the sort and the page size (FR-UI-05).
 */
import { onMount, setContext } from 'svelte';
import { goto } from '$app/navigation';
import { page } from '$app/state';
import { emptyFilters, joinList, readQuery, splitList, writeQuery } from '#lib/filters.ts';
import {
	isStringRecord,
	loadPreference,
	removePreference,
	savePreference
} from './ui-preferences.svelte.ts';

export interface QueryFilters<K extends string> {
	readonly value: Record<K, string>;
	/** True when the list took its view from the kept one and not from the address. */
	readonly restored: boolean;
	/** Applies at once. A change of a filter other than `page` returns the list to its first page. */
	update(patch: Partial<Record<K, string>>): void;
	/** Clears the filters and the page. The page size and the sort order stay: they are the view. */
	reset(): void;
}

/** What a filter bar needs to drop a kept value that the list no longer offers. */
export interface KeptFilters {
	prune(key: string, allowed: readonly string[]): void;
}

export const KEPT_FILTERS = 'rh-kept-filters';

const VIEW_KEYS: readonly string[] = ['size', 'sort', 'order'];
// The search text and the page are not a view. The actor and the object come from a link of a record.
const NOT_KEPT: readonly string[] = ['q', 'page', 'actor_id', 'object_id'];

/** Call during the initialization of a page component. */
export function queryFilters<K extends string>(keys: readonly K[]): QueryFilters<K> {
	const preference = `list.${page.route?.id ?? page.url.pathname}`;
	const keptKeys = keys.filter((key) => !NOT_KEPT.includes(key));
	const explicit = keys.some((key) => page.url.searchParams.has(key));

	const kept: Partial<Record<K, string>> = {};
	if (!explicit) {
		const stored = loadPreference('user', preference, isStringRecord, {});
		for (const key of keptKeys) {
			if (stored[key]) kept[key] = stored[key];
		}
	}
	const restored = Object.keys(kept).length > 0;
	const restoredKeys = new Set<string>(Object.keys(kept));

	let state = $state({ ...readQuery(page.url.searchParams, keys), ...kept });
	// The address changes after the goto call, so the effect must not read the old one meanwhile.
	let inFlight = 0;
	let settled = $state(0);
	// A call of a page during its initialization sets a default, not a choice of the person.
	let mounted = false;

	// A view that came from the kept one is not in the address yet, so only a change of the address
	// counts as a reason to read it again.
	let seen = page.url.search;

	$effect(() => {
		void settled;
		const search = page.url.search;
		if (inFlight > 0 || search === seen) return;
		seen = search;
		const fresh = readQuery(page.url.searchParams, keys);
		if (keys.some((key) => fresh[key] !== state[key])) state = fresh;
	});

	function sync(): void {
		const query = writeQuery(state, page.url.searchParams).toString();
		const address = query ? `${page.url.pathname}?${query}` : page.url.pathname;
		inFlight += 1;
		// The path comes from the current address, which is already resolved.
		// eslint-disable-next-line svelte/no-navigation-without-resolve
		void Promise.resolve(goto(address, { replace: true, reset: false })).finally(() => {
			inFlight -= 1;
			settled += 1;
		});
	}

	function persist(): void {
		const record: Record<string, string> = {};
		for (const key of keptKeys) {
			if (state[key]) record[key] = state[key];
		}
		if (Object.keys(record).length > 0) savePreference('user', preference, record);
		else removePreference('user', preference);
	}

	onMount(() => {
		mounted = true;
		if (restored) sync();
	});

	setContext<KeptFilters>(KEPT_FILTERS, {
		prune(key, allowed) {
			if (!restoredKeys.has(key)) return;
			const current = state[key as K];
			if (!current) return;
			const values = splitList(current).filter((item) => allowed.includes(item));
			const next = joinList(values);
			if (next === current) return;
			state = { ...state, [key]: next };
			sync();
			persist();
		}
	});

	return {
		get value() {
			return state;
		},
		get restored() {
			return restored;
		},
		update(patch) {
			const next = { ...state, ...patch };
			if ((keys as readonly string[]).includes('page') && !('page' in patch)) {
				(next as Record<string, string>).page = '';
			}
			for (const key of Object.keys(patch)) restoredKeys.delete(key);
			state = next;
			sync();
			if (mounted) persist();
		},
		reset() {
			const next = emptyFilters(keys);
			for (const key of keys) {
				if (VIEW_KEYS.includes(key)) next[key] = state[key];
			}
			restoredKeys.clear();
			state = next;
			sync();
			if (mounted) persist();
		}
	};
}
