/*
 * Theme selection. The store writes the attribute `data-theme` on the document
 * element. The values are `light`, `dark` and `system`.
 */
export type Theme = 'light' | 'dark' | 'system';

const STORAGE_KEY = 'restichub.theme';

export const themes: readonly Theme[] = ['light', 'dark', 'system'];

function isTheme(value: string | null | undefined): value is Theme {
	return value === 'light' || value === 'dark' || value === 'system';
}

function detectTheme(): Theme {
	if (typeof localStorage !== 'undefined') {
		const stored = localStorage.getItem(STORAGE_KEY);
		if (isTheme(stored)) return stored;
	}
	return 'system';
}

let theme = $state<Theme>(detectTheme());

export function getTheme(): Theme {
	return theme;
}

export function setTheme(next: Theme): void {
	theme = next;
	if (typeof localStorage !== 'undefined') localStorage.setItem(STORAGE_KEY, next);
	applyTheme();
}

/** Call this after mount. */
export function applyTheme(): void {
	if (typeof document !== 'undefined') {
		document.documentElement.dataset.theme = theme;
	}
}
