import type { Locator, Page } from '@playwright/test';

/** Selects an option of a record picker, such as the host field, by its label. */
export async function pickRecord(page: Page, field: Locator, option: string): Promise<void> {
	await field.click();
	await page.getByRole('listbox').getByRole('option', { name: option, exact: true }).click();
}
