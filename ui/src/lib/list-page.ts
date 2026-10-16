/*
 * The page window of a list that pages on the server (design/style/ui-principles.md, 2.14). The
 * page and the page size live in the query string with the filters.
 */

export const PAGE_SIZES = [10, 25, 50, 100];
export const DEFAULT_PAGE_SIZE = 25;

export function pageNumber(value: string): number {
	return Math.max(1, Math.floor(Number(value)) || 1);
}

export function pageSizeOf(value: string): number {
	const size = Number(value);
	return PAGE_SIZES.includes(size) ? size : DEFAULT_PAGE_SIZE;
}

/**
 * The last page with rows when the shown page came back empty past the end of the list, for
 * example after a refresh that shrank it; `null` when the page is in range.
 */
export function lastPageIfPast(
	page: number,
	pageSize: number,
	total: number,
	rows: number
): number | null {
	if (rows > 0 || total === 0 || page <= 1) return null;
	const last = Math.max(1, Math.ceil(total / pageSize));
	return last < page ? last : null;
}
