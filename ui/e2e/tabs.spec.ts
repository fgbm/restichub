import { expect, test, type Page } from '@playwright/test';

// The tab list scrolls horizontally on a narrow viewport. It must not scroll vertically: on
// classic scrollbars a 1px vertical overflow adds a scrollbar with arrows at the right edge.
async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
}

for (const viewport of [
	{ width: 1920, height: 1080 },
	{ width: 1280, height: 800 }
]) {
	test(`the tab bar has no vertical scrollbar at ${viewport.width}x${viewport.height}`, async ({
		page
	}) => {
		await page.setViewportSize(viewport);
		await signIn(page);

		const nav = page.getByRole('navigation', { name: 'Primary navigation' });
		const fits = await nav.evaluate((element) => element.scrollHeight <= element.clientHeight);
		expect(fits).toBe(true);
	});
}
