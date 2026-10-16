import type { Page } from '@playwright/test';

/** Chooses a value of a facet in the filter bar above a table or a list. */
export async function chooseFacet(page: Page, facet: string, option: string): Promise<void> {
	await page
		.locator('.filter-bar, .list-head')
		.getByRole('button', { name: new RegExp(`^${facet}`) })
		.click();
	await page.locator('.rh-popover').getByText(option, { exact: true }).click();
	await page.keyboard.press('Escape');
}

/** Chooses a state tab in the header of a list. */
export async function chooseTab(page: Page, tab: string): Promise<void> {
	await page.locator('.list-head .tab').filter({ hasText: tab }).first().click();
}
