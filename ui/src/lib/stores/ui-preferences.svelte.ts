/*
 * The view settings that a person chooses and the UI shows again on the next visit (FR-UI-05).
 * The value lives in the local storage of the browser as JSON. It holds view choices and ids only,
 * never a secret, so a sign-out does not clear it. A value that fails the check is removed.
 */
import { getSession } from './session.svelte.ts';

/** `user`: one tenant and one user. `browser`: the workstation, whoever signs in. */
export type PreferenceScope = 'user' | 'browser';

const PREFIX = 'restichub.v1';
// A session without a tenant is the platform one (ADR-0029): its keys need a segment as well.
const PLATFORM_TENANT = 'platform';

function storage(): Storage | null {
	try {
		return typeof localStorage === 'undefined' ? null : localStorage;
	} catch {
		// A browser with disabled storage throws on the access itself.
		return null;
	}
}

/** The storage key, or null when the scope needs a user and nobody has signed in (an API token). */
export function preferenceKey(scope: PreferenceScope, key: string): string | null {
	if (scope === 'browser') return `${PREFIX}.${key}`;
	const session = getSession();
	const user = session?.user?.id;
	if (!user) return null;
	return `${PREFIX}.${session?.tenant?.id ?? PLATFORM_TENANT}.${user}.${key}`;
}

export function removePreference(scope: PreferenceScope, key: string): void {
	const full = preferenceKey(scope, key);
	if (!full) return;
	try {
		storage()?.removeItem(full);
	} catch {
		// The view works without the stored value.
	}
}

/** The stored value when `validate` accepts it. Anything else is removed and gives the fallback. */
export function loadPreference<T>(
	scope: PreferenceScope,
	key: string,
	validate: (value: unknown) => value is T,
	fallback: T
): T {
	const full = preferenceKey(scope, key);
	const store = storage();
	if (!full || !store) return fallback;
	try {
		const raw = store.getItem(full);
		if (raw === null) return fallback;
		const value: unknown = JSON.parse(raw);
		if (validate(value)) return value;
		store.removeItem(full);
	} catch {
		removePreference(scope, key);
	}
	return fallback;
}

export function savePreference(scope: PreferenceScope, key: string, value: unknown): void {
	const full = preferenceKey(scope, key);
	if (!full) return;
	try {
		storage()?.setItem(full, JSON.stringify(value));
	} catch {
		// The quota is full or the storage is off: the choice lasts until the page closes.
	}
}

export function isStringArray(value: unknown): value is string[] {
	return Array.isArray(value) && value.every((item) => typeof item === 'string');
}

export function isStringRecord(value: unknown): value is Record<string, string> {
	return (
		typeof value === 'object' &&
		value !== null &&
		!Array.isArray(value) &&
		Object.values(value).every((item) => typeof item === 'string')
	);
}
