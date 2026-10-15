import { redirect } from '@sveltejs/kit';
import { resolve } from '$app/paths';

// The secrets screen lives under Settings, Access; the short address keeps working.
export function load(): never {
	redirect(308, resolve('/(app)/settings/access/secrets'));
}
