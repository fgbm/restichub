import { afterEach, describe, expect, it, vi } from 'vitest';
import { BrowseTimeout, settle } from './restore-poll';

afterEach(() => vi.useRealTimers());

describe('settle (FR-RS-02)', () => {
	it('fr_rs_02_asks_again_until_the_state_is_not_pending', async () => {
		const read = vi
			.fn<(view: { state: 'pending' | 'done' }) => Promise<{ state: 'pending' | 'done' }>>()
			.mockResolvedValueOnce({ state: 'pending' })
			.mockResolvedValueOnce({ state: 'done' });
		const result = await settle({ state: 'pending' as const }, read, { interval: 1 });
		expect(result?.state).toBe('done');
		expect(read).toHaveBeenCalledTimes(2);
	});

	it('fr_rs_02_returns_at_once_for_a_stored_page', async () => {
		const read = vi.fn();
		const result = await settle({ state: 'done' as const }, read);
		expect(result?.state).toBe('done');
		expect(read).not.toHaveBeenCalled();
	});

	it('fr_rs_02_stops_when_the_caller_cancels', async () => {
		const read = vi.fn();
		const result = await settle({ state: 'pending' as const }, read, {
			interval: 1,
			cancelled: () => true
		});
		expect(result).toBeNull();
		expect(read).not.toHaveBeenCalled();
	});

	it('fr_rs_02_gives_up_after_the_timeout', async () => {
		const read = async () => ({ state: 'pending' as const });
		await expect(
			settle({ state: 'pending' as const }, read, { interval: 1, timeout: 5 })
		).rejects.toBeInstanceOf(BrowseTimeout);
	});
});
