import { expect, test, type Page } from '@playwright/test';

// A settings page is a page: its heading and its body keep the spacing of the list of hosts.

async function signIn(page: Page): Promise<void> {
	await page.addInitScript(() => localStorage.setItem('restichub.mock.agents', 'sample'));
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

// A reload drops the mock session, so the test follows the links of the shell.
async function openSettingsList(page: Page, path: string): Promise<void> {
	await page.locator('a[href="/settings"]').first().click();
	await page.locator(`a[href="${path}"]`).first().click();
	await expect(page.locator('li.item').first()).toBeVisible();
}

async function headingGap(page: Page): Promise<number> {
	return page.evaluate(() => {
		const heading = document.querySelector('h1')?.getBoundingClientRect();
		const block = document.querySelector('.list-view')?.getBoundingClientRect();
		return Math.round((block?.top ?? 0) - (heading?.bottom ?? 0));
	});
}

test('a_settings_page_keeps_the_spacing_between_its_heading_and_its_body', async ({ page }) => {
	await page.setViewportSize({ width: 1280, height: 900 });
	await signIn(page);

	await openSettingsList(page, '/settings/scripts/templates');
	const settings = await headingGap(page);

	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.getByRole('link', { name: 'Hosts', exact: true })
		.click();
	await expect(page.getByRole('heading', { level: 1, name: 'Hosts' })).toBeVisible();
	const hosts = await headingGap(page);

	expect(settings, 'the settings page spaces its heading differently from the hosts list').toBe(
		hosts
	);
});
