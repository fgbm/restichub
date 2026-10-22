import { describe, expect, it } from 'vitest';
import { latest } from './latest.ts';

describe('latest', () => {
	it('keeps the answer of the only request current', () => {
		const requests = latest();
		const current = requests.start();
		expect(current()).toBe(true);
	});

	it('drops the answer of a request that a newer request has replaced', () => {
		const requests = latest();
		const first = requests.start();
		const second = requests.start();

		expect(first()).toBe(false);
		expect(second()).toBe(true);
	});
});
