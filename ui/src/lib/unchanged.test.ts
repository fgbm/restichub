import { describe, expect, it } from 'vitest';
import { keepUnchanged, sameData } from './unchanged.ts';

const key = (item: { id: string }) => item.id;

describe('keepUnchanged', () => {
	it('gives the previous array when no item changed', () => {
		const previous = [
			{ id: 'a', status: 'running' },
			{ id: 'b', status: 'succeeded' }
		];
		const next = structuredClone(previous);
		expect(keepUnchanged(previous, next, key)).toBe(previous);
	});

	it('keeps the objects of the unchanged items and takes the changed ones', () => {
		const previous = [
			{ id: 'a', status: 'running' },
			{ id: 'b', status: 'queued' }
		];
		const next = [
			{ id: 'a', status: 'running' },
			{ id: 'b', status: 'running' }
		];
		const merged = keepUnchanged(previous, next, key);
		expect(merged).not.toBe(previous);
		expect(merged[0]).toBe(previous[0]);
		expect(merged[1]).toBe(next[1]);
	});

	it('follows the order and the items of the response', () => {
		const previous = [
			{ id: 'a', status: 'running' },
			{ id: 'b', status: 'queued' }
		];
		const next = [
			{ id: 'c', status: 'queued' },
			{ id: 'a', status: 'running' }
		];
		const merged = keepUnchanged(previous, next, key);
		expect(merged.map(key)).toEqual(['c', 'a']);
		expect(merged[1]).toBe(previous[0]);
	});

	it('takes a live item from the response also when it is unchanged', () => {
		const previous = [
			{ id: 'a', status: 'running' },
			{ id: 'b', status: 'succeeded' }
		];
		const next = structuredClone(previous);
		const merged = keepUnchanged(previous, next, key, (item) => item.status === 'running');
		expect(merged[0]).toBe(next[0]);
		expect(merged[1]).toBe(previous[1]);
	});
});

describe('sameData', () => {
	it('compares the data, not the objects', () => {
		expect(sameData({ all: { count: 3 } }, { all: { count: 3 } })).toBe(true);
		expect(sameData({ all: { count: 3 } }, { all: { count: 4 } })).toBe(false);
		expect(sameData(null, { all: { count: 3 } })).toBe(false);
	});
});
