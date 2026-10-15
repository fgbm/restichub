import { expect, test } from '@playwright/test';

// A table grows to its content and the page scrolls. The table has no inner vertical scroll.
test('a long table has no inner scroll and the page scrolls', async ({ page }) => {
	await page.setViewportSize({ width: 1440, height: 500 });
	await page.addInitScript(() => {
		localStorage.setItem('restichub.mock.agents', 'sample');
		localStorage.setItem('restichub.mock.profiles', 'sample');
		localStorage.setItem('restichub.mock.plans', 'sample');
		localStorage.setItem('restichub.mock.scripts', 'sample');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
	await page
		.getByRole('navigation', { name: 'Primary navigation' })
		.locator('a[href="/settings"]')
		.first()
		.click();
	await page.locator('a[href="/settings/scripts/templates"]').click();
	await expect(page.getByRole('heading', { level: 1 })).toBeVisible();

	const scroll = page.locator('.table-scroll').first();
	await expect(scroll.locator('tbody tr').nth(5)).toBeVisible();
	const sizes = await scroll.evaluate((element) => ({
		scrollHeight: element.scrollHeight,
		clientHeight: element.clientHeight
	}));
	expect(sizes.scrollHeight).toBe(sizes.clientHeight);

	const pageScrolls = await page.evaluate(() => {
		const root = document.scrollingElement as Element;
		const main = document.querySelector('main');
		return (
			root.scrollHeight > root.clientHeight ||
			(main !== null && main.scrollHeight > main.clientHeight)
		);
	});
	expect(pageScrolls).toBe(true);
});
