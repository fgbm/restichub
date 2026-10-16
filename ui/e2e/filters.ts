import type { Page } from '@playwright/test';

/** Chooses a value of a facet in the filter bar above a table. */
export async function chooseFacet(page: Page, facet: string, option: string): Promise<void> {
	await page
		.locator('.filter-bar')
		.getByRole('button', { name: new RegExp(`^${facet}`) })
		.click();
	await page.locator('.rh-popover').getByText(option, { exact: true }).click();
	await page.keyboard.press('Escape');
}
