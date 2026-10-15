import { expect, test, type Page } from '@playwright/test';

// Dates, relative times, statuses and durations stay on one line at the common desktop widths.
// The check reads the line boxes of each element, so a wrapped value shows as two rectangles.
const SCREENS = [
	'/',
	'/jobs',
	'/hosts',
	'/plans',
	'/profiles',
	'/repositories',
	'/restore',
	'/alerts',
	'/audit',
	'/settings/access/api-tokens',
	'/settings/notifications/channels'
];

async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: 'Основная навигация' })).toBeVisible();
}

async function wrapped(page: Page): Promise<string[]> {
	return page.evaluate(() => {
		const selector = 'time, .status, td.table__cell--nowrap, .table__cell--nowrap > *';
		const found: string[] = [];
		for (const element of document.querySelectorAll(selector)) {
			const lines = new Set<number>();
			const range = document.createRange();
			range.selectNodeContents(element);
			for (const rect of range.getClientRects()) {
				if (rect.width > 0) lines.add(Math.round(rect.top));
			}
			if (lines.size > 1) found.push((element.textContent ?? '').trim().slice(0, 60));
		}
		return found;
	});
}

for (const width of [1440, 1280]) {
	test(`no date, time or status wraps at ${width} px`, async ({ page }) => {
		await page.addInitScript(() => {
			localStorage.setItem('restichub.mock.agents', 'sample');
			localStorage.setItem('restichub.mock.profiles', 'sample');
			localStorage.setItem('restichub.mock.plans', 'sample');
			localStorage.setItem('restichub.locale', 'ru');
		});
		await page.setViewportSize({ width, height: 900 });
		await signIn(page);
		for (const path of SCREENS) {
			await page.goto(path);
			await page.getByRole('heading', { level: 1 }).waitFor();
			await page.waitForTimeout(300);
			expect(await wrapped(page), `${path} at ${width} px`).toEqual([]);
		}
	});
}
