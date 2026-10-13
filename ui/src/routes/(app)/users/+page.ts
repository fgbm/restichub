import { redirect } from '@sveltejs/kit';
import { resolve } from '$app/paths';

// The users screen moved under Settings, Access; old links keep working.
export function load(): never {
	redirect(308, resolve('/(app)/settings/access/users'));
}
