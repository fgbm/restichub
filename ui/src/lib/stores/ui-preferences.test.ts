import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Me } from '#lib/api/index.ts';
import {
	isStringArray,
	loadPreference,
	preferenceKey,
	removePreference,
	savePreference
} from './ui-preferences.svelte.ts';
import { clearSession, setSession } from './session.svelte.ts';

function signIn(user: string, tenant: string | null = 't_01'): void {
	setSession({
		user: { id: user },
		tenant: tenant ? { id: tenant } : null,
		role: 'admin'
	} as Me);
}

beforeEach(() => signIn('u_01'));

afterEach(() => {
	vi.unstubAllGlobals();
	clearSession();
});

describe('ac_ui_08_kept_view_belongs_to_one_user', () => {
	it('keys a user value by tenant and user and a browser value by itself', () => {
		expect(preferenceKey('user', 'a.b')).toBe('restichub.v1.t_01.u_01.a.b');
		expect(preferenceKey('browser', 'a.b')).toBe('restichub.v1.a.b');
	});

	it('gives a platform session its own tenant segment', () => {
		signIn('u_01', null);
		expect(preferenceKey('user', 'a')).toBe('restichub.v1.platform.u_01.a');
	});

	it('keeps a user value apart for each user and tenant, and a browser value for all', () => {
		savePreference('user', 'x', ['one']);
		savePreference('browser', 'y', 'linux');
		signIn('u_02');
		expect(loadPreference('user', 'x', isStringArray, [])).toEqual([]);
		signIn('u_01', 't_02');
		expect(loadPreference('user', 'x', isStringArray, [])).toEqual([]);
		expect(loadPreference('browser', 'y', (v): v is string => typeof v === 'string', '')).toBe(
			'linux'
		);
		signIn('u_01');
		expect(loadPreference('user', 'x', isStringArray, [])).toEqual(['one']);
	});

	it('keeps no user value without a user', () => {
		clearSession();
		savePreference('user', 'x', ['one']);
		expect(localStorage.length).toBe(0);
		expect(loadPreference('user', 'x', isStringArray, ['fallback'])).toEqual(['fallback']);
	});

	it('does not clear the kept values when the session ends', () => {
		savePreference('user', 'x', ['one']);
		clearSession();
		signIn('u_01');
		expect(loadPreference('user', 'x', isStringArray, [])).toEqual(['one']);
	});
});

describe('ac_ui_07_stale_kept_values_are_dropped', () => {
	it('removes a value that the check rejects and gives the fallback', () => {
		savePreference('user', 'x', { a: 1 });
		expect(loadPreference('user', 'x', isStringArray, ['fallback'])).toEqual(['fallback']);
		expect(localStorage.length).toBe(0);
	});

	it('removes a value that is not JSON', () => {
		localStorage.setItem('restichub.v1.t_01.u_01.x', '{not json');
		expect(loadPreference('user', 'x', isStringArray, [])).toEqual([]);
		expect(localStorage.length).toBe(0);
	});

	it('removes a value on request', () => {
		savePreference('user', 'x', ['one']);
		removePreference('user', 'x');
		expect(localStorage.length).toBe(0);
	});
});

describe('ac_ui_13_no_browser_storage', () => {
	it('gives the fallback when the storage is missing', () => {
		vi.stubGlobal('localStorage', undefined);
		savePreference('browser', 'x', ['one']);
		expect(loadPreference('browser', 'x', isStringArray, ['fallback'])).toEqual(['fallback']);
	});

	it('gives the fallback when the access to the storage throws', () => {
		const broken = {
			getItem: () => {
				throw new DOMException('denied', 'SecurityError');
			},
			setItem: () => {
				throw new DOMException('full', 'QuotaExceededError');
			},
			removeItem: () => {
				throw new DOMException('denied', 'SecurityError');
			}
		};
		vi.stubGlobal('localStorage', broken);
		savePreference('browser', 'x', ['one']);
		expect(loadPreference('browser', 'x', isStringArray, ['fallback'])).toEqual(['fallback']);
		removePreference('browser', 'x');
	});
});
