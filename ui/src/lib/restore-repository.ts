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

/** One address write. A write may finish after a newer one started or after the page went away. */
export interface AddressTokens {
	/** Starts a newer write. Each earlier token is stale from now on. */
	begin(): number;
	isCurrent(token: number): boolean;
	/** The page is gone: no token is current any more. */
	close(): void;
}

/**
 * Keeps a late completion of a repository change off the address of the page the person is on now
 * (AC-UI-09, FR-UI-05).
 */
export function addressTokens(): AddressTokens {
	let current = 0;
	let closed = false;
	return {
		begin() {
			current += 1;
			return current;
		},
		isCurrent(token) {
			return !closed && token === current;
		},
		close() {
			closed = true;
		}
	};
}
