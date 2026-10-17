/*
 * The repository that the Restore page shows. The kept choice belongs to one tenant and one user,
 * so a shared browser does not carry it to another person (FR-UI-05).
 */
import {
	loadPreference,
	savePreference,
	type PreferenceScope
} from '#lib/stores/ui-preferences.svelte.ts';

const KEY = 'restore.repository';
const SCOPE: PreferenceScope = 'user';

export function loadRepository(): string | null {
	return loadPreference<string | null>(
		SCOPE,
		KEY,
		(value): value is string => typeof value === 'string' && value !== '',
		null
	);
}

export function saveRepository(repositoryId: string): void {
	if (repositoryId) savePreference(SCOPE, KEY, repositoryId);
}

function isKnown(available: readonly { id: string }[], id: string | null): id is string {
	return Boolean(id) && available.some((item) => item.id === id);
}

/** Order: the address, then the kept choice, then the first repository. An unknown id is skipped. */
export function pickRepository(
	available: readonly { id: string }[],
	fromAddress: string | null,
	kept: string | null
): { id: string; fromAddress: boolean } {
	if (isKnown(available, fromAddress)) return { id: fromAddress, fromAddress: true };
	if (isKnown(available, kept)) return { id: kept, fromAddress: false };
	return { id: available[0]?.id ?? '', fromAddress: false };
}
