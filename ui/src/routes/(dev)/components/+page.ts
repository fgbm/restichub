import { dev } from '$app/env';
import { error } from '@sveltejs/kit';
import type { PageLoad } from './$types';

// The gallery is a development tool. A mock build (the e2e build) keeps it for the CSP and
// accessibility checks; a production build answers 404.
export const load: PageLoad = () => {
	if (!dev && import.meta.env.VITE_USE_MOCK !== 'true') error(404, 'Not found');
};
