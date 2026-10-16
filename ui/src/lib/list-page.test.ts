import { describe, expect, it } from 'vitest';
import { DEFAULT_PAGE_SIZE, lastPageIfPast, pageNumber, pageSizeOf } from './list-page';

describe('the page window of a server list', () => {
	it('reads the page and the page size from the query string', () => {
		expect(pageNumber('')).toBe(1);
		expect(pageNumber('0')).toBe(1);
		expect(pageNumber('abc')).toBe(1);
		expect(pageNumber('3')).toBe(3);
		expect(pageSizeOf('50')).toBe(50);
		expect(pageSizeOf('7')).toBe(DEFAULT_PAGE_SIZE);
	});

	it('fr_ui_01 moves an empty page past the end to the last page with rows', () => {
		expect(lastPageIfPast(3, 25, 40, 0)).toBe(2);
		expect(lastPageIfPast(9, 10, 5, 0)).toBe(1);
		expect(lastPageIfPast(3, 25, 60, 10)).toBeNull();
		expect(lastPageIfPast(1, 25, 40, 0)).toBeNull();
		expect(lastPageIfPast(3, 25, 0, 0)).toBeNull();
	});
});
