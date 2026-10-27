/*
 * Session state: the current user, the tenant and the role.
 */
import { service } from '#lib/api/index.ts';
import type { Me, Role } from '#lib/api/index.ts';
import { forgetAbout } from '#lib/stores/about.svelte.ts';

let current = $state<Me | null>(null);
let loading = $state(false);
let loaded = $state(false);

export function getSession(): Me | null {
	return current;
}

export function isLoading(): boolean {
	return loading;
}

export function isLoaded(): boolean {
	return loaded;
}

export function setSession(value: Me | null): void {
	current = value;
	loaded = true;
}

export function clearSession(): void {
	current = null;
	loaded = true;
	forgetAbout();
}

export async function loadSession(): Promise<Me | null> {
	loading = true;
	try {
		current = await service.currentUser();
	} catch {
		current = null;
	} finally {
		loading = false;
		loaded = true;
	}
	return current;
}

export function getRole(): Role | null {
	return current?.role ?? null;
}

export function hasRole(...roles: Role[]): boolean {
	if (!current) return false;
	return roles.includes(current.role);
}
