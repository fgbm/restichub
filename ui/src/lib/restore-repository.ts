/*
 * The last repository that a person chose on the Restore page. The key holds the tenant and
 * the user, so a shared browser does not carry the choice to another tenant or person.
 */
import type { Me } from '#lib/api/index.ts';

const PREFIX = 'restichub.restore.repository';

export function storageKey(session: Me | null): string | null {
	const tenant = session?.tenant?.id;
	if (!tenant) return null;
	const user = session?.user?.id;
	return user ? `${PREFIX}.${tenant}.${user}` : `${PREFIX}.${tenant}`;
}

export function loadRepository(session: Me | null): string | null {
	const key = storageKey(session);
	if (!key || typeof localStorage === 'undefined') return null;
	return localStorage.getItem(key);
}

export function saveRepository(session: Me | null, repositoryId: string): void {
	const key = storageKey(session);
	if (!key || !repositoryId || typeof localStorage === 'undefined') return;
	localStorage.setItem(key, repositoryId);
}

/** Order: the address, then the stored choice, then the first repository. An unknown id is skipped. */
export function pickRepository(
	available: readonly { id: string }[],
	fromAddress: string | null,
	stored: string | null
): string {
	for (const candidate of [fromAddress, stored]) {
		if (candidate && available.some((item) => item.id === candidate)) return candidate;
	}
	return available[0]?.id ?? '';
}
