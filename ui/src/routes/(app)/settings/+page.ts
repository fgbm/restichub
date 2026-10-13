import { redirect } from '@sveltejs/kit';
import { resolve } from '$app/paths';

// Access is the only settings group with a screen today.
export function load(): never {
	redirect(307, resolve('/(app)/settings/access/users'));
}
