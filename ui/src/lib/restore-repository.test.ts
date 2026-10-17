import { afterEach, describe, expect, it } from 'vitest';
import type { Me } from '#lib/api/index.ts';
import { loadRepository, pickRepository, saveRepository, storageKey } from './restore-repository';

function session(tenant: string, user = 'u-1'): Me {
	return {
		platform_admin: false,
		role: 'operator',
		tenant: { id: tenant },
		user: { id: user }
	} as Me;
}

const available = [{ id: 'r-1' }, { id: 'r-2' }, { id: 'r-3' }];

afterEach(() => {
	localStorage.clear();
});

describe('last repository of the Restore page', () => {
	it('the address wins over the stored choice', () => {
		expect(pickRepository(available, 'r-3', 'r-2')).toBe('r-3');
	});

	it('the stored choice wins over the first repository', () => {
		expect(pickRepository(available, null, 'r-2')).toBe('r-2');
	});

	it('an unknown address id falls back to the stored choice', () => {
		expect(pickRepository(available, 'gone', 'r-2')).toBe('r-2');
	});

	it('a stored id of a deleted repository falls back to the first one', () => {
		const owner = session('t-1');
		saveRepository(owner, 'deleted');
		expect(pickRepository(available, null, loadRepository(owner))).toBe('r-1');
	});

	it('an empty list gives no repository', () => {
		expect(pickRepository([], 'r-1', 'r-2')).toBe('');
	});

	it('each tenant keeps its own choice', () => {
		saveRepository(session('t-1'), 'r-2');
		saveRepository(session('t-2'), 'r-3');
		expect(loadRepository(session('t-1'))).toBe('r-2');
		expect(loadRepository(session('t-2'))).toBe('r-3');
		expect(loadRepository(session('t-3'))).toBeNull();
	});

	it('each user in a tenant keeps its own choice', () => {
		saveRepository(session('t-1', 'u-1'), 'r-2');
		expect(loadRepository(session('t-1', 'u-2'))).toBeNull();
		expect(storageKey(session('t-1', 'u-1'))).toBe('restichub.restore.repository.t-1.u-1');
	});

	it('a session without a tenant stores nothing', () => {
		const none = { platform_admin: true, role: 'admin' } as Me;
		saveRepository(none, 'r-2');
		expect(localStorage.length).toBe(0);
		expect(loadRepository(none)).toBeNull();
	});
});
