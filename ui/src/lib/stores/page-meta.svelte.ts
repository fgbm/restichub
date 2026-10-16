/*
 * The breadcrumb levels that the current page declares below its section, and the last query string
 * of each list. A parent level links to its list with that query, so the list opens with the same
 * filters and page.
 */
import { SvelteMap } from 'svelte/reactivity';

export interface Crumb {
	label: string;
	/** A resolved path, with a query string when the level keeps a list state. */
	href?: string;
}

export interface PageMeta {
	/** The levels below the section; the last one is the current page. */
	crumbs: Crumb[];
	/** The query of the section link. Default: the last query of that list in this tab. */
	listQuery?: string;
}

let meta = $state<PageMeta | null>(null);
let owner: object | null = null;
const listQueries = new SvelteMap<string, string>();

/** The levels of the current page, or `null` when the page declares none. */
export function getPageMeta(): PageMeta | null {
	return meta;
}

/** Sets the levels for `by`, the page that declares them. */
export function setPageMeta(next: PageMeta, by: object): void {
	meta = next;
	owner = by;
}

/** Clears the levels, unless a newer page has set its own already. */
export function clearPageMeta(by: object): void {
	if (owner !== by) return;
	meta = null;
	owner = null;
}

/** Records the query string of an address, without the leading `?`. */
export function rememberListQuery(pathname: string, search: string): void {
	const query = search.startsWith('?') ? search.slice(1) : search;
	if (query) listQueries.set(pathname, query);
	else listQueries.delete(pathname);
}

/** The address of a list with the given query, or else with its last query string. */
export function listHref(pathname: string, query?: string): string {
	const kept = query ?? listQueries.get(pathname) ?? '';
	return kept ? `${pathname}?${kept}` : pathname;
}

export function forgetListQueries(): void {
	listQueries.clear();
}
