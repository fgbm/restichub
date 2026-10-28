import { expect, test, type Page } from '@playwright/test';

// The support text is short, so the whole text is visible: it never scrolls in its own box.

async function openAbout(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.getByRole('button', { name: 'Sign in', exact: true }).click();
	await expect(page.getByRole('navigation', { name: 'Primary navigation' })).toBeVisible();
	// The mock build keeps the session in memory, so the screen opens from the account menu.
	await page.locator('.rh-menu__trigger').click();
	await page.getByRole('menuitem', { name: 'About ResticHub' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'About ResticHub' })).toBeVisible();
}

for (const width of [1280, 360]) {
	test(`the support text shows all its lines at ${width} px`, async ({ page }) => {
		await page.setViewportSize({ width, height: 900 });
		await openAbout(page);
		const panel = page.locator('section.panel', {
			has: page.getByRole('heading', { name: 'Support information' })
		});
		const text = panel.locator('.panel__body').locator('pre, textarea');
		await expect(text).toBeVisible();
		const box = await text.evaluate((element) => ({
			text: element instanceof HTMLTextAreaElement ? element.value : element.textContent,
			scrollHeight: element.scrollHeight,
			clientHeight: element.clientHeight,
			scrollWidth: element.scrollWidth,
			clientWidth: element.clientWidth
		}));
		expect(box.text).toContain('organizations:');
		expect(box.scrollHeight, 'the support text scrolls vertically').toBeLessThanOrEqual(
			box.clientHeight
		);
		expect(box.scrollWidth, 'the support text scrolls horizontally').toBeLessThanOrEqual(
			box.clientWidth
		);
		await expect(
			panel.locator('.panel__head').getByRole('button', { name: 'Copy support text' })
		).toBeVisible();
	});
}
