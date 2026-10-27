import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { Me } from '#lib/api/index.ts';
import { clearSession, setSession } from '#lib/stores/session.svelte.ts';
import {
	addressTokens,
	loadRepository,
	pickRepository,
	saveRepository
} from './restore-repository';

function session(tenant: string, user = 'u-1'): Me {
	return {
		platform_admin: false,
		role: 'operator',
		tenant: { id: tenant },
		user: { id: user }
	} as Me;
}

const available = [{ id: 'r-1' }, { id: 'r-2' }, { id: 'r-3' }];

beforeEach(() => setSession(session('t-1')));

afterEach(() => {
	localStorage.clear();
	clearSession();
});

// AC-UI-09: the Restore page keeps the repository.
describe('ac_ui_09_restore_repository', () => {
	it('the address wins over the kept choice', () => {
		expect(pickRepository(available, 'r-3', 'r-2')).toEqual({ id: 'r-3', fromAddress: true });
	});

	it('the kept choice wins over the first repository', () => {
		expect(pickRepository(available, null, 'r-2')).toEqual({ id: 'r-2', fromAddress: false });
	});

	it('an unknown address id falls back to the kept choice', () => {
		expect(pickRepository(available, 'gone', 'r-2').id).toBe('r-2');
	});

	it('a kept id of a deleted repository falls back to the first one', () => {
		saveRepository('deleted');
		expect(pickRepository(available, null, loadRepository()).id).toBe('r-1');
	});

	it('an empty list gives no repository', () => {
		expect(pickRepository([], 'r-1', 'r-2').id).toBe('');
	});

	it('each tenant keeps its own choice', () => {
		saveRepository('r-2');
		setSession(session('t-2'));
		expect(loadRepository()).toBeNull();
		saveRepository('r-3');
		setSession(session('t-1'));
		expect(loadRepository()).toBe('r-2');
	});

	it('each user in a tenant keeps its own choice', () => {
		saveRepository('r-2');
		setSession(session('t-1', 'u-2'));
		expect(loadRepository()).toBeNull();
	});

	it('a value that is not a text is removed', () => {
		localStorage.setItem('restichub.v1.t-1.u-1.restore.repository', '{"x":1}');
		expect(loadRepository()).toBeNull();
		expect(localStorage.length).toBe(0);
	});
});

// AC-UI-09: a late completion of a repository change must not change the address of the page the
// person is on now.
describe('ac_ui_09_a_late_address_update_stays_off_the_restore_page', () => {
	it('a newer request makes the answer of an older one stale', () => {
		const tokens = addressTokens();
		const first = tokens.begin();
		const second = tokens.begin();
		expect(tokens.isCurrent(first)).toBe(false);
		expect(tokens.isCurrent(second)).toBe(true);
	});

	it('leaving the page makes the pending answer stale', () => {
		const tokens = addressTokens();
		const token = tokens.begin();
		tokens.close();
		expect(tokens.isCurrent(token)).toBe(false);
	});

	it('the answer of the latest request stays current', () => {
		const tokens = addressTokens();
		const token = tokens.begin();
		expect(tokens.isCurrent(token)).toBe(true);
		expect(tokens.isCurrent(token + 1)).toBe(false);
	});
});
