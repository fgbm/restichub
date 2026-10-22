/*
 * The address of a screen test. SvelteKit keeps one `page` object and replaces its parts on a
 * navigation, so a test can move a rendered screen to another record.
 */
import { SvelteURL } from 'svelte/reactivity';

export const testPage = $state<{ params: Record<string, string>; url: URL }>({
	params: {},
	url: new SvelteURL('http://localhost/')
});
