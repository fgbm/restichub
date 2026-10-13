import { redirect } from '@sveltejs/kit';
import { resolve } from '$app/paths';
import { platformRedirectFor } from '#lib/navigation.ts';
import { isInstanceLoaded, isSingleTenant, loadInstance } from '#lib/stores/instance.svelte.ts';
import { getSession, isLoaded, loadSession } from '#lib/stores/session.svelte.ts';
import type { LayoutLoad } from './$types';

// The session and the instance load before a page runs its data, so an organization route never
// fetches data for an actor that cannot see it: an unauthorized visitor reaches the sign-in
// screen and a Platform Admin without a current organization reaches the platform mode
// (ADR-0029) without an error screen.
export const load: LayoutLoad = async ({ url }) => {
	if (!isInstanceLoaded()) await loadInstance();
	if (!isLoaded()) await loadSession();
	const session = getSession();
	if (!session) redirect(307, resolve('/(auth)/sign-in'));
	const target = platformRedirectFor(session, isSingleTenant(), url.pathname);
	if (target) redirect(307, resolve(target));
};
