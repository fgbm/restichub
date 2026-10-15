/*
 * The title of the current detail page. The top bar shows it as the last part of the path, for
 * a page whose name the route does not hold, such as a profile that the page loads.
 */
let title = $state<string | null>(null);

export function getPageTitle(): string | null {
	return title;
}

export function setPageTitle(next: string | null): void {
	title = next;
}
