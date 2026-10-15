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

// The screens whose mock data holds rows, so the check does not pass on an empty screen.
const WITH_DATA = new Set(['/', '/jobs', '/hosts', '/plans', '/alerts', '/audit']);

async function signIn(page: Page): Promise<void> {
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: 'Основная навигация' })).toBeVisible();
}

async function wrapped(page: Page): Promise<string[]> {
	return page.evaluate(() => {
		// A reason text may wrap by design; every other text of a nowrap cell may not.
		const texts: Text[] = [];
		const add = (root: Element) => {
			const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
			for (let node = walker.nextNode(); node; node = walker.nextNode()) {
				const text = node as Text;
				if (!text.data.trim() || text.parentElement?.closest('.job__reason')) continue;
				texts.push(text);
			}
		};
		for (const element of document.querySelectorAll(
			'time, .status, td.table__cell--nowrap, .about__time'
		)) {
			add(element);
		}
		const found = new Set<string>();
		for (const text of texts) {
			const tops = new Set<number>();
			const range = document.createRange();
			range.selectNodeContents(text);
			for (const rect of range.getClientRects()) {
				if (rect.width > 0) tops.add(Math.round(rect.top));
			}
			if (tops.size > 1) found.add(text.data.trim().slice(0, 60));
		}
		return [...found];
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
			// A reload drops the mock session, so the test follows the links of the shell.
			if (path.startsWith('/settings/')) await page.locator('a[href="/settings"]').first().click();
			await page.locator(`a[href="${path}"]`).first().click();
			await expect(page).toHaveURL(new RegExp(`${path === '/' ? '/$' : path}$`));
			await page.getByRole('heading', { level: 1 }).waitFor();
			await page.waitForTimeout(400);
			if (WITH_DATA.has(path)) {
				const cells = await page.locator('td, .row').count();
				expect(cells, `${path} shows no data`).toBeGreaterThan(0);
			}
			expect(await wrapped(page), `${path} at ${width} px`).toEqual([]);
			if (path === '/jobs') {
				await page
					.getByRole('link', { name: /Открыть задание/ })
					.first()
					.click();
				await expect(page.locator('.about__time').first()).toBeVisible();
				expect(await wrapped(page), `a job detail at ${width} px`).toEqual([]);
				await page.goBack();
			}
		}
	});
}
