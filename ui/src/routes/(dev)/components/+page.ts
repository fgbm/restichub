import { dev } from '$app/env';
import { error } from '@sveltejs/kit';
import type { PageLoad } from './$types';

// The component gallery is a development tool. It is not part of a production build.
export const load: PageLoad = () => {
	if (!dev) error(404, 'Not found');
};
