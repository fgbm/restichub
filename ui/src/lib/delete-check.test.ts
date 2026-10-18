import { describe, expect, it, vi } from 'vitest';
import { ApiError } from '#lib/api/errors.ts';
import { blockedView, fromRefusal, fromUsage } from './delete-check';

vi.mock('$app/paths', () => ({
	resolve: (path: string, params?: Record<string, string>) =>
		params ? path.replace('[id]', params.id) : path
}));

const usage = {
	blocked_by: 'repository_in_use' as const,
	blockers: {
		items: [
			{ kind: 'plan' as const, id: 'p1', name: 'Alpha' },
			{ kind: 'plan' as const, id: 'p2', name: 'Beta' }
		],
		total: 12
	}
};

describe('delete check', () => {
	it('builds the reason, the links and the count of the hidden records', () => {
		const view = fromUsage(usage);
		expect(view?.code).toBe('repository_in_use');
		expect(view?.reason).toMatch(/Plans use this repository/);
		expect(view?.links.map((link) => link.label)).toEqual(['Alpha', 'Beta']);
		expect(view?.links[0].href).toBe('/(app)/plans/p1');
		expect(view?.links[0].kind).toBe('Plan');
		expect(view?.more).toBe(10);
	});

	it('returns null when the delete is allowed', () => {
		expect(fromUsage({ blocked_by: null, blockers: { items: [], total: 0 } })).toBeNull();
	});

	it('reads the list of a refused delete', () => {
		const error = new ApiError({
			status: 409,
			code: 'profile_in_use',
			title: 'Conflict',
			blockers: { items: [{ kind: 'plan', id: 'pl1', name: 'Nightly' }], total: 1 }
		});
		const view = fromRefusal(error);
		expect(view?.code).toBe('profile_in_use');
		expect(view?.links[0].href).toBe('/(app)/plans/pl1');
		expect(view?.more).toBe(0);
	});

	it('ignores a refusal without a list and any other failure', () => {
		expect(
			fromRefusal(new ApiError({ status: 409, code: 'conflict', title: 'Conflict' }))
		).toBeNull();
		expect(fromRefusal(new Error('network'))).toBeNull();
	});

	it('keeps the code when no text exists', () => {
		expect(blockedView('unknown_code', undefined).reason).toBe('unknown_code');
	});
});
