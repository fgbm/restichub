/*
 * The path in the top bar: the section levels from the navigation, then the levels that the page
 * declares. The breadcrumb is the only way back from a page (UI principles 1.1), so each level above
 * the current one is a link, and the list level keeps the list state (UI principles 1.2).
 */
import { listHref, type Crumb, type PageMeta } from '#lib/stores/page-meta.svelte.ts';

export type { Crumb };

export function buildBreadcrumb(section: readonly Crumb[], meta: PageMeta | null): Crumb[] {
	const levels = (meta?.crumbs ?? []).filter((crumb) => crumb.label !== '');
	if (levels.length === 0) {
		return section.map((crumb, index) =>
			index === section.length - 1 ? { label: crumb.label } : crumb
		);
	}
	const parents = section.map((crumb, index) =>
		index === section.length - 1 && crumb.href
			? { label: crumb.label, href: listHref(crumb.href, meta?.listQuery) }
			: crumb
	);
	return [
		...parents,
		...levels.map((crumb, index) => (index === levels.length - 1 ? { label: crumb.label } : crumb))
	];
}
