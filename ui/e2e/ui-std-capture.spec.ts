import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

// Captures each screen on the shared layout and table components for a review against
// design/style/ui-principles.md. It runs only with CAPTURE and CAPTURE_DIR set, so CI skips it.
const dir = process.env.CAPTURE_DIR;
test.skip(!process.env.CAPTURE || !dir, 'set CAPTURE and CAPTURE_DIR to write the screenshots');

const out = dir as string;

const LISTS: [string, string][] = [
	['/', 'overview'],
	['/hosts', 'hosts'],
	['/repositories', 'repositories'],
	['/profiles', 'profiles'],
	['/plans', 'plans'],
	['/jobs', 'jobs'],
	['/restore', 'restore'],
	['/alerts', 'alerts'],
	['/audit', 'audit'],
	['/settings/access/users', 'settings-users'],
	['/settings/access/api-tokens', 'settings-tokens'],
	['/settings/access/secrets', 'settings-secrets'],
	['/settings/notifications/channels', 'settings-channels'],
	['/settings/scripts/templates', 'settings-script-templates'],
	['/settings/scripts/jobs', 'settings-job-templates']
];

async function signIn(page: Page, width: number): Promise<void> {
	mkdirSync(out, { recursive: true });
	await page.setViewportSize({ width, height: width < 600 ? 844 : 900 });
	await page.emulateMedia({ colorScheme: 'light' });
	await page.addInitScript(() => {
		for (const key of ['agents', 'repositories', 'profiles', 'plans', 'alerts', 'scripts']) {
			localStorage.setItem(`restichub.mock.${key}`, 'sample');
		}
		localStorage.setItem('restichub.mock.snapshots', 'many');
		localStorage.setItem('restichub.theme', 'light');
		localStorage.setItem('restichub.locale', 'ru');
	});
	await page.goto('/sign-in');
	await page.locator('#signin-username').fill('admin');
	await page.locator('#signin-password').fill('secret');
	await page.locator('form button[type="submit"]').click();
	await expect(page.getByRole('navigation', { name: 'Основная навигация' })).toBeVisible();
}

async function open(page: Page, path: string): Promise<void> {
	// A reload drops the mock session, so the capture follows the links of the shell.
	const nav = page.getByRole('navigation', { name: 'Основная навигация' });
	if (path.startsWith('/settings/')) await nav.locator('a[href="/settings"]').click();
	await page.locator(`a[href="${path}"]:visible`).first().click();
	await page.getByRole('heading', { level: 1 }).waitFor();
	await page.waitForTimeout(500);
}

async function shot(page: Page, name: string, width: number): Promise<void> {
	await page.screenshot({ path: `${out}/${name}-${width}.png`, fullPage: true });
}

async function overflow(page: Page): Promise<number> {
	return page.evaluate(() => {
		const scrollers = [document.documentElement, ...document.querySelectorAll('.table-scroll')];
		return Math.max(...scrollers.map((item) => item.scrollWidth - item.clientWidth));
	});
}

for (const width of [1440, 390]) {
	test(`captures the lists and the record pages at ${width}`, async ({ page }) => {
		test.setTimeout(600_000);
		await signIn(page, width);
		const wide: string[] = [];
		for (const [path, name] of LISTS) {
			await open(page, path);
			await shot(page, name, width);
			if (width === 1440 && (await overflow(page)) > 0) wide.push(name);
		}

		await open(page, '/hosts');
		await page.locator('main a.item__title, main a.row-link').first().click();
		await page.getByRole('heading', { level: 1 }).waitFor();
		await page.waitForTimeout(500);
		await shot(page, 'host-page', width);

		await open(page, '/repositories');
		await page.locator('main a.item__title, main a.row-link').first().click();
		await page.locator('#repository-name, #rp-name, form').first().waitFor();
		await page.waitForTimeout(500);
		await shot(page, 'repository-page', width);

		await open(page, '/profiles');
		await page.locator('main a.item__title, main a.row-link').first().click();
		await page.locator('#profile-name').waitFor();
		await page.waitForTimeout(300);
		await shot(page, 'profile-page', width);

		await open(page, '/settings/scripts/templates');
		await page.locator('main a.item__title').first().click();
		await page.locator('#template-name').waitFor();
		await page.waitForTimeout(400);
		await shot(page, 'script-template-page', width);

		await open(page, '/settings/scripts/templates');
		await page.locator('.list-head .tab').filter({ hasText: 'Встроенные' }).click();
		await page.locator('main a.item__title').first().click();
		await page.locator('#template-name').waitFor();
		await page.waitForTimeout(400);
		await shot(page, 'script-template-builtin-page', width);

		await open(page, '/settings/scripts/jobs');
		await page.locator('main a.item__title').first().click();
		await page.locator('#job-template-name').waitFor();
		await page.waitForTimeout(400);
		await shot(page, 'job-template-page', width);

		await open(page, '/jobs');
		await page.locator('main a.item__title, main a.row-link').first().click();
		await page.getByRole('heading', { level: 1 }).waitFor();
		await page.waitForTimeout(500);
		await shot(page, 'job-page', width);

		await open(page, '/restore');
		await page
			.getByRole('link', { name: /^Открыть снимок / })
			.first()
			.click();
		await page.getByRole('tree').waitFor();
		await page.waitForTimeout(500);
		await shot(page, 'snapshot-page', width);

		await open(page, '/alerts');
		await page.locator('main button.item__title, main button.row-link').first().click();
		await page.getByRole('dialog').waitFor();
		await page.waitForTimeout(400);
		await page.screenshot({ path: `${out}/alerts-detail-${width}.png` });
		await page.keyboard.press('Escape');

		await open(page, '/settings/access/api-tokens');
		await page.locator('main button.item__title, main button.row-link').first().click();
		await page.getByRole('dialog').waitFor();
		await page.waitForTimeout(400);
		await page.screenshot({ path: `${out}/settings-tokens-detail-${width}.png` });
		await page.keyboard.press('Escape');

		await open(page, '/jobs');
		await page.locator('.list-head .tab').filter({ hasText: 'Не выполнены' }).click();
		await page.waitForTimeout(400);
		await shot(page, 'jobs-filtered', width);

		await page.getByRole('button', { name: /Сменить организацию/ }).click();
		await page.getByRole('menuitem', { name: 'Администрирование платформы' }).click();
		await page.getByRole('heading', { level: 1 }).waitFor();
		await page.waitForTimeout(400);
		await shot(page, 'platform-tenants', width);
		await page
			.getByRole('navigation', { name: 'Основная навигация' })
			.getByRole('link', { name: 'Пользователи', exact: true })
			.click();
		await page.getByRole('heading', { level: 1 }).waitFor();
		await page.waitForTimeout(400);
		await shot(page, 'platform-users', width);

		expect(wide, 'screens with a horizontal scroll at 1440').toEqual([]);
	});
}
